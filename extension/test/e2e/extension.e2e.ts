// End to end: the e2e build of the extension in the installed Chrome (a
// throwaway profile), a mock desktop speaking docs/BROWSER-PROTOCOL.md, and
// local fixture pages. Measures snapshot sizes and command round trips into
// dist-extension/e2e-report.json.
//
//   npm run ext:e2e                    headless Chrome
//   MC_E2E_HEADFUL=1 npm run ext:e2e   watch it
//   MC_E2E_BROWSER=/path/to/browser    another Chromium (Brave, Edge, Chrome for Testing)

import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { after, before, describe, it } from 'node:test';
import { Browser, defaultBrowserPath } from './browser.ts';
import { Fixtures } from './fixtures.ts';
import { MockDesktop, type Frame } from './mock-desktop.ts';

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, '..', '..', '..');
const BUILD = join(repo, 'dist-extension', 'chrome-e2e');
const EXPECTED_ID = 'bagfjjgeehalpbpjjajhlghbehfmfclf';
const LEASE = 'l_e2e';
const executable = defaultBrowserPath();

const report: Record<string, unknown> = {};

type Result = Awaited<ReturnType<MockDesktop['command']>>;

function ok(r: Result, what: string): Record<string, unknown> {
  assert.equal(r.ok, true, `${what}: ${JSON.stringify(r.error)}`);
  return r.data;
}

function fails(r: Result, code: string, what: string): Result {
  assert.equal(
    r.ok,
    false,
    `${what} should fail with ${code}, got ${JSON.stringify(r.data).slice(0, 300)}`,
  );
  assert.equal(r.error?.code, code, `${what}: ${JSON.stringify(r.error)}`);
  return r;
}

/** The ref on the first snapshot line matching `pattern`. */
function refOf(snapshot: string, pattern: RegExp): string {
  const line = snapshot.split('\n').find((l) => pattern.test(l));
  assert.ok(line, `no line matching ${pattern} in:\n${snapshot}`);
  const m = /\[ref=([a-z0-9]+)\]/.exec(line);
  assert.ok(m?.[1], `no ref on: ${line}`);
  return m[1];
}

function percentile(values: number[], p: number): number {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length))] ?? 0;
}

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

