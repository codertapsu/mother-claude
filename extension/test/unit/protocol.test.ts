import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { backoffDelay } from '../../src/shared/backoff.ts';
import { modifierMask, parseKey } from '../../src/shared/keys.ts';
import {
  DEFAULT_TIMEOUT_MS,
  MAX_FRAME_BYTES,
  decodeIncoming,
  encodeFrame,
  errorResult,
  utf8Length,
} from '../../src/shared/protocol.ts';
import {
  KeyedQueue,
  DeadlineError,
  sleep,
  withDeadline,
  workBudgetMs,
} from '../../src/shared/queue.ts';
import { hostPattern, isSensitive, modeFor, originOf, siteRefusal } from '../../src/shared/site.ts';

describe('decoding frames', () => {
  it('decodes a command with its defaults', () => {
    const d = decodeIncoming(
      JSON.stringify({
        type: 'command',
        id: 'c_17',
        lease: 'l_3',
        action: 'click',
        params: { ref: 'e14' },
        extra: 1,
      }),
    );
    assert.equal(d.ok, true);
    assert.deepEqual(d.ok && d.frame, {
      type: 'command',
      id: 'c_17',
      lease: 'l_3',
      action: 'click',
      params: { ref: 'e14' },
      timeoutMs: DEFAULT_TIMEOUT_MS,
    });
  });

  it('keeps tabId and clamps timeoutMs', () => {
    const d = decodeIncoming(
      JSON.stringify({
        type: 'command',
        id: 'c',
        lease: 'l',
        action: 'snapshot',
        tabId: 412,
        timeoutMs: 99_999_999,
      }),
    );
    assert.ok(d.ok && d.frame.type === 'command');
    assert.equal(d.frame.tabId, 412);
    assert.equal(d.frame.timeoutMs, 600_000);
  });

  it('answers an unknown frame type with unknown_frame, echoing the id', () => {
    const d = decodeIncoming(JSON.stringify({ type: 'teleport', id: 'x9' }));
    assert.equal(d.ok, false);
    assert.deepEqual(!d.ok && d.reply, {
      type: 'error',
      code: 'unknown_frame',
      message: 'unknown frame type "teleport"',
      id: 'x9',
    });
  });

  it('refuses frames that are not JSON objects or lack required fields', () => {
    for (const text of [
      'nope',
      '[]',
      '{"id":"a"}',
      '{"type":"command","id":"c"}',
      '{"type":"command","id":"c","lease":"l","action":"x","tabId":"7"}',
    ]) {
      const d = decodeIncoming(text);
      assert.equal(d.ok, false, text);
      assert.equal(!d.ok && d.reply?.code, 'bad_frame', text);
    }
  });

  it('decodes state tolerantly: bad modes dropped, leases and pending kept', () => {
    const d = decodeIncoming(
      JSON.stringify({
        type: 'state',
        siteModes: { 'https://github.com': 'act', '*': 'off', 'https://x.test': 'everything' },
        settings: { screenshots: true, defaultMode: 'read', future: 1 },
        leases: [
          { lease: 'l_1', session: { id: 's', title: 'Frontend' }, color: 'blue', groupId: 7 },
          { nope: 1 },
        ],
        pending: [{ id: 'p1', summary: 'Submit', dangerous: true, createdAt: 5 }],
      }),
    );
    assert.ok(d.ok && d.frame.type === 'state');
    assert.deepEqual(d.frame.siteModes, { 'https://github.com': 'act', '*': 'off' });
    assert.deepEqual(d.frame.settings, { screenshots: true, defaultMode: 'read', future: 1 });
    assert.deepEqual(d.frame.leases, [
      { lease: 'l_1', session: { id: 's', title: 'Frontend' }, color: 'blue', groupId: 7 },
    ]);
    assert.deepEqual(d.frame.pending, [
      { id: 'p1', summary: 'Submit', dangerous: true, createdAt: 5 },
    ]);
  });

  it('decodes the pairing and authentication replies', () => {
    const paired = decodeIncoming(
      '{"type":"paired","id":"x1","token":"mcx_a","browserId":"brw_1"}',
    );
    assert.deepEqual(paired.ok && paired.frame, {
      type: 'paired',
      id: 'x1',
      token: 'mcx_a',
      browserId: 'brw_1',
    });
    const welcome = decodeIncoming(
      '{"type":"welcome","id":"x2","browserId":"brw_1","protocol":1,"appVersion":"0.8.0","siteModes":{"*":"read"}}',
    );
    assert.ok(welcome.ok && welcome.frame.type === 'welcome');
    assert.equal(welcome.frame.appVersion, '0.8.0');
    const err = decodeIncoming('{"type":"error","id":"x3","code":"revoked","message":"removed"}');
    assert.deepEqual(err.ok && err.frame, {
      type: 'error',
      code: 'revoked',
      id: 'x3',
      message: 'removed',
    });
  });
});

