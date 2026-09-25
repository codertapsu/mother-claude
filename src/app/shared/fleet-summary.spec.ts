import { SessionSummary } from '../core/models';
import { fleetCounts } from './fleet-summary.component';

const s = (over: Record<string, unknown>): SessionSummary =>
  ({ id: 'x', live: true, status: 'idle', pending: [], ...over }) as unknown as SessionSummary;

describe('fleetCounts', () => {
  const midnight = 1_000;

  it('counts like the server fleet summary', () => {
    const c = fleetCounts(
      [
        s({ status: 'working', agents: { running: 2, backgroundRunning: 3 } }),
        s({ status: 'starting' }),
        s({ live: false, status: 'working' }),
        s({ status: 'waiting', pending: [{ answerable: false }] }),
        s({ status: 'waiting', pending: [{ answerable: false }, { answerable: true }] }),
      ],
      midnight,
    );
    expect(c.working).toBe(1);
    expect(c.agentsRunning).toBe(2);
    expect(c.needYou).toBe(1);
  });

  it("sums each session's costToday, and falls back to tokens when unpriced", () => {
    const priced = fleetCounts(
      [
        // Lifetime cost is not today's: only what was priced since midnight.
        s({ lastActivityAt: 2_000, costUsd: 5, costToday: 1.004 }),
        s({ lastActivityAt: 2_000, costUsd: 2, costToday: 2 }),
        s({ lastActivityAt: 500, startedAt: 2_000, costUsd: 9 }),
      ],
      midnight,
    );
    expect(priced.costToday).toBe(3);
    expect(priced.todaySessions).toBe(2);
    expect(priced.priced).toBeTrue();

    const unpriced = fleetCounts(
      [s({ lastActivityAt: 2_000, usage: { input: 10, output: 5 } })],
      midnight,
    );
    expect(unpriced.priced).toBeFalse();
    expect(unpriced.tokensToday).toBe(15);
  });
});
