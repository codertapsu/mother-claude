// The extension against the real desktop: Mother Claude's own server
// (`/ws/browser`, `/api/browser/*`, `/mcp`) as the UI fixture serves it
// (src-tauri/tests/ui_fixture.rs) — not the mock, and never the app you run.
// Opt-in, since it needs that fixture running on a spare port:
//
//   MC_UI_PORT=6799 MC_UI_SECONDS=900 MC_UI_OUT=/tmp/mc-ui.json \
//     cargo test --manifest-path src-tauri/Cargo.toml --test ui_fixture -- --ignored --nocapture
//   MC_E2E_DESKTOP=/tmp/mc-ui.json npm run ext:e2e
//
// The tool calls come from this process over `/mcp`, which the fixture takes
// for a Claude Code session by the pidfile written into its synthetic home:
// the path a real session's calls take, without spending tokens.

import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { after, before, describe, it } from 'node:test';
import { Browser, defaultBrowserPath } from './browser.ts';
import { Fixtures } from './fixtures.ts';

const here = dirname(fileURLToPath(import.meta.url));
const BUILD = join(here, '..', '..', '..', 'dist-extension', 'chrome-e2e');
const executable = defaultBrowserPath();
const infoPath = process.env['MC_E2E_DESKTOP'];
const skip = !infoPath
  ? "set MC_E2E_DESKTOP to a running UI fixture's info file"
  : !executable
    ? 'no Chrome installed'
    : false;

interface Info {
  base: string;
  desktopToken: string;
  dataDir: string;
  home: string;
  /** The fixture's live VS Code session. */
  session: string;
}

type Json = Record<string, unknown>;

interface ToolResult {
  text: string;
  isError: boolean;
}

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

/** The ref on the first snapshot line matching `pattern`. */
function refOf(snapshot: string, pattern: RegExp): string {
  const line = snapshot.split('\n').find((l) => pattern.test(l));
  assert.ok(line, `no line matching ${pattern} in:\n${snapshot}`);
  const m = /\[ref=([a-z0-9]+)\]/.exec(line);
  assert.ok(m?.[1], `no ref on: ${line}`);
  return m[1];
}