describe('encoding frames', () => {
  it('counts UTF-8 bytes', () => {
    assert.equal(utf8Length('abc'), 3);
    assert.equal(utf8Length('é'), 2);
    assert.equal(utf8Length('€'), 3);
    assert.equal(utf8Length('😀'), 4);
  });

  it('refuses a frame over 8 MiB', () => {
    assert.equal(encodeFrame({ data: 'x'.repeat(1000) }).ok, true);
    const big = encodeFrame({ data: 'x'.repeat(MAX_FRAME_BYTES) });
    assert.equal(big.ok, false);
  });

  it('names a needed permission in data as well as in error.data', () => {
    const r = errorResult('c1', {
      code: 'permission_needed',
      message: 'm',
      data: { permission: 'debugger' },
    });
    assert.deepEqual(r.data, { permission: 'debugger' });
    assert.deepEqual(r.error?.data, { permission: 'debugger' });
    assert.equal(errorResult('c2', { code: 'timeout', message: 'm' }).data, undefined);
  });
});

describe('site modes', () => {
  const modes = {
    'https://github.com': 'act',
    'https://docs.example.com:8443': 'read',
    '*': 'off',
  } as const;

  it('keys modes by origin with "*" as the default', () => {
    assert.equal(originOf('https://github.com/a/b?c'), 'https://github.com');
    assert.equal(originOf('https://github.com:443/'), 'https://github.com');
    assert.equal(originOf('chrome://settings'), null);
    assert.equal(modeFor('https://github.com/issues', { ...modes }), 'act');
    assert.equal(modeFor('https://docs.example.com:8443/x', { ...modes }), 'read');
    assert.equal(modeFor('https://docs.example.com/x', { ...modes }), 'off');
    assert.equal(modeFor('https://unknown.test/', {}, { defaultMode: 'read' }), 'read');
    assert.equal(modeFor('https://unknown.test/', {}), 'off');
    assert.equal(modeFor('chrome://extensions', { '*': 'act' }), 'off');
  });

  it('resolves wildcards (subdomains only, the longest wins) after the exact origin', () => {
    const wild = {
      'https://*.example.com': 'read',
      'https://*.docs.example.com': 'act',
      'https://*.example.com:8443': 'act',
      'https://exact.example.com': 'off',
      '*': 'act',
    } as const;
    assert.equal(modeFor('https://a.example.com/x', { ...wild }), 'read');
    assert.equal(modeFor('https://a.docs.example.com/', { ...wild }), 'act');
    assert.equal(modeFor('https://exact.example.com/', { ...wild }), 'off');
    assert.equal(
      modeFor('https://example.com/', { ...wild }),
      'act',
      'the domain itself is not covered',
    );
    assert.equal(modeFor('https://a.example.com:8443/', { ...wild }), 'act');
    assert.equal(
      modeFor('http://a.example.com/', { ...wild }),
      'act',
      'another scheme is not covered',
    );
  });

  it('keeps a sensitive site off unless it has an entry of its own', () => {
    const settings = { sensitiveDomains: ['paypal.com'] };
    assert.equal(
      modeFor('https://www.paypal.com/', { '*': 'act', 'https://*.paypal.com': 'act' }, settings),
      'off',
    );
    assert.equal(
      modeFor('https://www.paypal.com/', { 'https://www.paypal.com': 'read' }, settings),
      'read',
    );
    assert.equal(isSensitive('https://paypal.com', settings), true);
    assert.equal(isSensitive('https://notpaypal.com', settings), false);
    assert.equal(isSensitive('https://paypal.com', {}), false);
  });

  it('refuses by what an action needs', () => {
    assert.equal(siteRefusal('snapshot', 'off'), 'site_off');
    assert.equal(siteRefusal('snapshot', 'read'), null);
    assert.equal(siteRefusal('click', 'read'), 'site_read_only');
    assert.equal(siteRefusal('type', 'act'), null);
    assert.equal(siteRefusal('scroll', 'read'), null);
    assert.equal(siteRefusal('eval', 'read'), 'site_read_only');
    assert.equal(siteRefusal('tabs.list', 'off'), null);
    assert.equal(siteRefusal('dialog', 'read', { accept: false }), null);
    assert.equal(siteRefusal('dialog', 'read', { accept: true }, 'alert'), null);
    assert.equal(siteRefusal('dialog', 'read', { accept: true }, 'confirm'), 'site_read_only');
  });

  it('turns a URL into the host permission pattern Chrome grants', () => {
    assert.equal(hostPattern('https://github.com:8443/x'), 'https://github.com/*');
    assert.equal(hostPattern('http://[::1]:9000/'), 'http://[::1]/*');
    assert.equal(hostPattern('about:blank'), null);
  });
});

