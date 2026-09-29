import { SessionSummary } from '../../core/models';
import { sessionMatches } from './sessions.component';

const s = (over: Partial<SessionSummary>): SessionSummary =>
  ({
    id: 's1',
    name: 'auth',
    projectName: 'api',
    cwd: '/Users/me/dev/api',
    usage: { input: 0, output: 10_000, cacheRead: 0, cacheWrite: 0 },
    ...over,
  }) as SessionSummary;

describe('session search', () => {
  it('finds what the card chips say', () => {
    const xhigh = s({ effort: 'xhigh', effortSource: 'observed' });
    // The chip reads "X-high"; the Info tab "Extra high".
    expect(sessionMatches(xhigh, 'x-high')).toBeTrue();
    expect(sessionMatches(xhigh, 'extra high')).toBeTrue();
    expect(sessionMatches(xhigh, 'xhigh')).toBeTrue();
    expect(sessionMatches(s({ effort: 'medium' }), 'med')).toBeTrue();

    const off = s({ thinking: { mode: 'off', source: 'launch' } });
    expect(sessionMatches(off, 'thinking off')).toBeTrue();
    expect(sessionMatches(s({}), 'thinking off')).toBeFalse();
    expect(sessionMatches(s({ ultracode: 'on' }), 'ultracode')).toBeTrue();
    expect(sessionMatches(s({ ultracode: 'off' }), 'ultracode')).toBeFalse();
  });
});
