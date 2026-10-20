// A fake bridge host for the offline account tests (selected with
// MOTHER_CLAUDE_BRIDGE_HOST_PATH). It speaks the host's line protocol just
// enough: `ready` first, then a `reply` to every request. Each request is
// appended to $FAKE_BRIDGE_LOG with the account part of its environment — the
// variable names, the config directory and only the SHA-256 of a token.
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
      return reply({ sessionId: c.sessionId, cwd: c.options && c.options.cwd });
    case 'store.list':
      return reply({ sessions: [] });
    case 'session.input': {
      reply({ turnId: c.turnId });
      // A turn: "AUTHFAIL" in the text answers like an expired token, anything
      // else succeeds.
      const text = JSON.stringify(c.content || '');
      const event = (message) =>
        send({ t: 'event', sessionId: c.sessionId, turnId: c.turnId, message });
      if (text.includes('AUTHFAIL')) {
        event({ type: 'assistant', error: 'authentication_failed', message: { content: [] } });
        event({ type: 'result', subtype: 'error_during_execution', is_error: true });
      } else {
        event({ type: 'result', subtype: 'success', is_error: false, result: 'ok' });
      }
      send({ t: 'turn', sessionId: c.sessionId, turnId: c.turnId, status: 'completed' });
      return;
    }
    default:
      return reply({ ok: true });
  }
});
