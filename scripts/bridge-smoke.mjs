#!/usr/bin/env node
/**
 * Live smoke test for the Claude HTTP bridge, driven through the *shipped*
 * JavaScript client — so a change that breaks `client.mjs` fails here rather
 * than in someone's browser.
 *
 * It makes real model calls (a few cents) and needs `claude` signed in.
 *
 *   node scripts/bridge-smoke.mjs <base-url> [token]
 *
 * With the desktop app running:
 *   node scripts/bridge-smoke.mjs http://127.0.0.1:5612 "$MOTHER_CLAUDE_TOKEN"
 *
 * `cargo test --test bridge -- --ignored` runs this against a server it boots
 * itself, which is the usual way in.
 *
 * Exits non-zero on the first failure and prints what it actually saw.
 */
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import os from 'node:os';
import fs from 'node:fs';

const here = path.dirname(fileURLToPath(import.meta.url));
const client = await import(
  path.join(here, '..', 'src-tauri', 'assets', 'bridge', 'client.mjs')
);
const {
  BridgeApiError,
  chat,
  closeThread,
  createThread,
  getCapabilities,
  getHealth,
  listModels,
  listRequests,
  listThreads,
  readThread,
  runTurnStreaming,
  startTurn,
  streamOperation,
  withImages,
} = client;

const baseUrl = process.argv[2] || 'http://127.0.0.1:5612';
const token = process.argv[3] ?? process.env.MOTHER_CLAUDE_TOKEN ?? '';
const conn = { baseUrl, token };

// Somewhere harmless for the agent to consider its working directory.
const cwd = fs.mkdtempSync(path.join(os.tmpdir(), 'mc-bridge-smoke-'));
const opts = { cwd, model: 'haiku', setting_sources: [] };

let failures = 0;
let created = null;

function ok(name, detail = '') {
  console.log(`  ✓ ${name}${detail ? ` — ${detail}` : ''}`);
}

function fail(name, detail) {
  failures += 1;
  console.error(`  ✗ ${name} — ${detail}`);
}

async function step(name, fn) {
  const started = Date.now();
  try {
    const detail = await fn();
    ok(name, `${detail ?? ''}${detail ? ' · ' : ''}${Date.now() - started}ms`);
  } catch (err) {
    const detail =
      err instanceof BridgeApiError
        ? `${err.status} ${err.code}: ${err.message}`
        : (err && err.stack) || String(err);
    fail(name, detail);
  }
}

function expect(condition, message) {
  if (!condition) throw new Error(message);
}

console.log(`Claude bridge smoke test → ${baseUrl}`);
console.log(`  working directory: ${cwd}\n`);

await step('health', async () => {
  const health = await getHealth(conn);
  expect(health.status === 'ready', `status is ${health.status}`);
  expect(health.host_built === true, 'the Node host is not built (npm run sidecar:build)');
  return `v${health.version}, messages API ${health.messages_api ? 'on' : 'off'}`;
});

await step('capabilities', async () => {
  const caps = await getCapabilities(conn);
  expect(caps.limits.max_active_operations > 0, 'no operation limit reported');
  expect(Array.isArray(caps.backends.agent_sdk.features), 'no agent SDK feature list');
  return `${caps.limits.max_active_operations} concurrent operations`;
});

await step('models', async () => {
  const models = await listModels(conn);
  expect(Array.isArray(models.data), 'models.data is not an array');
  return `${models.data.length} models (${models.source})`;
});

await step('create conversation', async () => {
  created = await createThread(conn, opts);
  expect(typeof created.thread_id === 'string', 'no thread_id returned');
  return created.thread_id;
});

await step('start a turn and stream it', async () => {
  const seen = [];
  const terminal = await runTurnStreaming(
    conn,
    created.thread_id,
    'Reply with exactly: SMOKEOK',
    { onEvent: (event) => seen.push(event.method) },
  );
  expect(seen.includes('turn/started'), `never saw turn/started (saw ${seen.join(', ')})`);
  expect(terminal.status === 'completed', `turn status ${terminal.status}`);
  const text = terminal.result?.result ?? '';
  expect(text.includes('SMOKEOK'), `unexpected answer: ${JSON.stringify(text)}`);
  return `${seen.length} events, $${(terminal.result?.total_cost_usd ?? 0).toFixed(4)}`;
});

await step('replay the same turn from its log', async () => {
  const op = await startTurn(conn, created.thread_id, 'Reply with exactly: AGAIN');
  let terminal = null;
  for await (const { data } of streamOperation(conn, op.operation_id)) {
    if (data.method === 'bridge/completed' || data.method === 'bridge/error') terminal = data;
  }
  expect(terminal?.method === 'bridge/completed', 'turn did not complete');

  // Replay is non-destructive: reading again after completion yields the same
  // events, which is what makes reconnecting mid-turn safe.
  const replayed = [];
  for await (const { id, data } of streamOperation(conn, op.operation_id)) {
    replayed.push([id, data.method]);
  }
  expect(replayed.length > 1, 'replay returned nothing');
  expect(replayed.at(-1)[1] === 'bridge/completed', 'replay lost the terminal frame');

  // A cursor skips what it has already seen.
  const resumed = [];
  for await (const { id } of streamOperation(conn, op.operation_id, { after: 1 })) {
    resumed.push(id);
  }
  expect(!resumed.includes('1'), 'cursor did not skip the first event');
  return `${replayed.length} events replayed`;
});

