// A fake bridge host for the offline account tests (selected with
// MOTHER_CLAUDE_BRIDGE_HOST_PATH). It speaks the host's line protocol just
// enough: `ready` first, then a `reply` to every request. Each request is
// appended to $FAKE_BRIDGE_LOG with the account part of its environment — the
// variable names, the config directory and only the SHA-256 of a token.
//
// Its session store is the conversations created through it, each in the
// config directory it was created with (`store.list` with a `configDir`
// lists those, without one the rest). A turn whose text says "ASK" raises a
// tool request first and finishes once it is answered (`request.respond`).
'use strict';
const fs = require('node:fs');
const crypto = require('node:crypto');
const readline = require('node:readline');

const LOG = process.env.FAKE_BRIDGE_LOG;
const send = (frame) => process.stdout.write(JSON.stringify(frame) + '\n');
const record = (entry) => {
  if (LOG) fs.appendFileSync(LOG, JSON.stringify(entry) + '\n');
};
const sha = (s) => crypto.createHash('sha256').update(s).digest('hex');

// sessionId -> { configDir, lastModified }
const store = new Map();
// requestId -> { sessionId, turnId }
const asked = new Map();

const finish = (sessionId, turnId) => {
  send({
    t: 'event',
    sessionId,
    turnId,
    message: { type: 'result', subtype: 'success', is_error: false, result: 'ok' },
  });
  send({ t: 'turn', sessionId, turnId, status: 'completed' });
};

send({ t: 'ready', pid: process.pid });
readline.createInterface({ input: process.stdin }).on('line', (line) => {
  let c;
  try {
    c = JSON.parse(line);
  } catch {
    return;
  }
  const env = c.options && c.options.env;
  const set = (env && env.set) || {};
  record({
    op: c.op,
    sessionId: c.sessionId || null,
    configDir: c.configDir || null,
    resume: (c.options && c.options.resume) || null,
    forkSession: Boolean(c.options && c.options.forkSession),
    model: (c.options && c.options.model) || null,
    env: env
      ? {
          set: Object.keys(set).sort(),
          unset: env.unset || [],
          configDir: set.CLAUDE_CONFIG_DIR || null,
          tokenSha256: set.CLAUDE_CODE_OAUTH_TOKEN ? sha(set.CLAUDE_CODE_OAUTH_TOKEN) : null,
        }
      : null,
  });
  const reply = (result) => send({ t: 'reply', id: c.id, ok: true, result });
  switch (c.op) {
    case 'session.create':
      store.set(c.sessionId, {
        configDir: set.CLAUDE_CONFIG_DIR || null,
        lastModified: Date.now() + store.size,
      });
      return reply({ sessionId: c.sessionId, cwd: c.options && c.options.cwd });
    case 'store.list': {
      const dir = c.configDir || null;
      const sessions = [...store.entries()]
        .filter(([, s]) => s.configDir === dir)
        .map(([sessionId, s]) => ({ sessionId, summary: 'fake', lastModified: s.lastModified }));
      return reply({ sessions });
    }
    case 'session.input': {
      reply({ turnId: c.turnId });
      // A turn: "AUTHFAIL" in the text answers like an expired token, "ASK"
      // waits on a tool approval, anything else succeeds.
      const text = JSON.stringify(c.content || '');
      const event = (message) =>
        send({ t: 'event', sessionId: c.sessionId, turnId: c.turnId, message });
      if (text.includes('AUTHFAIL')) {
        event({ type: 'assistant', error: 'authentication_failed', message: { content: [] } });
        event({ type: 'result', subtype: 'error_during_execution', is_error: true });
        send({ t: 'turn', sessionId: c.sessionId, turnId: c.turnId, status: 'completed' });
      } else if (text.includes('ASK')) {
        const requestId = crypto.randomUUID();
        asked.set(requestId, { sessionId: c.sessionId, turnId: c.turnId });
        send({
          t: 'request',
          requestId,
          sessionId: c.sessionId,
          turnId: c.turnId,
          kind: 'permission',
          tool: 'Bash',
          prompt: 'Run ls',
          input: { command: 'ls' },
        });
      } else {
        finish(c.sessionId, c.turnId);
      }
      return;
    }
    case 'request.respond': {
      reply({ ok: true });
      const waiting = asked.get(c.requestId);
      if (waiting) {
        asked.delete(c.requestId);
        finish(waiting.sessionId, waiting.turnId);
      }
      return;
    }
    default:
      return reply({ ok: true });
  }
});
