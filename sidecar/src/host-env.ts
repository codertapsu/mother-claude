/**
 * Two pieces of the bridge host's account handling (docs/ACCOUNTS.md D5),
 * kept apart from the host so they can be tested without the SDK:
 *
 * - each conversation's environment, composed from the host's own as it
 *   started plus its account's part;
 * - the lock the store calls take, since the SDK reads the config directory
 *   from `process.env.CLAUDE_CONFIG_DIR` at call time.
 */

/** What an account changes in a conversation's environment. */
export interface AccountEnvSpec {
  set?: Record<string, string>;
  unset?: string[];
}

/**
 * The variables the Agent SDK (0.3.304) removes from the environment it
 * inherits when `query()` gets no `env` — and keeps when it gets one. The
 * host always passes one (a copy of its own, so that a store call pointing
 * `CLAUDE_CONFIG_DIR` elsewhere for a moment never leaks into a
 * conversation), so it removes them itself: a conversation starts exactly as
 * it would have without an `env`.
 */
export const SDK_UNSET_WITHOUT_ENV = [
  'CLAUDE_CODE_QUESTION_EXTENDED',
  'CLAUDE_CODE_QUESTION_OPTIONAL_DESCRIPTIONS',
  'CLAUDE_CODE_AGENT_PROGRESS_SUMMARIES',
] as const;

/** `base` plus the account's part, without what the SDK would drop. */
export function conversationEnv(
  base: Record<string, string | undefined>,
  spec?: AccountEnvSpec,
): Record<string, string | undefined> {
  const env = { ...base };
  const set = spec?.set ?? {};
  for (const name of SDK_UNSET_WITHOUT_ENV) {
    // Matched without regard to case, as the SDK does; kept only when the
    // account itself sets it.
    for (const key of Object.keys(env)) {
      if (key.toUpperCase() === name && !(key in set)) delete env[key];
    }
  }
  for (const key of spec?.unset ?? []) delete env[key];
  for (const [key, value] of Object.entries(set)) env[key] = value;
  return env;
}

/**
 * A readers-writer lock for the store calls. A call for another account's
 * directory points `process.env.CLAUDE_CONFIG_DIR` there while it runs, so it
 * runs alone; the Default's calls leave it alone and share. With only the
 * Default nothing ever waits. Calls start in the order they came, so a
 * waiting exclusive call is not starved by a stream of shared ones.
 */
export class StoreLock {
  private shared = 0;
  private exclusive = false;
  private readonly waiting: Array<{ exclusive: boolean; start: () => void }> = [];

  run<T>(exclusive: boolean, fn: () => Promise<T>): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const start = (): void => {
        if (exclusive) this.exclusive = true;
        else this.shared += 1;
        let done: Promise<T>;
        try {
          done = fn();
        } catch (err) {
          done = Promise.reject(err);
        }
        done.then(resolve, reject).finally(() => {
          if (exclusive) this.exclusive = false;
          else this.shared -= 1;
          this.pump();
        });
      };
      this.waiting.push({ exclusive, start });
      this.pump();
    });
  }

  private pump(): void {
    while (this.waiting.length > 0 && !this.exclusive) {
      const next = this.waiting[0];
      if (next.exclusive && this.shared > 0) return;
      this.waiting.shift();
      next.start();
      if (next.exclusive) return;
    }
  }
}
