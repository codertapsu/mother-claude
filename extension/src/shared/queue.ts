// One command at a time per tab, in order; different tabs overlap
// (docs/BROWSER-PROTOCOL.md, "Limits and rules both sides keep").

import { DEADLINE_MARGIN_MS } from './protocol.ts';

/** Runs tasks one after another per key; tasks under different keys run concurrently. */
export class KeyedQueue {
  private readonly tails = new Map<string, Promise<void>>();

  /** Number of keys with work queued or running (for tests and diagnostics). */
  get busyKeys(): number {
    return this.tails.size;
  }

  /**
   * Run `task` when the key is free. When `signal` aborts, the slot is freed at
   * once (the next task may start) even if `task` is still finishing: an
   * aborted command must not hold its tab until its own work times out.
   */
  run<T>(key: string, task: () => Promise<T>, signal?: AbortSignal): Promise<T> {
    const previous = this.tails.get(key) ?? Promise.resolve();
    let release!: () => void;
    const done = new Promise<void>((resolve) => (release = resolve));
    const tail = previous.then(() => done);
    this.tails.set(key, tail);
    const cleanup = (): void => {
      release();
      if (this.tails.get(key) === tail) this.tails.delete(key);
    };
    return previous.then(async () => {
      if (signal?.aborted) {
        cleanup();
        throw abortReason(signal);
      }
      let onAbort: (() => void) | undefined;
      const aborted = signal
        ? new Promise<never>((_, reject) => {
            onAbort = () => {
              cleanup();
              reject(abortReason(signal));
            };
            signal.addEventListener('abort', onAbort, { once: true });
          })
        : null;
      try {
        return await (aborted ? Promise.race([task(), aborted]) : task());
      } finally {
        if (onAbort) signal?.removeEventListener('abort', onAbort);
        cleanup();
      }
    });
  }
}

/**
 * How long the extension may work on a command before it must answer: the
 * desktop's `timeoutMs` less 500 ms, so the desktop always gets a result.
 * A deadline shorter than 2 s keeps three quarters of it instead.
 */
export function workBudgetMs(timeoutMs: number): number {
  const margin = Math.min(DEADLINE_MARGIN_MS, Math.floor(timeoutMs / 4));
  return Math.max(0, timeoutMs - margin);
}

export class DeadlineError extends Error {
  constructor() {
    super('deadline reached');
    this.name = 'DeadlineError';
  }
}

/**
 * Resolve with `work()`, or reject with DeadlineError at `deadline` (epoch ms)
 * and abort `controller` so the work stops at its next check.
 */
export function withDeadline<T>(
  deadline: number,
  controller: AbortController,
  work: () => Promise<T>,
  now: () => number = Date.now,
): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const remaining = deadline - now();
    if (remaining <= 0) {
      controller.abort(new DeadlineError());
      reject(new DeadlineError());
      return;
    }
    const timer = setTimeout(() => {
      controller.abort(new DeadlineError());
      reject(new DeadlineError());
    }, remaining);
    work().then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error: unknown) => {
        clearTimeout(timer);
        reject(error instanceof Error ? error : new Error(String(error)));
      },
    );
  });
}

/** Sleep that ends early (rejecting) when the signal aborts. */
export function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise<void>((resolve, reject) => {
    if (signal?.aborted) {
      reject(abortReason(signal));
      return;
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener('abort', onAbort);
      resolve();
    }, ms);
    const onAbort = (): void => {
      clearTimeout(timer);
      reject(abortReason(signal));
    };
    signal?.addEventListener('abort', onAbort, { once: true });
  });
}

export function abortReason(signal: AbortSignal | undefined): Error {
  const r: unknown = signal?.reason;
  return r instanceof Error ? r : new DeadlineError();
}
