import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import type { DebuggerManager as Recorder, NetRecord } from '../../src/background/debugger.ts';

// What the build defines (extension/build.mjs), before the modules that read it.
(globalThis as Record<string, unknown>)['__MC_VERSION__'] = '0.0.0-test';
const { DebuggerManager, NETWORK_LIMIT, netMatches, netSummary } =
  await import('../../src/background/debugger.ts');
const { Dialogs } = await import('../../src/background/dialogs.ts');

const TAB = 7;

function recorder(): Recorder {
  const dbg = new DebuggerManager(new Dialogs(() => undefined));
  dbg.startLog(TAB);
  return dbg;
}

function sent(dbg: Recorder, id: string, url: string, extra: Record<string, unknown> = {}): void {
  dbg.record(TAB, 'Network.requestWillBeSent', {
    requestId: id,
    request: { url, method: 'GET', headers: { Accept: '*/*' } },
    type: 'Fetch',
    timestamp: 100,
    wallTime: 1_700_000_000,
    ...extra,
  });
}

describe('the network log', () => {
  it('follows a request from start to end, headers from both events', () => {
    const dbg = recorder();
    sent(dbg, 'r1', 'https://app.example/api/items', {
      request: {
        url: 'https://app.example/api/items',
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        postData: '{"a":1}',
      },
    });
    dbg.record(TAB, 'Network.requestWillBeSentExtraInfo', {
      requestId: 'r1',
      headers: { Cookie: 'sid=1' },
    });
    dbg.record(TAB, 'Network.responseReceived', {
      requestId: 'r1',
      type: 'XHR',
      response: {
        status: 201,
        statusText: 'Created',
        mimeType: 'application/json',
        headers: { 'x-id': '9' },
        protocol: 'h2',
        remoteIPAddress: '10.0.0.1',
        remotePort: 443,
      },
    });
    dbg.record(TAB, 'Network.responseReceivedExtraInfo', {
      requestId: 'r1',
      headers: { 'set-cookie': 'sid=2' },
    });
    dbg.record(TAB, 'Network.loadingFinished', {
      requestId: 'r1',
      timestamp: 100.25,
      encodedDataLength: 321,
    });
    const r = dbg.request(TAB, 'r1') as NetRecord;
    assert.equal(r.method, 'POST');
    assert.equal(r.type, 'xhr');
    assert.equal(r.at, 1_700_000_000_000);
    assert.equal(r.status, 201);
    assert.equal(r.ms, 250);
    assert.equal(r.size, 321);
    assert.equal(r.postData, '{"a":1}');
    assert.equal(r.remoteAddress, '10.0.0.1:443');
    assert.deepEqual(r.requestHeaders, { 'Content-Type': 'application/json', Cookie: 'sid=1' });
    assert.deepEqual(r.responseHeaders, { 'x-id': '9', 'set-cookie': 'sid=2' });
    // A list shows no headers, no bodies.
    assert.deepEqual(netSummary(r), {
      id: 'r1',
      method: 'POST',
      url: 'https://app.example/api/items',
      type: 'xhr',
      at: 1_700_000_000_000,
      status: 201,
      mimeType: 'application/json',
      size: 321,
      ms: 250,
    });
  });

  it('keeps a redirect as one request at its new address, and a failure with why', () => {
    const dbg = recorder();
    sent(dbg, 'r2', 'http://app.example/old');
    sent(dbg, 'r2', 'https://app.example/new', { redirectResponse: { status: 301 } });
    dbg.record(TAB, 'Network.loadingFailed', {
      requestId: 'r2',
      timestamp: 101,
      errorText: 'net::ERR_CONNECTION_REFUSED',
    });
    const r = dbg.request(TAB, 'r2') as NetRecord;
    assert.equal(r.url, 'https://app.example/new');
    assert.equal(r.redirects, 1);
    assert.equal(r.failed, 'net::ERR_CONNECTION_REFUSED');
    assert.equal(dbg.networkLog(TAB)?.records.length, 1);
    dbg.record(TAB, 'Network.loadingFailed', { requestId: 'r3', canceled: true });
    assert.equal(dbg.request(TAB, 'r3'), null, 'an event for a request never seen');
  });

  it('keeps WebSocket messages, the last few, cut', () => {
    const dbg = recorder();
    dbg.record(TAB, 'Network.webSocketCreated', { requestId: 'w1', url: 'wss://app.example/ws' });
    dbg.record(TAB, 'Network.webSocketFrameSent', {
      requestId: 'w1',
      response: { opcode: 1, payloadData: 'hello' },
    });
    dbg.record(TAB, 'Network.webSocketFrameReceived', {
      requestId: 'w1',
      response: { opcode: 2, payloadData: 'AAEC' },
    });
    for (let i = 0; i < 60; i++) {
      dbg.record(TAB, 'Network.webSocketFrameReceived', {
        requestId: 'w1',
        response: { opcode: 1, payloadData: 'x'.repeat(3000) },
      });
    }
    const r = dbg.request(TAB, 'w1') as NetRecord;
    assert.equal(r.type, 'websocket');
    assert.equal(r.frames?.length, 50);
    assert.ok(r.frames?.every((f) => f.payload.length <= 2001));
    assert.equal(netSummary(r)['frames'], 50);
  });

  it('keeps the last requests only, and starts over when cleared', () => {
    const dbg = recorder();
    for (let i = 0; i < NETWORK_LIMIT + 20; i++) sent(dbg, `n${i}`, `https://app.example/${i}`);
    const log = dbg.networkLog(TAB);
    assert.equal(log?.records.length, NETWORK_LIMIT);
    assert.equal(log?.records[0]?.id, 'n20', 'the oldest went first');
    dbg.clearNetwork(TAB);
    assert.equal(dbg.networkLog(TAB)?.records.length, 0);
    assert.equal(dbg.networkLog(99), null, 'a tab never attached has no log');
  });

  it('filters by address, type, failure and time', () => {
    const base = { method: 'GET', at: 1000 } as const;
    const ok: NetRecord = {
      ...base,
      id: 'a',
      url: 'https://x.example/API/items',
      type: 'fetch',
      status: 200,
    };
    const bad: NetRecord = {
      ...base,
      id: 'b',
      url: 'https://x.example/x.js',
      type: 'script',
      status: 404,
    };
    const failed: NetRecord = {
      ...base,
      id: 'c',
      url: 'https://x.example/y',
      type: 'ping',
      failed: 'canceled',
    };
    assert.ok(netMatches(ok, { filter: '/api/' }), 'case aside');
    assert.ok(!netMatches(bad, { filter: '/api/' }));
    assert.ok(netMatches(bad, { type: 'script' }) && !netMatches(ok, { type: 'script' }));
    assert.ok(netMatches(failed, { type: 'other' }) && !netMatches(ok, { type: 'other' }));
    assert.deepEqual(
      [ok, bad, failed].filter((r) => netMatches(r, { failed: true })).map((r) => r.id),
      ['b', 'c'],
    );
    assert.ok(netMatches(ok, { since: 1000 }) && !netMatches(ok, { since: 1001 }));
  });
});
