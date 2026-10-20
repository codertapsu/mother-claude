// The bridge host's environment composition and store lock
// (src/host-env.ts), against the built module: `npm run build` first, or
// `npm test`, which builds.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { conversationEnv, StoreLock, SDK_UNSET_WITHOUT_ENV } from '../dist/host-env.js';

test('a conversation starts as it would without an env', () => {
  const base = {
    PATH: '/bin',
    CLAUDE_CODE_QUESTION_EXTENDED: '1',
    CLAUDE_CODE_QUESTION_OPTIONAL_DESCRIPTIONS: '1',
    claude_code_agent_progress_summaries: '1',
    CLAUDE_CODE_OAUTH_TOKEN: 'inherited',
    ANTHROPIC_API_KEY: 'inherited',
  };
  const plain = conversationEnv(base);
  assert.equal(plain.PATH, '/bin');
  for (const name of SDK_UNSET_WITHOUT_ENV) {
    assert.ok(!Object.keys(plain).some((k) => k.toUpperCase() === name), name);
  }
  // The Default inherits the sign-in variables as it always did.
  assert.equal(plain.CLAUDE_CODE_OAUTH_TOKEN, 'inherited');
  assert.equal(plain.ANTHROPIC_API_KEY, 'inherited');
  assert.equal(base.CLAUDE_CODE_QUESTION_EXTENDED, '1', 'the base is never changed');

  // A token account: its token, the overrides removed.
  const token = conversationEnv(base, {
    set: { CLAUDE_CODE_OAUTH_TOKEN: 'mine' },
    unset: ['ANTHROPIC_API_KEY'],
  });
  assert.equal(token.CLAUDE_CODE_OAUTH_TOKEN, 'mine');
  assert.ok(!('ANTHROPIC_API_KEY' in token));
  assert.ok(!('CLAUDE_CODE_QUESTION_EXTENDED' in token));

  // What an account sets itself stays.
  const kept = conversationEnv(base, { set: { CLAUDE_CODE_QUESTION_EXTENDED: '1' } });
  assert.equal(kept.CLAUDE_CODE_QUESTION_EXTENDED, '1');
});

test("the Default's store calls share, another directory's run alone", async () => {
  const lock = new StoreLock();
  const log = [];
  let release;
  const gate = new Promise((r) => (release = r));
  const a = lock.run(false, async () => {
    log.push('a start');
    await gate;
    log.push('a end');
  });
  const b = lock.run(false, async () => {
    log.push('b');
  });
  await b;
  assert.deepEqual(log, ['a start', 'b'], 'b ran while a was still running');

  const c = lock.run(true, async () => {
    log.push('c');
  });
  const d = lock.run(false, async () => {
    log.push('d');
  });
  await new Promise((r) => setTimeout(r, 20));
  assert.deepEqual(log, ['a start', 'b'], 'c waits for a, and d waits behind c');
  release();
  await Promise.all([a, c, d]);
  assert.deepEqual(log, ['a start', 'b', 'a end', 'c', 'd']);
});

test('a call that fails lets the next one run', async () => {
  const lock = new StoreLock();
  await assert.rejects(
    lock.run(true, async () => {
      throw new Error('async');
    }),
  );
  await assert.rejects(
    lock.run(true, () => {
      throw new Error('sync');
    }),
  );
  assert.equal(await lock.run(true, async () => 7), 7);
  assert.equal(await lock.run(false, async () => 8), 8);
});
