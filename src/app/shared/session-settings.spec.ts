import { SessionSummary } from '../core/models';
import {
  effortText,
  explainFast,
  explainThinking,
  explainUltracode,
  fastText,
  noEffortLevels,
  thinkingChip,
  thinkingEvidence,
  thinkingLabel,
  thinkingOffContradicted,
  thinkingSettingText,
  thinkingTitle,
  thoughtThisRun,
  ultracodeText,
  ultracodeUnknownText,
  ultracodeUnknownTitle,
} from './session-settings';

const NOW = 1_000_000_000_000;
const M = 60_000;
const s = (over: Partial<SessionSummary>): SessionSummary =>
  ({
    live: true,
    usage: { input: 0, output: 10_000, cacheRead: 0, cacheWrite: 0 },
    ...over,
  }) as SessionSummary;

describe('session settings texts', () => {
  it('says how the effort is known', () => {
    expect(effortText(s({ effort: 'max', effortSource: 'observed' }))).toBe(
      'Max (as sent with the latest request)',
    );
    expect(effortText(s({ effort: 'xhigh' }))).toBe('Extra high');
    expect(effortText(s({}))).toBe('');
  });

  it('tells a model without effort levels apart from an unknown effort', () => {
    // Claude Code applied nothing: the model has no effort levels (Haiku).
    expect(noEffortLevels(s({ effortSource: 'applied' }))).toBeTrue();
    expect(noEffortLevels(s({}))).toBeFalse();
    expect(noEffortLevels(s({ effortSource: 'requested' }))).toBeFalse();
    expect(noEffortLevels(s({ effort: 'high', effortSource: 'applied' }))).toBeFalse();
  });

  it('dates a transcript ultracode value to the last message', () => {
    const vs = s({ ultracode: 'on', ultracodeSource: 'transcript', ultracodeAt: NOW - 6 * M });
    expect(ultracodeText(vs, NOW)).toBe('On (as of the last message, 6m ago)');
    expect(ultracodeText(s({ ultracode: 'off', ultracodeSource: 'applied' }), NOW)).toBe(
      'Off (as Claude Code applies it)',
    );
    expect(ultracodeText(s({ ultracode: 'on', ultracodeSource: 'launch' }), NOW)).toBe(
      'On (set at launch)',
    );
    expect(ultracodeText(s({ ultracode: 'unavailable' }), NOW)).toContain("can't run it");
    const asked = s({ ultracode: 'on', ultracodeSource: 'requested' });
    expect(ultracodeText(asked, NOW)).toBe('On (requested, not confirmed yet)');
    expect(explainUltracode(asked, NOW).join(' ')).toContain("hasn't confirmed it yet");
    expect(ultracodeText(s({}), NOW)).toBe('');
    const why = explainUltracode({ ...vs, surface: 'vscode' }, NOW).join(' ');
    expect(why).toContain('As of the last message (6m ago)');
    expect(why).toContain('a switch made in VS Code since then shows here after the next message');
    // An ended session's value is simply its last one.
    expect(explainUltracode({ ...vs, live: false }, NOW)[1]).toBe(
      'As of the last message (6m ago).',
    );
  });

  it('says why ultracode is not known, and when it will be', () => {
    const vscode = (over: Partial<SessionSummary>) => s({ surface: 'vscode', ...over });
    // A new conversation: nothing until its first message.
    const fresh = vscode({ ultracodeUnknown: 'no-prompt' });
    expect(ultracodeUnknownText(fresh, NOW)).toBe('shows after the first message in VS Code');
    expect(ultracodeUnknownTitle(fresh, NOW)).toBe(
      'Ultracode: not known; shows after the first message in VS Code',
    );
    expect(explainUltracode(fresh, NOW).join(' ')).toContain(
      'only when a message is sent, so it shows here after the first message in VS Code',
    );
    expect(explainUltracode(fresh, NOW).join(' ')).toContain('Ultracode is set per session');
    // Restarted: what it was before says nothing about now.
    const restarted = vscode({
      ultracodeUnknown: 'restarted',
      ultracodeBefore: 'on',
      ultracodeBeforeAt: NOW - 10 * 60 * M,
    });
    expect(ultracodeUnknownText(restarted, NOW)).toBe(
      'was on at the last message (10h ago), before the restart',
    );
    expect(ultracodeUnknownTitle(restarted, NOW)).toBe(
      'Ultracode: not known since the session restarted (it was on); the next message in VS Code shows it',
    );
    const lines = explainUltracode(restarted, NOW);
    expect(lines[0]).toBe(
      'Not known since this session restarted. It was on at the last message before the restart (10h ago).',
    );
    expect(lines[1]).toContain(
      'A restart turns ultracode off until it is switched on again in VS Code',
    );
    // Still reading, compacted, too old a version; the terminal by name.
    expect(ultracodeUnknownText(vscode({ ultracodeUnknown: 'reading' }), NOW)).toBe(
      "still reading this session's history",
    );
    expect(explainUltracode(vscode({ ultracodeUnknown: 'compacted' }), NOW)[0]).toContain(
      'The conversation was compacted',
    );
    expect(ultracodeUnknownText(vscode({ ultracodeUnknown: 'unsupported' }), NOW)).toBe(
      "this Claude Code version doesn't note it",
    );
    expect(ultracodeUnknownText(s({ surface: 'terminal' }), NOW)).toBe(
      'shows after the next message in the terminal',
    );
    // Owned: Claude Code reports it by itself in a moment.
    expect(ultracodeUnknownText(s({ owned: true, surface: 'mother-claude' }), NOW)).toBe(
      "Claude Code hasn't reported it yet",
    );
    // Ended: not recorded, with why when there is one.
    expect(ultracodeUnknownText(s({ live: false }), NOW)).toBe('');
    expect(explainUltracode(s({ live: false }), NOW)).toEqual(['Not recorded for this session.']);
    expect(ultracodeUnknownText(s({ live: false, ultracodeUnknown: 'no-prompt' }), NOW)).toBe(
      'no message was sent',
    );
    // Known: nothing to explain away.
    expect(ultracodeUnknownText(vscode({ ultracode: 'off' }), NOW)).toBe('');
  });

  it('never states a thinking setting it does not know', () => {
    const vs = s({
      thinking: { mode: 'adaptive', source: 'launch' },
      usage: { input: 0, output: 12_500, cacheRead: 0, cacheWrite: 0, thinking: 3_100 },
      lastThinkingAt: NOW - 3 * M,
    });
    expect(thinkingSettingText(vs)).toBe('Adaptive (at launch)');
    expect(
      thinkingSettingText(s({ thinking: { mode: 'on', source: 'control', budget: 16_000 } })),
    ).toBe('On · 16k budget (set by Mother Claude)');
    expect(thinkingSettingText(s({}))).toBe('');
    expect(thinkingEvidence(vs, NOW)).toBe('3.1k tokens · 25% of output · last used 3m ago');
    expect(thinkingEvidence(s({}), NOW)).toBe('No thinking tokens so far');
    expect(thinkingEvidence(s({ live: false }), NOW)).toBe('No thinking tokens');

    const why = explainThinking(vs, NOW).join(' ');
    expect(why).toContain('without Mother Claude seeing it');
    expect(why).toContain('3.1k thinking tokens (25% of the output), most recently 3m ago');
    expect(explainThinking(s({}), NOW)[0]).toContain("doesn't report whether thinking is on");
    // No tokens under adaptive thinking is not "off".
    expect(
      explainThinking(s({ thinking: { mode: 'adaptive', source: 'default' } }), NOW).join(' '),
    ).toContain("doesn't mean it is off");
  });

  it('gives the list card a thinking chip only when there is something to say', () => {
    expect(thinkingChip(s({ thinking: { mode: 'off', source: 'launch' } }))?.text).toBe(
      'Thinking off',
    );
    expect(
      thinkingChip(
        s({ usage: { input: 0, output: 41_000, cacheRead: 0, cacheWrite: 0, thinking: 9_800 } }),
      )?.text,
    ).toBe('Thinking 24%');
    expect(thinkingChip(s({ thinking: { mode: 'adaptive', source: 'default' } }))).toBeNull();
    expect(thinkingChip(s({ thinking: { mode: 'off', source: 'launch' } }))?.short).toBe('Off');
  });

  it('says an unavailable ultracode request stays in place', () => {
    const lines = explainUltracode(
      s({ ultracode: 'unavailable', ultracodeSource: 'applied' }),
      NOW,
    );
    expect(lines[0]).toContain("it can't run here");
    expect(lines[1]).toContain('comes back on by itself');
  });

  it('distrusts a launch or default "off" once the session has thought since', () => {
    const tokens = { input: 0, output: 10_000, cacheRead: 0, cacheWrite: 0, thinking: 2_500 };
    const started = NOW - 30 * M;
    const off = (source: 'launch' | 'default' | 'control', lastThinkingAt?: number) =>
      s({ thinking: { mode: 'off', source }, startedAt: started, lastThinkingAt, usage: tokens });

    // This process thought (within the start-up slack, too).
    expect(thoughtThisRun(off('launch', started - 2_000))).toBeTrue();
    expect(thoughtThisRun(off('launch', started - 60_000))).toBeFalse();
    expect(thoughtThisRun(off('launch'))).toBeFalse();
    // An ended session's thinking all counts.
    expect(thoughtThisRun(s({ live: false, startedAt: started, lastThinkingAt: 1 }))).toBeTrue();

    const since = off('launch', NOW - M);
    expect(thinkingOffContradicted(since)).toBeTrue();
    expect(thinkingChip(since)?.text).toBe('Thinking 25%');
    expect(thinkingLabel(since)).toBe('Thinking 25%');
    expect(thinkingTitle(since)).toBe(
      'Thinking: Off (at launch), but used since (2.5k tokens, 25% of the output)',
    );
    expect(explainThinking(since, NOW)[0]).toContain('off when the session was launched');
    expect(explainThinking(off('default', NOW - M), NOW)[0]).toContain(
      'Your Claude settings turn thinking off by default',
    );
    // What Mother Claude set stands; an old run's thinking says nothing.
    expect(thinkingOffContradicted(off('control', NOW - M))).toBeFalse();
    expect(thinkingLabel(off('control', NOW - M))).toBe('Thinking off');
    expect(thinkingLabel(off('launch', NOW - 60 * M))).toBe('Thinking off');
    expect(explainThinking(off('launch', NOW - 60 * M), NOW)[0]).toBe('Thinking is off.');
  });

  it('labels the header thinking chip by what is known', () => {
    expect(thinkingLabel(s({ thinking: { mode: 'adaptive', source: 'default' } }))).toBe(
      'Thinking adaptive',
    );
    expect(thinkingTitle(s({ thinking: { mode: 'adaptive', source: 'default' } }))).toBe(
      "Thinking: Adaptive (Claude Code's default)",
    );
    expect(thinkingLabel(s({}))).toBe('');
    expect(thinkingTitle(s({}))).toBe("Thinking: the setting isn't reported");
  });

  it("words fast mode by how it's known", () => {
    expect(fastText(s({ owned: false }))).toBe('the latest request ran in fast mode');
    expect(fastText(s({ owned: true }))).toBe('switched on, or the latest request ran in it');
    expect(fastText(s({ owned: true, live: false }))).toBe('the latest request ran in fast mode');
    expect(explainFast(s({ owned: true }))).toContain('reports its faster output mode');
    expect(explainFast(s({ owned: false }))).toContain('the latest request ran');
  });
});