describe(
  'Mother Claude extension in Chrome',
  { skip: executable ? false : 'no Chrome installed' },
  () => {
    let browser: Browser;
    let desktop: MockDesktop;
    const fixtures = new Fixtures();
    let extensionId = '';
    let worker = '';
    let tabId = 0;
    const downloads = mkdtempSync(join(tmpdir(), 'mc-ext-e2e-downloads-'));
    const started = Date.now();

    const cmd = (
      action: string,
      params: Record<string, unknown> = {},
      opts: { tabId?: number; timeoutMs?: number; lease?: string } = {},
    ): Promise<Result> => desktop.command(opts.lease ?? LEASE, action, params, opts);

    /** Evaluate in the service worker (chrome.* available). */
    async function sw<T = unknown>(expression: string): Promise<T> {
      return browser.evaluate<T>(worker, expression);
    }

    async function connectWorker(): Promise<void> {
      const target = await browser.waitForTarget(
        (t) =>
          t.type === 'service_worker' && t.url.startsWith(`chrome-extension://${extensionId}/`),
        15_000,
      );
      worker = await browser.attachWorker(target.targetId);
    }

    /** An extension page in its own tab, for UI requests and the side panel. */
    async function extensionPage(path: string): Promise<{ targetId: string; sessionId: string }> {
      const p = await browser.openPage(`chrome-extension://${extensionId}/${path}`);
      await sleep(300);
      return p;
    }

    async function ui(
      sessionId: string,
      req: Record<string, unknown>,
    ): Promise<Record<string, unknown>> {
      return browser.evaluate(
        sessionId,
        `chrome.runtime.sendMessage({ type: 'mc.ui', req: ${JSON.stringify(req)} })`,
      );
    }

    async function text(selector: string, opts: { tabId?: number } = {}): Promise<string> {
      const r = await cmd('read', { mode: 'text', selector }, opts);
      return String(ok(r, `read ${selector}`)['content']);
    }

    before(async () => {
      await fixtures.start();
      desktop = new MockDesktop(EXPECTED_ID);
      await desktop.start();
      desktop.state.siteModes = {
        [fixtures.base]: 'act',
        [fixtures.other]: 'act',
        [fixtures.v6]: 'act',
        '*': 'off',
      };
      browser = await Browser.launch(executable ?? '', {
        headless: process.env['MC_E2E_HEADFUL'] !== '1',
      });
      await browser.send('Browser.setDownloadBehavior', {
        behavior: 'allow',
        downloadPath: downloads,
      });
      extensionId = await browser.loadExtension(BUILD);
      await connectWorker();
      await sw(
        `chrome.storage.local.set({ config: { host: '127.0.0.1', port: ${desktop.port}, browserName: 'E2E Chrome' } })`,
      );
    });

    after(async () => {
      report['suiteSeconds'] = Math.round((Date.now() - started) / 1000);
      report['browser'] = await browser?.send('Browser.getVersion').catch(() => ({}));
      await browser?.close();
      await desktop?.stop();
      await fixtures.stop();
      rmSync(downloads, { recursive: true, force: true });
      const out = join(repo, 'dist-extension', 'e2e-report.json');
      mkdirSync(dirname(out), { recursive: true });
      writeFileSync(out, `${JSON.stringify(report, null, 2)}\n`);
      console.log(`\ne2e report → ${out}\n${JSON.stringify(report, null, 2)}`);
    });

    it('loads the unpacked build under the pinned id', () => {
      assert.equal(extensionId, EXPECTED_ID);
    });

    it('refuses a wrong code, then pairs and authenticates with hello', async () => {
      const options = await extensionPage('options.html');
      // The worker must be on the mock's port before anything connects.
      let port = 0;
      for (let i = 0; i < 50 && port !== desktop.port; i++) {
        const st = (await ui(options.sessionId, { op: 'status' }))['status'] as {
          connection: { port: number };
        };
        port = st.connection.port;
        if (port !== desktop.port) await sleep(100);
      }
      assert.equal(port, desktop.port);
      const wrong = await ui(options.sessionId, {
        op: 'pair',
        code: 'ZZZZ-ZZZZ',
        name: 'E2E Chrome',
      });
      assert.equal(wrong['ok'], false);
      assert.match(String(wrong['message']), /not right/);
      // Case, spaces and dashes don't matter.
      const right = await ui(options.sessionId, {
        op: 'pair',
        code: 'ab-cd efgh',
        name: 'E2E Chrome',
      });
      assert.equal(right['ok'], true, JSON.stringify(right));
      const pair = await desktop.waitFor((f) => f.type === 'pair' && f['code'] === 'ABCD-EFGH');
      assert.equal(pair['name'], 'E2E Chrome');
      assert.equal((pair['browser'] as { name: string }).name.length > 0, true);
      assert.equal(pair['extensionVersion'], '0.1.0');
      const hello = await desktop.waitFor((f) => f.type === 'hello');
      assert.equal(hello['token'], desktop.token);
      const caps = hello['capabilities'] as Record<string, boolean>;
      assert.equal(caps['debugger'], true);
      assert.equal(caps['tabGroups'], true);
      assert.equal(caps['sidePanel'], true);
      assert.ok(Array.isArray(hello['granted']));
      assert.ok(desktop.origins.every((o) => o === `chrome-extension://${EXPECTED_ID}`));
      const status = await ui(options.sessionId, { op: 'status' });
      const s = status['status'] as { connection: { state: string; appVersion: string } };
      assert.equal(s.connection.state, 'connected');
      assert.equal(s.connection.appVersion, '0.8.0-e2e');
      await browser.send('Target.closeTarget', { targetId: options.targetId });
    });

    it('answers a frame type it does not know with unknown_frame', async () => {
      desktop.raw({ type: 'teleport', id: 'q1' });
      const reply = await desktop.waitFor((f) => f.type === 'error' && f.id === 'q1');
      assert.equal(reply['code'], 'unknown_frame');
    });

    it('opens a lease as a titled, colored tab group', async () => {
      const opened = await desktop.request({
        type: 'lease.open',
        lease: LEASE,
        session: { id: 'sess-1', title: 'Frontend · my-app' },
        color: 'blue',
      });
      assert.equal(opened.type, 'lease.opened');
      assert.equal(opened['lease'], LEASE);
      const groupId = opened['groupId'] as number;
      assert.equal(typeof groupId, 'number');
      const group = await sw<{ title: string; color: string }>(`chrome.tabGroups.get(${groupId})`);
      assert.deepEqual([group.title, group.color], ['Frontend · my-app', 'blue']);
      const tabs = await sw<{ url: string }[]>(`chrome.tabs.query({ groupId: ${groupId} })`);
      assert.equal(tabs.length, 1);
      assert.match(tabs[0]?.url ?? '', /lease\.html#l_e2e$/);
      desktop.setState({
        leases: [
          {
            lease: LEASE,
            session: { id: 'sess-1', title: 'Frontend · my-app' },
            color: 'blue',
            groupId,
          },
        ],
      });
    });

    it('answers no_tab before the first page, no_lease and unsupported when asked nonsense', async () => {
      fails(await cmd('snapshot'), 'no_tab', 'snapshot on the placeholder');
      fails(await cmd('snapshot', {}, { lease: 'l_nope' }), 'no_lease', 'unknown lease');
      fails(await cmd('teleport'), 'unsupported', 'unknown action');
    });

    it('opens a page in the group, reusing the placeholder, and lists it', async () => {
      const since = Date.now();
      const opened = ok(await cmd('tabs.open', { url: `${fixtures.base}/form.html` }), 'tabs.open');
      tabId = opened['tabId'] as number;
      assert.equal(opened['title'], 'Sign up — Fixture');
      const list = ok(await cmd('tabs.list'), 'tabs.list')['tabs'] as {
        tabId: number;
        inLease: boolean;
        active: boolean;
        url: string;
      }[];
      assert.deepEqual(
        list.map((t) => [t.tabId, t.inLease, t.active]),
        [[tabId, true, true]],
      );
      const updated = await desktop.event(
        'tab.updated',
        (f) => f['tabId'] === tabId && f['status'] === 'complete',
        10_000,
        since,
      );
      assert.equal(updated['lease'], LEASE);
      assert.equal(updated['url'], `${fixtures.base}/form.html`);
    });

    it('takes a snapshot in the protocol’s format, secrets redacted', async () => {
      const r = ok(await cmd('snapshot'), 'snapshot');
      const snap = String(r['snapshot']);
      assert.equal(r['truncated'], false);
      assert.equal(r['url'], `${fixtures.base}/form.html`);
      for (const line of [
        '- banner',
        '  - link "Fixture Co" [ref=',
        '  - heading "Create your account" [level=1]',
        '    - textbox "Full name" [ref=',
        '    - textbox "Password" [ref=',
        '    - checkbox "I accept the terms" [ref=',
        '      - radio "Free" [ref=',
      ]) {
        assert.ok(snap.includes(line), `missing ${line} in\n${snap}`);
      }
      assert.match(snap, /textbox "Password" \[ref=e\d+\] value=«redacted»/);
      assert.ok(!snap.includes('hunter2'));
    });

    it('fills a form so a React-style field sees it, selects, checks, types and submits', async () => {
      const snap = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      const name = refOf(snap, /textbox "Full name"/);
      const email = refOf(snap, /textbox "Email"/);
      const nick = refOf(snap, /textbox "Nickname/);
      const country = refOf(snap, /combobox "Country"/);
      const terms = refOf(snap, /checkbox "I accept/);
      const pro = refOf(snap, /radio "Pro"/);
      const bio = refOf(snap, /textbox "About you"/);
      const submit = refOf(snap, /button "Create account"/);
      const filled = ok(
        await cmd('fill', {
          fields: [
            { ref: name, value: 'Ada Lovelace' },
            { ref: email, value: 'ada@example.com' },
            { ref: nick, value: 'ada' },
          ],
        }),
        'fill',
      );
      assert.equal(filled['filled'], 3);
      assert.equal(await text('#react-state'), 'ada');
      ok(await cmd('select', { ref: country, values: ['Germany'] }), 'select');
      ok(await cmd('check', { ref: terms, checked: true }), 'check');
      ok(await cmd('check', { ref: pro, checked: true }), 'check radio');
      ok(await cmd('type', { ref: bio, text: 'Hello\nworld', clear: true }), 'type');
      const after = String(
        ok(await cmd('snapshot', { selector: 'form' }), 'snapshot form')['snapshot'],
      );
      assert.match(after, /combobox "Country" \[ref=e\d+\] value="Germany"/);
      assert.match(after, /checkbox "I accept the terms" \[ref=e\d+\] \[checked\]/);
      assert.match(after, /textbox "About you" \[ref=e\d+\] \[focused\] value="Hello\\nworld"/);
      const clicked = ok(await cmd('click', { ref: submit }), 'click submit');
      assert.match(
        String(clicked['snapshot'] ?? ''),
        /yes: Ada Lovelace <ada@example.com> de pro terms=on/,
      );
      assert.equal(await text('#submitted'), 'yes: Ada Lovelace <ada@example.com> de pro terms=on');
    });

    it('never types into a password field unless the desktop says the user granted it', async () => {
      const snap = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      const pw = refOf(snap, /textbox "Password"/);
      const refused = fails(
        await cmd('type', { ref: pw, text: 'x' }),
        'unsupported',
        'type into a password',
      );
      assert.match(refused.error?.message ?? '', /secret field/);
      fails(
        await cmd('fill', { fields: [{ ref: pw, value: 'x' }] }),
        'unsupported',
        'fill a password',
      );
      // The desktop's own flag asks for its own code.
      fails(
        await cmd('type', { ref: pw, text: 'x', refuseSecret: true }),
        'secret_field',
        'refuseSecret',
      );
      fails(
        await cmd('fill', {
          fields: [{ ref: pw, value: 'x' }],
          refuseSecret: true,
          allowSecret: true,
        }),
        'secret_field',
        'refuseSecret wins over allowSecret',
      );
      ok(
        await cmd('type', { ref: pw, text: 'granted', clear: true, allowSecret: true }),
        'type with the grant',
      );
      const again = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      assert.ok(!again.includes('granted'));
    });

    it('presses keys: Tab moves the focus', async () => {
      const snap = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      ok(await cmd('click', { ref: refOf(snap, /textbox "Full name"/) }), 'focus name');
      ok(await cmd('press', { key: 'Tab' }), 'Tab');
      const after = String(
        ok(await cmd('snapshot', { interactiveOnly: true }), 'snapshot')['snapshot'],
      );
      assert.match(after, /textbox "Email" \[ref=e\d+\] \[focused\]/);
    });

    it('reports a ref from before a reload as stale', async () => {
      const snap = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      const submit = refOf(snap, /button "Create account"/);
      ok(await cmd('reload'), 'reload');
      const r = fails(await cmd('click', { ref: submit }), 'stale_ref', 'stale ref');
      assert.match(r.error?.message ?? '', /take a new snapshot/);
      const fresh = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      assert.notEqual(
        refOf(fresh, /button "Create account"/),
        submit,
        'refs are never reused in a tab',
      );
    });

    it('answers a click that navigates the whole page with the page it ended on', async () => {
      const snap = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      const r = ok(await cmd('click', { ref: refOf(snap, /link "Issues"/) }), 'click a link');
      assert.equal(r['url'], `${fixtures.base}/typical.html`);
      assert.equal(r['title'], 'Issues · mother-claude/example');
      const back = ok(await cmd('back'), 'back');
      assert.equal(back['url'], `${fixtures.base}/form.html`);
    });

    it('follows an SPA route change: tab.updated, wait for the URL, back', async () => {
      ok(await cmd('navigate', { url: `${fixtures.base}/spa.html` }), 'navigate spa');
      const snap = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      const since = Date.now();
      ok(await cmd('click', { ref: refOf(snap, /link "Settings"/) }), 'click route');
      const ev = await desktop.event(
        'tab.updated',
        (f) => String(f['url']).endsWith('/spa/settings'),
        10_000,
        since,
      );
      assert.equal(ev['tabId'], tabId);
      ok(await cmd('wait', { for: 'url', value: '*/spa/settings' }), 'wait url');
      const settings = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      ok(await cmd('click', { ref: refOf(settings, /button "Save settings"/) }), 'save');
      assert.equal(await text('#saved'), 'Saved');
      const back = ok(await cmd('back'), 'back');
      assert.equal(back['url'], `${fixtures.base}/spa.html`);
    });

    it('reads and acts inside open and closed shadow roots', async () => {
      ok(await cmd('navigate', { url: `${fixtures.base}/shadow.html` }), 'navigate shadow');
      const snap = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      const q = refOf(snap, /textbox "Search issues"/);
      ok(await cmd('type', { ref: q, text: 'is:open', submit: true }), 'type in shadow');
      assert.equal(await text('#out'), 'is:open');
      ok(
        await cmd('click', { ref: refOf(snap, /button "Closed shadow button"/) }),
        'click closed shadow',
      );
      assert.equal(await text('#out'), 'closed button clicked');
    });

    it('includes same-origin frames with their own refs and shows a cross-origin frame as one line', async () => {
      ok(await cmd('navigate', { url: `${fixtures.base}/frames.html` }), 'navigate frames');
      ok(await cmd('wait', { for: 'time', value: 400 }), 'let the frames load');
      const snap = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      assert.match(
        snap,
        /- iframe "Same-origin settings"\n\s+- button "Inner button" \[ref=f\d+e\d+\]/,
      );
      assert.match(snap, /- iframe "Captcha" \(cross-origin, not read\)/);
      ok(await cmd('click', { ref: refOf(snap, /button "Inner button"/) }), 'click in frame');
      const found = ok(await cmd('find', { text: 'inner clicked' }), 'find in frame')[
        'matches'
      ] as { role: string }[];
      assert.equal(found.length, 1);
      report['frames'] = { snapshot: snap };
    });

    it('uses trusted input for a page that ignores synthetic events', async () => {
      ok(await cmd('navigate', { url: `${fixtures.base}/trusted.html` }), 'navigate trusted');
      const snap = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      const go = refOf(snap, /button "Continue"/);
      ok(await cmd('click', { ref: go }), 'synthetic click');
      assert.equal(await text('#out'), 'ignored synthetic click');
      ok(await cmd('click', { ref: go, trusted: true }), 'trusted click');
      assert.equal(await text('#out'), 'trusted click');
      ok(
        await cmd('type', {
          ref: refOf(snap, /textbox "Trusted field"/),
          text: 'ok',
          trusted: true,
        }),
        'trusted type',
      );
      assert.equal(await text('#typed'), 'ok');
      ok(await cmd('press', { key: 'Backspace', trusted: true }), 'trusted press');
      const value = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      assert.match(value, /textbox "Trusted field" \[ref=e\d+\] \[focused\] value="o"/);
    });

    it('captures the console through the debugger', async () => {
      ok(await cmd('navigate', { url: `${fixtures.base}/misc.html` }), 'navigate misc');
      const since = Date.now() - 1;
      const snap = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      ok(await cmd('click', { ref: refOf(snap, /button "Log something"/) }), 'log');
      await sleep(100);
      const all = ok(await cmd('console', { since }), 'console')['messages'] as {
        level: string;
        text: string;
      }[];
      assert.ok(
        all.some((m) => m.level === 'warning' && m.text === 'warned from the page'),
        JSON.stringify(all),
      );
      const errors = ok(await cmd('console', { since, level: 'error' }), 'console errors')[
        'messages'
      ] as { level: string }[];
      assert.ok(
        errors.length >= 1 && errors.every((m) => m.level === 'error'),
        JSON.stringify(errors),
      );
    });

    it('evaluates an expression and reports what it throws', async () => {
      assert.equal(
        ok(await cmd('eval', { expression: 'document.title' }), 'eval')['value'],
        'Misc',
      );
      assert.deepEqual(
        ok(await cmd('eval', { expression: 'Promise.resolve({ a: [1, 2] })' }), 'eval promise')[
          'value'
        ],
        { a: [1, 2] },
      );
      const r = fails(await cmd('eval', { expression: 'nope.nope' }), 'internal', 'eval throws');
      assert.match(r.error?.message ?? '', /ReferenceError/);
    });

    it('takes viewport and element screenshots', async () => {
      const shot = ok(await cmd('screenshot'), 'screenshot');
      assert.equal(shot['mimeType'], 'image/png');
      assert.ok(
        (shot['width'] as number) > 100 && (shot['height'] as number) > 100,
        JSON.stringify({ w: shot['width'], h: shot['height'] }),
      );
      const snap = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      const el = ok(
        await cmd('screenshot', { scope: 'element', ref: refOf(snap, /button "Log something"/) }),
        'element screenshot',
      );
      assert.ok(
        (el['width'] as number) < (shot['width'] as number),
        'element smaller than the viewport',
      );
      assert.ok((el['width'] as number) > 10 && (el['height'] as number) > 5);
      report['screenshot'] = {
        viewport: [shot['width'], shot['height']],
        element: [el['width'], el['height']],
        base64Bytes: String(shot['data']).length,
      };
    });

    it('uploads bytes the desktop sent (never reads the disk)', async () => {
      const snap = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      const file = refOf(snap, /button "Attachment"/);
      ok(
        await cmd('upload', {
          ref: file,
          files: [
            {
              name: 'notes.txt',
              mimeType: 'text/plain',
              data: Buffer.from('hello upload').toString('base64'),
            },
          ],
        }),
        'upload',
      );
      await sleep(100);
      assert.equal(await text('#file-out'), 'notes.txt text/plain 12 hello upload');
    });

    it('opens a link meant for a new tab inside the lease, then closes it', async () => {
      const snap = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      const r = ok(
        await cmd('click', { ref: refOf(snap, /link "Open the form in a new tab"/) }),
        'click new-tab link',
      );
      const newTab = r['newTab'] as { tabId: number; url: string };
      assert.equal(newTab.url, `${fixtures.base}/form.html`);
      const tabs = ok(await cmd('tabs.list'), 'tabs.list')['tabs'] as { tabId: number }[];
      assert.ok(tabs.some((t) => t.tabId === newTab.tabId) && tabs.some((t) => t.tabId === tabId));
      const since = Date.now();
      ok(await cmd('tabs.close', { tabId: newTab.tabId }), 'tabs.close');
      const closed = await desktop.event(
        'tab.closed',
        (f) => f['tabId'] === newTab.tabId,
        10_000,
        since,
      );
      assert.equal(closed['lease'], LEASE);
      ok(await cmd('tabs.select', { tabId }), 'tabs.select');
    });

    it('reports a download a lease tab started (file name only)', async () => {
      const snap = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      const since = Date.now();
      ok(await cmd('click', { ref: refOf(snap, /link "Download the report"/) }), 'click download');
      const ev = await desktop.event('download.started', () => true, 10_000, since);
      assert.equal(ev['filename'], 'report.txt');
      assert.equal(ev['tabId'], tabId);
      assert.equal(ev['lease'], LEASE);
    });

    it('waits for a selector, a text, a time, and answers timeout 500 ms before the deadline', async () => {
      const snap = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      ok(await cmd('click', { ref: refOf(snap, /button "Add later"/) }), 'add later');
      const w = ok(await cmd('wait', { for: 'selector', value: '#late' }), 'wait selector');
      assert.ok((w['waitedMs'] as number) > 100, JSON.stringify(w));
      ok(await cmd('wait', { for: 'text', value: 'Late content' }), 'wait text');
      const t = ok(await cmd('wait', { for: 'time', value: 300 }), 'wait time');
      assert.ok((t['waitedMs'] as number) >= 290);
      const start = Date.now();
      const late = fails(
        await cmd('wait', { for: 'selector', value: '#never' }, { timeoutMs: 3000 }),
        'timeout',
        'wait past the deadline',
      );
      const elapsed = Date.now() - start;
      assert.ok(elapsed >= 2300 && elapsed < 3000, `answered after ${elapsed} ms`);
      report['deadline'] = {
        timeoutMs: 3000,
        answeredAfterMs: elapsed,
        message: late.error?.message,
      };
    });

    it('navigates and waits for a quiet network', async () => {
      const start = Date.now();
      const r = ok(
        await cmd('navigate', { url: `${fixtures.base}/network.html`, waitUntil: 'networkidle' }),
        'navigate networkidle',
      );
      const elapsed = Date.now() - start;
      // Two 600 ms requests in a row, then 500 ms of quiet: not sooner, and not
      // held up by a request an earlier page left open (the download test's).
      assert.ok(elapsed >= 1100 && elapsed < 4000, `returned after ${elapsed} ms`);
      report['networkIdleMs'] = elapsed;
      assert.equal(r['status'], 200);
      assert.equal(
        ok(await cmd('eval', { expression: 'document.title' }), 'title')['value'],
        'Network — done',
      );
    });

    it('reports dialogs at once and answers them with dialog', async () => {
      ok(await cmd('navigate', { url: `${fixtures.base}/dialogs.html` }), 'navigate dialogs');
      const snap = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      const since = Date.now();
      const clicked = ok(
        await cmd('click', { ref: refOf(snap, /button "Delete draft"/) }),
        'click confirm',
      );
      assert.deepEqual(clicked['dialog'], { kind: 'confirm', message: 'Delete the draft?' });
      const ev = await desktop.event('dialog.opened', () => true, 5000, since);
      assert.deepEqual(
        [ev['tabId'], ev['lease'], ev['kind'], ev['message']],
        [tabId, LEASE, 'confirm', 'Delete the draft?'],
      );
      fails(await cmd('snapshot'), 'dialog_open', 'snapshot while a dialog is open');
      ok(await cmd('dialog', { accept: true }), 'accept');
      assert.equal(await text('#out'), 'confirmed');
      ok(await cmd('click', { ref: refOf(snap, /button "Ask name"/) }), 'click prompt');
      ok(await cmd('dialog', { accept: true, text: 'Ada' }), 'answer prompt');
      assert.equal(await text('#out'), 'name: Ada');
      ok(await cmd('click', { ref: refOf(snap, /button "Show alert"/) }), 'click alert');
      ok(await cmd('dialog', { accept: true }), 'dismiss alert');
      assert.equal(await text('#out'), 'after alert');
      fails(await cmd('dialog', { accept: true }), 'not_found', 'no dialog open');
    });

    it('enforces site modes again: read refuses acting, off refuses everything', async () => {
      ok(await cmd('navigate', { url: `${fixtures.base}/form.html` }), 'navigate form');
      const snap = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      const name = refOf(snap, /textbox "Full name"/);
      desktop.setState({ siteModes: { ...desktop.state.siteModes, [fixtures.base]: 'read' } });
      await sleep(150);
      ok(await cmd('snapshot'), 'snapshot in read mode');
      ok(await cmd('scroll', { direction: 'down', amount: 200 }), 'scroll in read mode');
      fails(await cmd('type', { ref: name, text: 'x' }), 'site_read_only', 'type in read mode');
      desktop.setState({ siteModes: { ...desktop.state.siteModes, [fixtures.base]: 'off' } });
      await sleep(150);
      fails(await cmd('snapshot'), 'site_off', 'snapshot in off mode');
      fails(
        await cmd('navigate', { url: `${fixtures.base}/spa.html` }),
        'site_off',
        'navigate to an off site',
      );
      desktop.setState({ siteModes: { ...desktop.state.siteModes, [fixtures.base]: 'act' } });
      await sleep(150);
      ok(await cmd('snapshot'), 'snapshot back in act mode');
    });

    it('asks for a host permission it does not have (permission_needed)', async () => {
      ok(await cmd('navigate', { url: `${fixtures.v6}/form.html` }), 'navigate to [::1]');
      const r = fails(
        await cmd('snapshot'),
        'permission_needed',
        'snapshot without host permission',
      );
      assert.deepEqual(r.error?.data, { permission: 'http://[::1]/*' });
      assert.deepEqual(r.data, { permission: 'http://[::1]/*' });
      ok(await cmd('navigate', { url: `${fixtures.base}/form.html` }), 'back to a granted site');
    });

    it('forwards setSiteMode, and shows the desktop’s refusal for a sensitive site', async () => {
      const page = await extensionPage('options.html');
      const since = Date.now();
      const okr = await ui(page.sessionId, {
        op: 'setSiteMode',
        origin: `${fixtures.other}/anything`,
        mode: 'read',
      });
      assert.equal(okr['ok'], true, JSON.stringify(okr));
      const frame = await desktop.waitFor((f) => f.type === 'setSiteMode', 5000, since);
      assert.deepEqual([frame['origin'], frame['mode']], [fixtures.other, 'read']);
      desktop.sensitive.add(fixtures.other);
      const refused = await ui(page.sessionId, {
        op: 'setSiteMode',
        origin: fixtures.other,
        mode: 'act',
      });
      assert.equal(refused['ok'], false);
      assert.equal(refused['code'], 'needs_desktop');
      assert.match(String(refused['message']), /on the desktop/);
      desktop.sensitive.delete(fixtures.other);
      desktop.setState({ siteModes: { ...desktop.state.siteModes, [fixtures.other]: 'act' } });
      await browser.send('Target.closeTarget', { targetId: page.targetId });
    });

    it('answers an approval from the side panel; the first answer wins', async () => {
      desktop.setState({
        pending: [
          {
            id: 'p_1',
            lease: LEASE,
            session: 'Frontend · my-app',
            summary: 'Submit the form on 127.0.0.1',
            detail: 'POST /signup',
            dangerous: true,
            createdAt: Date.now(),
          },
        ],
      });
      const panel = await extensionPage('sidepanel.html');
      let card = false;
      for (let i = 0; i < 30 && !card; i++) {
        card = await browser.evaluate<boolean>(
          panel.sessionId,
          `!!document.querySelector('[data-key="allow-p_1"]')`,
        );
        if (!card) await sleep(100);
      }
      assert.ok(card, 'the approval card shows');
      const since = Date.now();
      await browser.evaluate(
        panel.sessionId,
        `document.querySelector('[data-key="allow-p_1"]').click()`,
      );
      const answer = await desktop.waitFor((f) => f.type === 'answer', 5000, since);
      assert.deepEqual([answer['pending'], answer['allow']], ['p_1', true]);
      const again = await ui(panel.sessionId, { op: 'answer', pending: 'p_1', allow: false });
      assert.equal(again['code'], 'already_answered');
      await browser.send('Target.closeTarget', { targetId: panel.targetId });
    });

    it('runs one command at a time per tab, while tabs overlap', async () => {
      const second = ok(
        await cmd('tabs.open', { url: `${fixtures.base}/spa.html`, active: false }),
        'second tab',
      )['tabId'] as number;
      const done: string[] = [];
      const track = (name: string, p: Promise<Result>): Promise<Result> =>
        p.then((r) => (done.push(name), r));
      await Promise.all([
        track('wait A', cmd('wait', { for: 'time', value: 600 }, { tabId })),
        track('snapshot A', cmd('snapshot', {}, { tabId })),
        track('snapshot B', cmd('snapshot', {}, { tabId: second })),
      ]);
      assert.deepEqual(done, ['snapshot B', 'wait A', 'snapshot A']);
      ok(await cmd('tabs.close', { tabId: second }), 'close second');
    });

    it('works on two sites in two tabs side by side', async () => {
      const a = ok(
        await cmd('tabs.open', { url: `${fixtures.base}/form.html`, active: false }),
        'site A',
      )['tabId'] as number;
      const b = ok(
        await cmd('tabs.open', { url: `${fixtures.other}/spa.html`, active: false }),
        'site B',
      )['tabId'] as number;
      const [ra, rb] = await Promise.all([
        cmd('read', { mode: 'text', selector: 'h1' }, { tabId: a }),
        cmd('read', { mode: 'text', selector: 'h1' }, { tabId: b }),
      ]);
      assert.notEqual(ok(ra, 'read A')['url'], ok(rb, 'read B')['url']);
      assert.match(String(ok(ra, 'read A')['url']), new RegExp(`^${fixtures.base}`));
      assert.match(String(ok(rb, 'read B')['url']), new RegExp(`^${fixtures.other}`));
      // Each tab keeps its own site's mode.
      desktop.setState({ siteModes: { ...desktop.state.siteModes, [fixtures.other]: 'off' } });
      await sleep(150);
      fails(await cmd('snapshot', {}, { tabId: b }), 'site_off', 'site B off');
      ok(await cmd('snapshot', {}, { tabId: a }), 'site A still fine');
      desktop.setState({ siteModes: { ...desktop.state.siteModes, [fixtures.other]: 'act' } });
      await sleep(150);
      ok(await cmd('tabs.close', { tabId: a }), 'close A');
      ok(await cmd('tabs.close', { tabId: b }), 'close B');
    });

    it('records the tab’s requests, with headers and bodies on request', async () => {
      // The log runs across page loads (like DevTools' "Preserve log"): an
      // earlier visit to this page is in it, so start from a clean one.
      ok(await cmd('network', { clear: true }), 'clear first');
      ok(
        await cmd('navigate', { url: `${fixtures.base}/network.html`, waitUntil: 'networkidle' }),
        'navigate network',
      );
      ok(
        await cmd('eval', {
          expression: `fetch('/api/items', { method: 'POST', headers: { 'content-type': 'application/json', authorization: 'Bearer abc' }, body: JSON.stringify({ name: 'pen' }) }).then(async (r) => (await r.text(), r.status))`,
        }),
        'post',
      );
      const list = ok(await cmd('network', { filter: '/api/' }), 'network list');
      const requests = list['requests'] as Record<string, unknown>[];
      const slow = requests.filter((r) => String(r['url']).includes('/api/slow'));
      assert.equal(slow.length, 2, JSON.stringify(requests));
      assert.ok(slow.every((r) => r['status'] === 200 && r['type'] === 'fetch'));
      assert.ok(slow.every((r) => typeof r['ms'] === 'number' && (r['ms'] as number) >= 500));
      assert.ok(!('requestHeaders' in (slow[0] ?? {})), 'a list has no headers');
      const post = requests.find((r) => r['method'] === 'POST');
      assert.ok(post, 'the POST is listed');
      assert.equal(post['status'], 201);
      // One in full: headers as sent (the desktop redacts them), the bodies.
      const one = ok(await cmd('network', { requestId: post['id'], body: true }), 'network detail');
      const req = one['request'] as Record<string, unknown>;
      const sent = req['requestHeaders'] as Record<string, string>;
      const authorization = Object.entries(sent).find(([k]) => k.toLowerCase() === 'authorization');
      assert.equal(authorization?.[1], 'Bearer abc');
      assert.equal(req['postData'], '{"name":"pen"}');
      const got = req['responseHeaders'] as Record<string, string>;
      assert.equal(got['x-request-id'], 'req-42');
      assert.deepEqual(one['body'], { text: '{"created":{"name":"pen"}}', size: 26 });
      // Filters, and forgetting.
      const failed = ok(await cmd('network', { failed: true }), 'failed only');
      assert.equal((failed['requests'] as unknown[]).length, 0);
      ok(await cmd('network', { clear: true }), 'clear');
      assert.equal((ok(await cmd('network'), 'after clear')['requests'] as unknown[]).length, 0);
      fails(await cmd('network', { requestId: 'nope' }), 'not_found', 'unknown request');
    });

    it('reads cookies, the HttpOnly one too, values only when asked, and changes them', async () => {
      ok(await cmd('navigate', { url: `${fixtures.base}/storage.html` }), 'navigate storage');
      ok(await cmd('wait', { for: 'text', value: 'ready' }), 'page ready');
      const plain = ok(await cmd('cookies'), 'cookies')['cookies'] as Record<string, unknown>[];
      const sid = plain.find((c) => c['name'] === 'sid');
      assert.ok(sid, JSON.stringify(plain));
      assert.equal(sid['httpOnly'], true);
      assert.ok(!('value' in sid), 'no value unless asked');
      const valued = ok(await cmd('cookies', { values: true, name: 'sid' }), 'sid value')[
        'cookies'
      ] as Record<string, unknown>[];
      assert.equal(valued[0]?.['value'], 's3ss10n');
      ok(await cmd('cookies.set', { name: 'added', value: 'yes', path: '/' }), 'set');
      const after = ok(await cmd('cookies', { values: true, name: 'added' }), 'added')[
        'cookies'
      ] as Record<string, unknown>[];
      assert.equal(after[0]?.['value'], 'yes');
      ok(await cmd('cookies.delete', { name: 'added' }), 'delete');
      fails(await cmd('cookies', { name: 'added' }), 'not_found', 'deleted');
      fails(
        await cmd('cookies.set', { name: 'x', value: 'y', domain: 'example.com' }),
        'unsupported',
        'another site’s cookie',
      );
    });

    it('reads and changes local and session storage, and reads IndexedDB', async () => {
      const local = ok(await cmd('storage', { area: 'local' }), 'local');
      const entries = local['entries'] as Record<string, unknown>[];
      assert.deepEqual(
        entries.map((e) => [e['key'], e['size'], 'value' in e]),
        [
          ['recent', 9, false],
          ['theme', 4, false],
        ],
      );
      const theme = ok(
        await cmd('storage', { area: 'local', key: 'theme', values: true }),
        'theme',
      );
      assert.equal((theme['entries'] as Record<string, unknown>[])[0]?.['value'], 'dark');
      const session = ok(await cmd('storage', { area: 'session', values: true }), 'session');
      assert.deepEqual(session['entries'], [{ key: 'draft', size: 5, value: 'hello' }]);
      ok(await cmd('storage.set', { area: 'local', key: 'theme', value: 'light' }), 'set');
      assert.equal(
        ok(await cmd('eval', { expression: `localStorage.getItem('theme')` }), 'page sees it')[
          'value'
        ],
        'light',
      );
      ok(await cmd('storage.remove', { area: 'local', key: 'recent' }), 'remove');
      ok(await cmd('storage.clear', { area: 'session' }), 'clear');
      assert.equal(ok(await cmd('storage', { area: 'session' }), 'session after')['count'], 0);
      fails(await cmd('storage', { area: 'local', key: 'gone' }), 'not_found', 'no such key');
      // IndexedDB: the databases, a database's stores, a store's records.
      const dbs = ok(await cmd('storage', { area: 'indexeddb' }), 'databases');
      assert.deepEqual(dbs['databases'], [{ name: 'appdb', version: 1 }]);
      const stores = ok(await cmd('storage', { area: 'indexeddb', db: 'appdb' }), 'stores');
      assert.deepEqual(
        (stores['stores'] as Record<string, unknown>[]).map((s) => [s['name'], s['count']]),
        [['items', 2]],
      );
      const records = ok(
        await cmd('storage', { area: 'indexeddb', db: 'appdb', store: 'items', values: true }),
        'records',
      );
      assert.deepEqual(
        (records['records'] as Record<string, unknown>[]).map((r) => [r['key'], r['value']]),
        [
          ['1', '{"id":1,"title":"First"}'],
          ['2', '{"id":2,"title":"Second","tags":["x"]}'],
        ],
      );
      fails(await cmd('storage', { area: 'indexeddb', db: 'nope' }), 'not_found', 'no such db');
      // Changing storage is acting: refused on a read-only site.
      desktop.setState({ siteModes: { ...desktop.state.siteModes, [fixtures.base]: 'read' } });
      await sleep(150);
      ok(await cmd('storage', { area: 'local' }), 'reading on a read-only site');
      fails(
        await cmd('storage.set', { area: 'local', key: 'k', value: 'v' }),
        'site_read_only',
        'changing on a read-only site',
      );
      fails(await cmd('cookies.delete', { name: 'sid' }), 'site_read_only', 'cookie on read');
      desktop.setState({ siteModes: { ...desktop.state.siteModes, [fixtures.base]: 'act' } });
      await sleep(150);
    });

    it('drags with HTML drag-and-drop events, and with the pointer', async () => {
      ok(await cmd('navigate', { url: `${fixtures.base}/drag.html` }), 'navigate drag');
      const snap = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      const card = refOf(snap, /button "Card"/);
      // A drop zone is not interactive, so the snapshot gives it no ref:
      // find does, by its text.
      const found = ok(await cmd('find', { text: 'Drop here' }), 'find the zone')['matches'] as {
        ref?: string;
      }[];
      const zone = found[0]?.ref;
      assert.ok(zone, JSON.stringify(found));
      const r = ok(await cmd('drag', { ref: card, toRef: zone }), 'drag card');
      assert.equal(r['html5'], true);
      assert.equal(r['dropped'], true);
      assert.equal(await text('#dropped'), 'dropped card-1');
      // A slider that follows the pointer: synthetic and trusted.
      const knob = refOf(snap, /slider "Knob"/);
      const end = refOf(snap, /button "Track end"/);
      ok(await cmd('drag', { ref: knob, toRef: end }), 'drag knob');
      assert.match(await text('#slid'), /^at (3[0-9]{2})$/);
      ok(await cmd('navigate', { url: `${fixtures.base}/drag.html` }), 'again');
      const again = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      ok(
        await cmd('drag', {
          ref: refOf(again, /slider "Knob"/),
          toRef: refOf(again, /button "Track end"/),
          trusted: true,
        }),
        'trusted drag',
      );
      assert.match(await text('#slid'), /^at (3[0-9]{2})$/);
    });

    it('measures snapshot sizes and round trips', async () => {
      const sizes: Record<string, unknown> = {};
      for (const [name, url] of [
        ['typical (issue list)', `${fixtures.base}/typical.html`],
        ['form', `${fixtures.base}/form.html`],
        ['long (3000 rows)', `${fixtures.base}/long.html`],
      ] as const) {
        ok(await cmd('navigate', { url }), `navigate ${name}`);
        const entry: Record<string, unknown> = {};
        for (const interactiveOnly of [true, false]) {
          const r = ok(await cmd('snapshot', { interactiveOnly }), `snapshot ${name}`);
          const bytes = Buffer.byteLength(String(r['snapshot']));
          entry[interactiveOnly ? 'interactiveOnly' : 'full'] = {
            bytes,
            approxTokens: Math.round(bytes / 4),
            lines: r['nodes'],
            truncated: r['truncated'],
          };
        }
        sizes[name] = entry;
      }
      report['snapshotSizes'] = sizes;
      const typical = (sizes['typical (issue list)'] as Record<string, { bytes: number }>)[
        'interactiveOnly'
      ];
      assert.ok(
        typical && typical.bytes <= 16_000,
        `typical interactiveOnly snapshot is ${typical?.bytes} bytes`,
      );
      const long = (
        sizes['long (3000 rows)'] as Record<string, { truncated: boolean; lines: number }>
      )['full'];
      assert.equal(long?.truncated, true);
      assert.equal(long?.lines, 1500);
      const capped = ok(await cmd('snapshot', { maxNodes: 200 }), 'maxNodes 200');
      assert.deepEqual([capped['truncated'], capped['nodes']], [true, 200]);

      ok(await cmd('navigate', { url: `${fixtures.base}/typical.html` }), 'navigate typical');
      const snap = String(
        ok(await cmd('snapshot', { interactiveOnly: true }), 'snapshot')['snapshot'],
      );
      const box = refOf(snap, /checkbox "Select issue #1200"/);
      const rtt: Record<string, { p50: number; p95: number; n: number }> = {};
      const measure = async (name: string, run: () => Promise<Result>, n = 30): Promise<void> => {
        const times: number[] = [];
        for (let i = 0; i < n; i++) {
          const r = await run();
          ok(r, name);
          times.push(r.ms);
        }
        rtt[name] = {
          p50: Math.round(percentile(times, 50) * 10) / 10,
          p95: Math.round(percentile(times, 95) * 10) / 10,
          n,
        };
      };
      await measure('tabs.list', () => cmd('tabs.list'));
      await measure('find', () => cmd('find', { role: 'link', name: 'Issues' }));
      await measure('snapshot (interactiveOnly, typical page)', () =>
        cmd('snapshot', { interactiveOnly: true }),
      );
      await measure('click (synthetic, with delta)', () => cmd('click', { ref: box }));
      await measure('read (text, selector)', () =>
        cmd('read', { mode: 'text', selector: 'h1, nav' }),
      );
      report['roundTripMs'] = rtt;
      report['pingMs'] = desktop.received.filter((f) => f.type === 'ping').length;
      const click = rtt['click (synthetic, with delta)'];
      assert.ok(click && click.p50 < 400, JSON.stringify(rtt));
    });

    it('Stop ends the in-flight command at once, refuses new ones, and Resume lifts it', async () => {
      ok(await cmd('navigate', { url: `${fixtures.base}/form.html` }), 'navigate');
      const panel = await extensionPage('sidepanel.html');
      let ready = false;
      for (let i = 0; i < 30 && !ready; i++) {
        ready = await browser.evaluate<boolean>(
          panel.sessionId,
          `!!document.querySelector('[data-key="lease-${LEASE}"]')`,
        );
        if (!ready) await sleep(100);
      }
      assert.ok(ready, 'the session shows in the side panel');
      const waiting = cmd('wait', { for: 'selector', value: '#never' }, { timeoutMs: 20_000 });
      await sleep(300);
      const pressed = Date.now();
      await browser.evaluate(
        panel.sessionId,
        `document.querySelector('[data-key="lease-${LEASE}"]').click()`,
      );
      const revoked = await desktop.event(
        'lease.revoked',
        (f) => f['lease'] === LEASE,
        5000,
        pressed - 1,
      );
      const revokedAfter = (revoked['at'] as number) - pressed;
      const result = await waiting;
      const stoppedAfter = Date.now() - pressed;
      fails(result, 'stopped', 'the in-flight wait');
      assert.equal(revoked['by'], 'user');
      assert.ok(
        revokedAfter < 1000 && stoppedAfter < 1000,
        `revoked after ${revokedAfter} ms, stopped after ${stoppedAfter} ms`,
      );
      fails(await cmd('snapshot'), 'stopped', 'a new command after Stop');
      report['stop'] = { leaseRevokedAfterMs: revokedAfter, inFlightStoppedAfterMs: stoppedAfter };
      // Resume takes Stop's place, disabled for a second so a double click cannot undo the Stop.
      const button = `document.querySelector('[data-key="lease-${LEASE}"]')`;
      const early = await browser.evaluate<{ text: string; disabled: boolean }>(
        panel.sessionId,
        `new Promise((r) => setTimeout(() => r({ text: ${button}.textContent, disabled: ${button}.disabled }), 150))`,
      );
      assert.deepEqual(early, { text: 'Resume', disabled: true });
      let enabled = false;
      for (let i = 0; i < 40 && !enabled; i++) {
        enabled = await browser.evaluate<boolean>(
          panel.sessionId,
          `${button}.textContent === 'Resume' && !${button}.disabled`,
        );
        if (!enabled) await sleep(100);
      }
      assert.ok(enabled, 'Resume becomes available');
      const resumeAt = Date.now();
      await browser.evaluate(panel.sessionId, `${button}.click()`);
      await desktop.event('lease.resumed', (f) => f['lease'] === LEASE, 5000, resumeAt - 1);
      // The stopped wait no longer holds the tab: the next command runs at once.
      const next = await cmd('snapshot');
      ok(next, 'snapshot after Resume');
      assert.ok(next.ms < 1000, `the next command took ${next.ms} ms`);
      (report['stop'] as Record<string, unknown>)['nextCommandAfterResumeMs'] = Math.round(next.ms);
      await browser.send('Target.closeTarget', { targetId: panel.targetId });
    });

    it('survives a service-worker restart: reconnects, keeps the lease, the tab and its refs', async () => {
      const snap = String(ok(await cmd('snapshot'), 'snapshot')['snapshot']);
      const name = refOf(snap, /textbox "Full name"/);
      const connections = desktop.connections;
      const target = await browser.waitForTarget(
        (t) =>
          t.type === 'service_worker' && t.url.startsWith(`chrome-extension://${extensionId}/`),
      );
      await browser.send('Target.closeTarget', { targetId: target.targetId });
      for (let i = 0; i < 50 && desktop.isConnected; i++) await sleep(100);
      assert.equal(desktop.isConnected, false, 'the worker stopped');
      const panel = await extensionPage('sidepanel.html'); // any extension event wakes the worker
      for (let i = 0; i < 100 && !desktop.isConnected; i++) await sleep(100);
      assert.ok(desktop.isConnected && desktop.connections > connections, 'reconnected');
      await connectWorker();
      ok(
        await cmd('type', { ref: name, text: 'after restart', clear: true }),
        'type with a ref from before the restart',
      );
      const list = ok(await cmd('tabs.list'), 'tabs.list')['tabs'] as { tabId: number }[];
      assert.deepEqual(
        list.map((t) => t.tabId),
        [tabId],
      );
      await browser.send('Target.closeTarget', { targetId: panel.targetId });
    });

    it('ungroups or closes a lease’s tabs on lease.close', async () => {
      for (const [lease, closeTabs] of [
        ['l_keep', false],
        ['l_close', true],
      ] as const) {
        ok(
          await (async () => {
            const opened = await desktop.request({
              type: 'lease.open',
              lease,
              session: { id: lease, title: lease },
              color: 'green',
            });
            assert.equal(opened.type, 'lease.opened');
            return cmd('tabs.open', { url: `${fixtures.base}/spa.html`, active: false }, { lease });
          })(),
          `open ${lease}`,
        );
        const tab = (
          ok(await cmd('tabs.list', {}, { lease }), 'list')['tabs'] as { tabId: number }[]
        )[0]?.tabId;
        const closed = await desktop.request({ type: 'lease.close', lease, closeTabs });
        assert.equal(closed.type, 'ok');
        const still = await sw<{ groupId: number } | null>(
          `chrome.tabs.get(${tab}).catch(() => null)`,
        );
        if (closeTabs) assert.equal(still, null);
        else assert.equal(still?.groupId, -1);
        fails(await cmd('tabs.list', {}, { lease }), 'no_lease', 'the closed lease');
        if (!closeTabs) await sw(`chrome.tabs.remove(${tab})`);
      }
    });

    it('revokes the lease when the user closes its group', async () => {
      const groupId = await sw<number>(`chrome.tabs.get(${tabId}).then((t) => t.groupId)`);
      const tabs = await sw<number[]>(
        `chrome.tabs.query({ groupId: ${groupId} }).then((ts) => ts.map((t) => t.id))`,
      );
      const since = Date.now();
      await sw(`chrome.tabs.remove(${JSON.stringify(tabs)})`);
      const revoked = await desktop.event(
        'lease.revoked',
        (f) => f['lease'] === LEASE,
        5000,
        since,
      );
      assert.equal(revoked['by'], 'user');
      await desktop.event('tab.closed', (f) => f['tabId'] === tabId, 5000, since);
    });

    it('keeps the connection alive with a ping every 20 s', async () => {
      // The socket opened again at the worker restart: its first ping is due 20 s later.
      const ping = await desktop.waitFor((f: Frame) => f.type === 'ping', 25_000);
      assert.equal(typeof ping.id, 'string');
      const next = await desktop
        .waitFor(
          (f: Frame) =>
            f.type === 'ping' && f.id !== ping.id && (f['at'] as number) > (ping['at'] as number),
          25_000,
        )
        .catch(() => null);
      if (next) {
        const gap = (next['at'] as number) - (ping['at'] as number);
        assert.ok(gap >= 19_000 && gap <= 21_500, `pings ${gap} ms apart`);
        report['pingIntervalMs'] = gap;
      }
    });

    it('connects a running session to the tab you are on, and tells it what to do', async () => {
      const sid = '0f1e2d3c-4b5a-4968-8776-655443322110';
      const mute = '1f1e2d3c-4b5a-4968-8776-655443322111';
      desktop.sessions = [
        {
          id: sid,
          name: 'Polish the docs site',
          project: 'codertapsu.github.io',
          cwd: '/Users/me/dev/codertapsu.github.io',
          surface: 'VS Code',
          status: 'idle',
          lastActivityAt: Date.now() - 60_000,
          owned: false,
          canTell: true,
          tellHow: 'relay',
          whyNot: null,
          hasTools: true,
          lease: null,
        },
        {
          id: mute,
          name: 'A terminal session',
          project: 'other',
          cwd: '/Users/me/dev/other',
          surface: 'Terminal',
          status: 'working',
          lastActivityAt: Date.now(),
          owned: false,
          canTell: false,
          tellHow: null,
          whyNot: 'Mother Claude can’t send it messages.',
          hasTools: false,
          lease: null,
        },
        // Not a session id: dropped by the extension.
        { id: 'nope', name: 'x' },
      ];
      // A tab of the user's own, in no session's group.
      const tabId = await sw<number>(
        `chrome.tabs.create({ url: ${JSON.stringify(`${fixtures.base}/form.html`)}, active: true }).then((t) => t.id)`,
      );
      for (let i = 0; i < 50; i++) {
        const done = await sw<string>(`chrome.tabs.get(${tabId}).then((t) => t.status)`);
        if (done === 'complete') break;
        await sleep(100);
      }
      const panel = await extensionPage('sidepanel.html');
      const list = await ui(panel.sessionId, { op: 'sessions' });
      assert.equal(list['ok'], true, JSON.stringify(list));
      const listed = list['sessions'] as { id: string; canTell: boolean; hasTools: boolean }[];
      assert.deepEqual(
        listed.map((x) => x.id),
        [sid, mute],
      );
      assert.equal(listed[1]?.hasTools, false);
      // Connect and send, in one click.
      const res = await ui(panel.sessionId, {
        op: 'connect',
        session: sid,
        tabId,
        message: 'Read this page and list the form fields.',
      });
      assert.equal(res['ok'], true, JSON.stringify(res));
      assert.equal(res['told'], true);
      assert.equal(res['delivery'], 'relayed');
      const connect = await desktop.waitFor((f: Frame) => f.type === 'session.connect');
      assert.equal(connect['session'], sid);
      assert.equal((connect['tab'] as { tabId: number }).tabId, tabId);
      const told = desktop.told.at(-1);
      assert.equal(told?.['message'], 'Read this page and list the form fields.');
      assert.match(String((told?.['tab'] as { url: string }).url), /form\.html$/);
      // The tab is the session's: in its group, alone (no placeholder left).
      const lease = String(res['lease']);
      const groupId = await sw<number>(`chrome.tabs.get(${tabId}).then((t) => t.groupId)`);
      assert.ok(groupId > 0, 'the tab joined a group');
      const inGroup = await sw<string[]>(
        `chrome.tabs.query({ groupId: ${groupId} }).then((ts) => ts.map((t) => t.url))`,
      );
      assert.deepEqual(inGroup, [`${fixtures.base}/form.html`]);
      // Its commands act there.
      const r = await cmd('read', { mode: 'text', selector: 'h1' }, { lease });
      assert.match(String(ok(r, 'read the adopted tab')['content']), /Create your account/);
      // Telling it again, from its card.
      const again = await ui(panel.sessionId, { op: 'tell', lease, message: 'Now submit it.' });
      assert.equal(again['ok'], true, JSON.stringify(again));
      assert.equal(desktop.told.at(-1)?.['message'], 'Now submit it.');
      // A session the desktop cannot reach: connected, but the message is refused.
      const other = await sw<number>(
        `chrome.tabs.create({ url: ${JSON.stringify(`${fixtures.base}/typical.html`)}, active: true }).then((t) => t.id)`,
      );
      const muted = await ui(panel.sessionId, {
        op: 'connect',
        session: mute,
        tabId: other,
        message: 'hello',
      });
      assert.equal(muted['ok'], true, JSON.stringify(muted));
      assert.equal(muted['told'], false);
      assert.match(String(muted['tellError']), /cannot reach/);
      // A tab already working for one session is not taken by another.
      const taken = await ui(panel.sessionId, { op: 'connect', session: mute, tabId });
      assert.equal(taken['ok'], false);
      assert.match(String(taken['message']), /another session/);
      for (const l of [lease, String(muted['lease'])]) {
        await desktop.request({ type: 'lease.close', lease: l, closeTabs: true });
      }
      await browser.send('Target.closeTarget', { targetId: panel.targetId });
    });

    it('forgets its token when the desktop revokes the browser', async () => {
      desktop.raw({ type: 'error', code: 'revoked', message: 'removed on the desktop' });
      const page = await extensionPage('options.html');
      let state = '';
      for (let i = 0; i < 30 && state !== 'revoked'; i++) {
        const s = (await ui(page.sessionId, { op: 'status' }))['status'] as {
          connection: { state: string };
          paired: boolean;
        };
        state = s.connection.state;
        if (state !== 'revoked') await sleep(100);
      }
      assert.equal(state, 'revoked');
      const auth = await sw<unknown>(
        `chrome.storage.local.get('auth').then((r) => r.auth ?? null)`,
      );
      assert.equal(auth, null);
    });
  },
);