describe('queues and deadlines', () => {
  it('runs one task at a time per key, in order, and keys concurrently', async () => {
    const q = new KeyedQueue();
    const log: string[] = [];
    const task = (name: string, ms: number) => async () => {
      log.push(`${name}+`);
      await sleep(ms);
      log.push(`${name}-`);
      return name;
    };
    const results = await Promise.all([
      q.run('tab:1', task('a', 30)),
      q.run('tab:1', task('b', 5)),
      q.run('tab:2', task('c', 5)),
    ]);
    assert.deepEqual(results, ['a', 'b', 'c']);
    assert.ok(log.indexOf('a-') < log.indexOf('b+'), log.join(' '));
    assert.ok(log.indexOf('c-') < log.indexOf('a-'), log.join(' '));
    assert.equal(q.busyKeys, 0);
  });

  it('frees the key at once when a running task is aborted (Stop)', async () => {
    const q = new KeyedQueue();
    const c = new AbortController();
    const started = Date.now();
    const slow = q.run('tab:1', () => sleep(2000), c.signal);
    const next = q.run('tab:1', () => Promise.resolve('next'));
    setTimeout(() => c.abort(new Error('stopped')), 20);
    await assert.rejects(slow, /stopped/);
    assert.equal(await next, 'next');
    assert.ok(Date.now() - started < 500, `next ran after ${Date.now() - started} ms`);
  });

  it('keeps running after a task fails', async () => {
    const q = new KeyedQueue();
    await assert.rejects(q.run('k', () => Promise.reject(new Error('boom'))));
    assert.equal(await q.run('k', () => Promise.resolve(2)), 2);
  });

  it('answers 500 ms before the desktop’s deadline', () => {
    assert.equal(workBudgetMs(30_000), 29_500);
    assert.equal(workBudgetMs(1000), 750);
  });

  it('rejects at the deadline and aborts the work', async () => {
    const c = new AbortController();
    const start = Date.now();
    await assert.rejects(
      withDeadline(Date.now() + 50, c, () => sleep(1000, c.signal)),
      DeadlineError,
    );
    assert.ok(Date.now() - start < 500);
    assert.equal(c.signal.aborted, true);
  });

  it('backs off from 1 s to 30 s with jitter', () => {
    assert.equal(
      backoffDelay(0, () => 0.5),
      1000,
    );
    assert.equal(
      backoffDelay(1, () => 0.5),
      2000,
    );
    assert.equal(
      backoffDelay(4, () => 0.5),
      16_000,
    );
    assert.equal(
      backoffDelay(20, () => 0.5),
      30_000,
    );
    for (let i = 0; i < 12; i++) {
      for (const r of [0, 0.999]) {
        const d = backoffDelay(i, () => r);
        assert.ok(d >= 1000 && d <= 30_000, `${i} ${r} → ${d}`);
      }
    }
    assert.notEqual(
      backoffDelay(3, () => 0),
      backoffDelay(3, () => 0.99),
    );
  });
});

describe('keys', () => {
  it('parses names, aliases and combinations', () => {
    assert.equal(parseKey('Enter')?.def.keyCode, 13);
    assert.equal(parseKey('esc')?.def.key, 'Escape');
    assert.deepEqual(parseKey('Shift+Tab')?.modifiers, ['Shift']);
    assert.equal(parseKey('a', ['Shift'])?.def.key, 'A');
    assert.deepEqual(parseKey('ControlOrMeta+a', [], true)?.modifiers, ['Meta']);
    assert.deepEqual(parseKey('ControlOrMeta+a', [], false)?.modifiers, ['Control']);
    assert.equal(parseKey('+')?.def.key, '+');
    assert.equal(parseKey('NotAKey'), null);
    assert.equal(modifierMask(['Alt', 'Shift']), 9);
  });
});
