// Reconnect: exponential backoff 1 s → 30 s with jitter, forever while paired.

export const BACKOFF_MIN_MS = 1_000;
export const BACKOFF_MAX_MS = 30_000;

/**
 * Delay before reconnect attempt number `attempt` (0 for the first retry):
 * 1 s doubling up to 30 s, each spread by ±20 % so browsers that lost the
 * desktop together don't come back in step, and never outside 1–30 s.
 */
export function backoffDelay(attempt: number, random: () => number = Math.random): number {
  const base = Math.min(BACKOFF_MAX_MS, BACKOFF_MIN_MS * 2 ** Math.max(0, Math.min(attempt, 10)));
  const jittered = base * (0.8 + 0.4 * random());
  return Math.round(Math.min(BACKOFF_MAX_MS, Math.max(BACKOFF_MIN_MS, jittered)));
}