describe('the extension against the real desktop', { skip }, () => {
  let info: Info;
  let browser: Browser | undefined;
  let worker = '';
  let extensionId = '';
  const fixtures = new Fixtures();
  const session = randomUUID();
  const project = mkdtempSync(join(tmpdir(), 'mc-ext-desktop-'));
  let pidfile = '';
  let mcpToken = '';
  let rpc = 0;

  function chrome(): Browser {
    assert.ok(browser, 'Chrome is running');
    return browser;
  }

  async function api(
    method: string,
    path: string,
    body?: unknown,
  ): Promise<{ status: number; json: Json }> {
    const r = await fetch(`${info.base}/api${path}`, {
      method,
      headers: {
        authorization: `Bearer ${info.desktopToken}`,
        'content-type': 'application/json',
      },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
    const text = await r.text();
    return { status: r.status, json: (text ? JSON.parse(text) : {}) as Json };
  }

  async function mcp(method: string, params: Json = {}): Promise<Json> {
    const r = await fetch(`${info.base}/mcp`, {
      method: 'POST',
      headers: {
        authorization: `Bearer ${mcpToken}`,
        'content-type': 'application/json',
        accept: 'application/json, text/event-stream',
        'mcp-protocol-version': '2025-11-25',
        'x-mc-pid': String(process.pid),
      },
      body: JSON.stringify({ jsonrpc: '2.0', id: ++rpc, method, params }),
    });
    const text = await r.text();
    assert.equal(r.status, 200, `${method}: HTTP ${r.status} ${text}`);
    return JSON.parse(text) as Json;
  }

  async function tool(name: string, args: Json = {}): Promise<ToolResult> {
    const v = await mcp('tools/call', { name, arguments: args });
    const result = v['result'] as
      { content?: { type: string; text?: string }[]; isError?: boolean } | undefined;
    assert.ok(result, `${name}: ${JSON.stringify(v['error'])}`);
    const text = (result.content ?? [])
      .filter((c) => c.type === 'text')
      .map((c) => c.text ?? '')
      .join('\n');
    return { text, isError: result.isError === true };
  }

  async function okTool(name: string, args: Json = {}): Promise<string> {
    const r = await tool(name, args);
    assert.equal(r.isError, false, `${name}: ${r.text}`);
    return r.text;
  }

  /** Answer this session's next browser approval, as the desktop would. */
  async function answerNext(decision: 'allow' | 'deny'): Promise<Json> {
    for (let i = 0; i < 150; i++) {
      const o = await api('GET', '/browser');
      const approvals = (o.json['approvals'] ?? []) as Json[];
      const mine = approvals.find((a) => a['session'] === session);
      if (mine) {
        const r = await api('POST', `/sessions/${session}/respond`, {
          requestId: mine['id'],
          decision,
        });
        assert.ok(r.status < 300, `respond: ${r.status} ${JSON.stringify(r.json)}`);
        return mine;
      }
      await sleep(100);
    }
    assert.fail('no approval showed up');
  }

  async function ui(pageSession: string, req: Json): Promise<Json> {
    return chrome().evaluate<Json>(
      pageSession,
      `chrome.runtime.sendMessage({ type: 'mc.ui', req: ${JSON.stringify(req)} })`,
    );
  }

  before(async () => {
    info = JSON.parse(readFileSync(infoPath ?? '', 'utf8')) as Info;
    mcpToken = readFileSync(join(info.dataDir, 'mcp.token'), 'utf8').trim();
    // This process is the calling session, as a pidfile makes any process.
    mkdirSync(join(info.home, 'sessions'), { recursive: true });
    pidfile = join(info.home, 'sessions', `${process.pid}.json`);
    writeFileSync(
      pidfile,
      JSON.stringify({
        pid: process.pid,
        sessionId: session,
        cwd: project,
        kind: 'interactive',
        entrypoint: 'cli',
        status: 'busy',
      }),
    );
    await fixtures.start();
    browser = await Browser.launch(executable ?? '', {
      headless: process.env['MC_E2E_HEADFUL'] !== '1',
    });
    extensionId = await browser.loadExtension(BUILD);
    const target = await browser.waitForTarget(
      (t) => t.type === 'service_worker' && t.url.startsWith(`chrome-extension://${extensionId}/`),
      15_000,
    );
    worker = await browser.attachWorker(target.targetId);
    const port = Number(new URL(info.base).port);
    await browser.evaluate(
      worker,
      `chrome.storage.local.set({ config: { host: '127.0.0.1', port: ${port}, browserName: 'E2E Chrome' } })`,
    );
  });

  after(async () => {
    if (pidfile) rmSync(pidfile, { force: true });
    await browser?.close();
    await fixtures.stop();
    rmSync(project, { recursive: true, force: true });
  });

  it('pairs with a code the desktop made, and the desktop sees it connected', async () => {
    const code = await api('POST', '/browser/pair');
    assert.equal(code.status, 201, JSON.stringify(code.json));
    const options = await chrome().openPage(`chrome-extension://${extensionId}/options.html`);
    // The page's scripts first: `chrome.runtime` appears once it has loaded.
    for (let i = 0; i < 50; i++) {
      const ready = await chrome().evaluate<boolean>(
        options.sessionId,
        `typeof chrome !== 'undefined' && !!chrome.runtime && typeof chrome.runtime.sendMessage === 'function'`,
      );
      if (ready) break;
      await sleep(100);
    }
    const port = Number(new URL(info.base).port);
    let on = 0;
    for (let i = 0; i < 50 && on !== port; i++) {
      const st = (await ui(options.sessionId, { op: 'status' }))['status'] as {
        connection: { port: number };
      };
      on = st.connection.port;
      if (on !== port) await sleep(100);
    }
    assert.equal(on, port, "the worker is on the fixture's port");
    const paired = await ui(options.sessionId, {
      op: 'pair',
      code: code.json['display'],
      name: 'E2E Chrome',
    });
    assert.equal(paired['ok'], true, JSON.stringify(paired));
    let connected = false;
    for (let i = 0; i < 100 && !connected; i++) {
      const o = await api('GET', '/browser');
      connected = ((o.json['browsers'] ?? []) as Json[]).some((b) => b['connected'] === true);
      if (!connected) await sleep(100);
    }
    assert.ok(connected, 'the desktop lists the browser as connected');
    await chrome().send('Target.closeTarget', { targetId: options.targetId });
  });

  it('offers the browser tools to the calling session', async () => {
    const init = await mcp('initialize', {
      protocolVersion: '2025-11-25',
      capabilities: {},
      clientInfo: { name: 'desktop-e2e', version: '1' },
    });
    assert.ok(init['result'], JSON.stringify(init));
    const list = await mcp('tools/list');
    const names = ((list['result'] as Json)['tools'] as Json[]).map((t) => t['name']);
    for (const n of ['browser_navigate', 'browser_snapshot', 'browser_click', 'browser_type']) {
      assert.ok(names.includes(n), `${n} in ${names.join(', ')}`);
    }
    assert.match(await okTool('mc_session'), new RegExp(session));
  });

  it('refuses a site that is off', async () => {
    const r = await tool('browser_navigate', { url: `${fixtures.base}/typical.html` });
    assert.equal(r.isError, true, r.text);
    assert.doesNotMatch(r.text, /no_browser/, 'refused for the site, not for want of a browser');
  });

  it('reads and acts on a site set to act', async () => {
    const put = await api('PUT', '/browser/sites', { origin: fixtures.base, mode: 'act' });
    assert.ok(put.status < 300, JSON.stringify(put.json));
    await okTool('browser_navigate', { url: `${fixtures.base}/typical.html` });
    const snap = await okTool('browser_snapshot', { interactiveOnly: true });
    assert.match(snap, /\[ref=/);
    const read = await okTool('browser_read', { mode: 'text', maxChars: 2000 });
    assert.match(read, /Issues/);
    // Another origin stays off.
    const other = await tool('browser_navigate', { url: `${fixtures.other}/typical.html` });
    assert.equal(other.isError, true, other.text);
  });

  it('waits for approval on a page with a password, and never types the password', async () => {
    await okTool('browser_navigate', { url: `${fixtures.base}/form.html` });
    const snap = await okTool('browser_snapshot', {});
    const typing = tool('browser_type', { ref: refOf(snap, /textbox "Full name"/), text: 'Ada' });
    const asked = await answerNext('allow');
    assert.ok(String(asked['summary']).length > 0);
    const typed = await typing;
    assert.equal(typed.isError, false, typed.text);
    const secret = await tool('browser_type', {
      ref: refOf(snap, /"Password"/),
      text: 'not-a-real-password',
    });
    assert.equal(secret.isError, true, secret.text);
    const clicking = tool('browser_click', { ref: refOf(snap, /button "Create account"/) });
    await answerNext('allow');
    const clicked = await clicking;
    assert.equal(clicked.isError, false, clicked.text);
    const submitted = await okTool('browser_read', { mode: 'text', selector: '#submitted' });
    assert.match(submitted, /yes: Ada/);
  });

  it('connects a running session to the tab you are on, from the side panel', async () => {
    const panel = await chrome().openPage(`chrome-extension://${extensionId}/sidepanel.html`);
    for (let i = 0; i < 50; i++) {
      const ready = await chrome().evaluate<boolean>(
        panel.sessionId,
        `typeof chrome !== 'undefined' && !!chrome.runtime && typeof chrome.runtime.sendMessage === 'function'`,
      );
      if (ready) break;
      await sleep(100);
    }
    const list = await ui(panel.sessionId, { op: 'sessions' });
    assert.equal(list['ok'], true, JSON.stringify(list));
    const ids = (list['sessions'] as { id: string }[]).map((x) => x.id);
    assert.ok(ids.includes(info.session), `the fixture's session is listed: ${ids.join(', ')}`);
    const tabId = await chrome().evaluate<number>(
      worker,
      `chrome.tabs.create({ url: ${JSON.stringify(`${fixtures.base}/typical.html`)}, active: true }).then((t) => t.id)`,
    );
    for (let i = 0; i < 50; i++) {
      const done = await chrome().evaluate<string>(
        worker,
        `chrome.tabs.get(${tabId}).then((t) => t.status)`,
      );
      if (done === 'complete') break;
      await sleep(100);
    }
    const res = await ui(panel.sessionId, {
      op: 'connect',
      session: info.session,
      tabId,
      message: 'List the open issues on this page.',
    });
    assert.equal(res['ok'], true, JSON.stringify(res));
    // The desktop holds the session's lease in this browser.
    const o = await api('GET', '/browser');
    const lease = ((o.json['leases'] ?? []) as Json[]).find((l) => l['session'] === info.session);
    assert.ok(lease, 'the session has a lease here');
    assert.equal(lease['id'], res['lease']);
    // Told through the hooks, or refused in words while the fixture's session
    // has sent no hook traffic yet.
    if (res['told'] !== true) {
      assert.match(String(res['tellError']), /can't send it messages|cannot reach/i);
    }
    await chrome().send('Target.closeTarget', { targetId: panel.targetId });
  });

  it('fails a call that is denied, and Stop ends the session’s browsing', async () => {
    const evaluating = tool('browser_eval', { expression: '1 + 1' });
    await answerNext('deny');
    const denied = await evaluating;
    assert.equal(denied.isError, true, denied.text);
    const stop = await api('POST', `/browser/sessions/${session}/stop`);
    assert.ok(stop.status < 300, JSON.stringify(stop.json));
    const stopped = await tool('browser_snapshot', {});
    assert.equal(stopped.isError, true, stopped.text);
    assert.match(stopped.text, /stop/i);
  });
});