await step('context recall across turns', async () => {
  const reply = await chat(conn, {
    message: 'What single word did I ask you to reply with two messages ago? One word.',
    threadId: created.thread_id,
  });
  expect(
    (reply.response ?? '').toUpperCase().includes('SMOKEOK') ||
      (reply.response ?? '').toUpperCase().includes('AGAIN'),
    `no recall: ${JSON.stringify(reply.response)}`,
  );
  return JSON.stringify(reply.response).slice(0, 40);
});

await step('vision: an inline image', async () => {
  const image = {
    type: 'image',
    source: { type: 'base64', media_type: 'image/png', data: solidBluePng(200, 200) },
  };
  const reply = await chat(conn, {
    input: withImages('What single colour is this image? One word.', [image]),
    ...opts,
  });
  expect(
    (reply.response ?? '').toLowerCase().includes('blue'),
    `unexpected answer: ${JSON.stringify(reply.response)}`,
  );
  await closeThread(conn, reply.thread_id).catch(() => {});
  return JSON.stringify(reply.response).slice(0, 30);
});

await step('validation is refused before the model runs', async () => {
  const tiny = { type: 'image', source: { type: 'base64', media_type: 'image/png', data: solidBluePng(2, 2) } };
  try {
    await chat(conn, { input: withImages('what is this', [tiny]), ...opts });
    throw new Error('a 2x2 image was accepted');
  } catch (err) {
    expect(err instanceof BridgeApiError, `wrong error type: ${err}`);
    expect(err.code === 'image_dimensions', `wrong code: ${err.code}`);
    return `${err.status} ${err.code}`;
  }
});

await step('list and read', async () => {
  const listed = await listThreads(conn, { live_only: 'true' });
  expect(Array.isArray(listed.data), 'threads.data is not an array');
  const read = await readThread(conn, created.thread_id);
  expect(read.thread_id === created.thread_id, 'read returned the wrong conversation');
  expect(Array.isArray(read.operations), 'no operations listed');
  return `${listed.data.length} live, ${read.operations.length} operations`;
});

await step('no requests left waiting', async () => {
  const pending = await listRequests(conn);
  expect(Array.isArray(pending.data), 'requests.data is not an array');
  return `${pending.data.length} pending`;
});

await step('close the conversation', async () => {
  const closed = await closeThread(conn, created.thread_id);
  expect(closed.status === 'closed', `status ${closed.status}`);
  return closed.thread_id;
});

fs.rmSync(cwd, { recursive: true, force: true });

console.log(failures ? `\n${failures} check(s) failed.` : '\nAll checks passed.');
process.exit(failures ? 1 : 0);

/** A minimal, valid single-colour PNG, base64 — no image dependency. */
function solidBluePng(width, height) {
  const crcTable = Array.from({ length: 256 }, (_, i) => {
    let c = i;
    for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    return c >>> 0;
  });
  const crc32 = (buf) => {
    let crc = 0xffffffff;
    for (const byte of buf) crc = crcTable[(crc ^ byte) & 0xff] ^ (crc >>> 8);
    return (crc ^ 0xffffffff) >>> 0;
  };
  const adler32 = (buf) => {
    let a = 1;
    let b = 0;
    for (const byte of buf) {
      a = (a + byte) % 65521;
      b = (b + a) % 65521;
    }
    return ((b << 16) | a) >>> 0;
  };
  const chunk = (kind, payload) => {
    const body = Buffer.concat([Buffer.from(kind, 'ascii'), payload]);
    const length = Buffer.alloc(4);
    length.writeUInt32BE(payload.length);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(body));
    return Buffer.concat([length, body, crc]);
  };

  const scanline = Buffer.concat([
    Buffer.from([0]),
    Buffer.concat(Array.from({ length: width }, () => Buffer.from([0x20, 0x4e, 0xd8]))),
  ]);
  const raw = Buffer.concat(Array.from({ length: height }, () => scanline));

  // zlib with stored deflate blocks: valid, and no compression needed.
  const blocks = [Buffer.from([0x78, 0x01])];
  for (let offset = 0; offset < raw.length; offset += 65535) {
    const block = raw.subarray(offset, offset + 65535);
    const header = Buffer.alloc(5);
    header.writeUInt8(offset + 65535 >= raw.length ? 1 : 0, 0);
    header.writeUInt16LE(block.length, 1);
    header.writeUInt16LE(~block.length & 0xffff, 3);
    blocks.push(header, block);
  }
  const adler = Buffer.alloc(4);
  adler.writeUInt32BE(adler32(raw));
  blocks.push(adler);

  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr.set([8, 2, 0, 0, 0], 8);

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', Buffer.concat(blocks)),
    chunk('IEND', Buffer.alloc(0)),
  ]).toString('base64');
}
