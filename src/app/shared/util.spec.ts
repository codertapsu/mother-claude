import { useLanguage } from '../core/i18n/i18n';
import {
  activityLabel,
  baseName,
  effortShortLabel,
  effortSourceText,
  formatCountdown,
  formatDuration,
  formatMinor,
  formatTokens,
  formatUsd,
  groupByDir,
  modeLabel,
  relativeTime,
  resetsIn,
  resumeNote,
  resumesIn,
  severityTone,
  shortPath,
  statusLabel,
  statusTone,
  stoppedBy,
  surfaceLabel,
  switchPlace,
  thinkingModeLabel,
  thinkingShare,
  thinkingSourceText,
  thinkingText,
  toolSummary,
  ultracodeLabel,
} from './util';
import { FileChange, SessionSummary } from '../core/models';

const fc = (path: string): FileChange => ({ path, status: 'modified', additions: 1, deletions: 0 });

describe('shared/util', () => {
  it('formats token counts', () => {
    expect(formatTokens(0)).toBe('0');
    expect(formatTokens(950)).toBe('950');
    expect(formatTokens(1500)).toBe('1.5k');
    expect(formatTokens(15_400)).toBe('15k');
    expect(formatTokens(2_500_000)).toBe('2.50M');
  });

  it('formats money and durations', () => {
    expect(formatUsd(undefined)).toBe('—');
    expect(formatUsd(0.004)).toBe('<$0.01');
    expect(formatUsd(1.844)).toBe('$1.84');
    expect(formatDuration(12_000)).toBe('12s');
    expect(formatDuration(184_000)).toBe('3m 04s');
    expect(formatDuration(3_720_000)).toBe('1h 02m');
  });

  it('formats relative time', () => {
    const now = 1_000_000_000;
    expect(relativeTime(undefined, now)).toBe('—');
    expect(relativeTime(now - 1_000, now)).toBe('just now');
    expect(relativeTime(now - 12_000, now)).toBe('12s ago');
    expect(relativeTime(now - 3 * 60_000, now)).toBe('3m ago');
  });

  it('labels statuses, surfaces, modes and activity', () => {
    expect(statusLabel('waiting')).toBe('Needs you');
    expect(statusLabel('mystery')).toBe('Mystery');
    expect(surfaceLabel('vscode')).toBe('VS Code');
    expect(surfaceLabel('unknown')).toBe('Unknown');
    expect(modeLabel('acceptEdits')).toBe('Accept edits');
    expect(modeLabel(undefined)).toBe('Ask');
    expect(activityLabel({ kind: 'tool', label: '', tool: 'Bash' })).toBe('Running Bash');
    expect(activityLabel({ kind: 'thinking', label: 'Pondering' })).toBe('Pondering');
    expect(activityLabel({ kind: 'delegating', label: 'Running 3 agents' })).toBe(
      'Running 3 agents',
    );
  });

  it("shows the server's activity labels in Vietnamese", async () => {
    try {
      await useLanguage('vi');
      expect(activityLabel({ kind: 'delegating', label: 'Running 3 agents' })).toBe(
        'Đang chạy 3 tác tử',
      );
      expect(activityLabel({ kind: 'tool', label: 'Running Bash' })).toBe('Đang chạy Bash');
      expect(activityLabel({ kind: 'thinking', label: 'Thinking' })).toBe('Đang suy nghĩ');
      expect(activityLabel({ kind: 'thinking', label: 'Pondering' })).toBe('Pondering');
    } finally {
      await useLanguage('en');
    }
  });

  it('labels effort, ultracode and thinking, with how each is known', () => {
    expect(effortShortLabel('xhigh')).toBe('X-high');
    expect(effortShortLabel('medium')).toBe('Med');
    expect(effortShortLabel('max')).toBe('Max');
    expect(effortShortLabel('turbo')).toBe('turbo');
    expect(effortShortLabel(undefined)).toBe('');
    expect(effortSourceText('observed')).toBe('as sent with the latest request');
    expect(effortSourceText('launch')).toBe('set at launch');
    expect(effortSourceText('mystery')).toBe('');
    expect(switchPlace('vscode')).toBe('in VS Code');
    expect(switchPlace('tmux')).toBe('in the terminal');
    expect(switchPlace('desktop')).toBe('in Claude Desktop');
    expect(switchPlace('background')).toBe('');
    expect(switchPlace(undefined)).toBe('');
    expect(ultracodeLabel('on')).toBe('On');
    expect(ultracodeLabel('unavailable')).toBe('Unavailable');
    expect(ultracodeLabel(undefined)).toBe('');
    expect(thinkingModeLabel('adaptive')).toBe('Adaptive');
    expect(thinkingText({ mode: 'on', source: 'launch', budget: 31_999 })).toBe('On · 32k budget');
    expect(thinkingText({ mode: 'adaptive', source: 'launch', budget: 31_999 })).toBe('Adaptive');
    expect(thinkingSourceText('default')).toBe("Claude Code's default");
    expect(thinkingSourceText('control')).toBe('set by Mother Claude');
  });

  it("computes thinking's share of the output", () => {
    const u = { input: 0, cacheRead: 0, cacheWrite: 0 };
    expect(thinkingShare({ ...u, output: 41_000, thinking: 9_800 })).toBe(24);
    expect(thinkingShare({ ...u, output: 41_000 })).toBeNull();
    expect(thinkingShare({ ...u, output: 0, thinking: 10 })).toBeNull();
    expect(thinkingShare({ ...u, output: 10_000, thinking: 1 })).toBe(1);
    expect(thinkingShare({ ...u, output: 10, thinking: 40 })).toBe(100);
  });

  it('names a session that resumes by itself by what wakes it', () => {
    expect(statusLabel('background')).toBe('Background work');
    expect(statusLabel('background', { kind: 'background' })).toBe('Background work');
    expect(statusLabel('background', { kind: 'scheduled' })).toBe('Scheduled');
    expect(statusLabel('idle', { kind: 'scheduled' })).toBe('Idle');
    expect(statusTone('background')).toBe('accent');
    expect(activityLabel({ kind: 'scheduled', label: '' })).toBe('Scheduled to resume');
    const now = 1_000_000_000;
    expect(resumesIn(now + 12 * 60_000, now)).toBe('in 12m');
    expect(resumesIn(now - 1, now)).toBe('due now');
    expect(resumesIn(undefined, now)).toBe('');
  });

  it('says whether closing the app that runs a session stops what resumes it', () => {
    const now = 1_000_000_000;
    const s = (over: Partial<SessionSummary>) =>
      ({
        live: true,
        owned: false,
        surface: 'vscode',
        status: 'background',
        activity: { kind: 'background', label: 'Background work' },
        ...over,
      }) as SessionSummary;
    expect(resumeNote(s({}), now)).toBe(
      'Claude picks this up when its background work finishes — closing VS Code now stops it.',
    );
    const wakeup = (resumesAt?: number) =>
      ({ kind: 'scheduled', label: 'Scheduled to resume', resumesAt }) as const;
    expect(resumeNote(s({ surface: 'terminal', activity: wakeup(now + 12 * 60_000) }), now)).toBe(
      'Claude resumes this by itself in 12m — closing its terminal now stops it.',
    );
    expect(resumeNote(s({ surface: 'jetbrains', activity: wakeup(now - 1) }), now)).toBe(
      'Claude resumes this by itself any moment — closing JetBrains now stops it.',
    );
    expect(resumeNote(s({ surface: 'desktop', activity: wakeup() }), now)).toBe(
      'Claude resumes this by itself on its schedule — quitting Claude Desktop now stops it.',
    );
    // Owned: it runs in Mother Claude.
    expect(resumeNote(s({ owned: true, surface: 'mother-claude' }), now)).toContain(
      '— quitting Mother Claude now stops it.',
    );
    // A tmux pane and a `--bg` job outlive a closed window.
    expect(resumeNote(s({ surface: 'tmux' }), now)).toBe(
      "Claude picks this up when its background work finishes. It runs in tmux, so closing a window doesn't stop it.",
    );
    expect(resumeNote(s({ surface: 'background' }), now)).toContain(
      "It runs as a background job, so closing a window doesn't stop it.",
    );
    expect(stoppedBy({ surface: 'unknown', owned: false })).toBe('closing the app that runs it');
    // Nothing to say of any other session.
    expect(resumeNote(s({ status: 'idle' }), now)).toBe('');
    expect(resumeNote(s({ live: false }), now)).toBe('');
  });

  it('counts down to a reset', () => {
    const now = 1_000_000_000;
    expect(formatCountdown(45_000)).toBe('<1m');
    expect(formatCountdown(12 * 60_000)).toBe('12m');
    expect(formatCountdown(2 * 3_600_000 + 9 * 60_000 + 30_000)).toBe('2h 10m');
    expect(formatCountdown(3_600_000)).toBe('1h');
    expect(formatCountdown(76 * 3_600_000)).toBe('3d 4h');
    expect(resetsIn(now + 2 * 3_600_000 + 10 * 60_000, now)).toBe('resets in 2h 10m');
    expect(resetsIn(now - 1, now)).toBe('reset');
    expect(resetsIn(undefined, now)).toBe('');
  });

  it('formats minor units and severity tones', () => {
    const usd = new Intl.NumberFormat(undefined, { style: 'currency', currency: 'USD' });
    expect(formatMinor(1_240, 'USD', 2)).toBe(usd.format(12.4));
    expect(formatMinor(5, 'JPY', 0)).toBe(
      new Intl.NumberFormat(undefined, { style: 'currency', currency: 'JPY' }).format(5),
    );
    expect(formatMinor(500, 'ZZZ-not-a-currency', 2)).toBe('5.00 ZZZ-not-a-currency');
    expect(severityTone('normal')).toBe('accent');
    expect(severityTone('warning')).toBe('amber');
    expect(severityTone('critical')).toBe('red');
  });

  it('shortens home paths', () => {
    expect(shortPath('/Users/marcus/dev/app')).toBe('~/dev/app');
    expect(shortPath('/opt/x')).toBe('/opt/x');
    expect(shortPath('C:\\Users\\marcus\\dev\\app')).toBe('~\\dev\\app');
    expect(shortPath('D:\\NAS\\MotherClaude')).toBe('D:\\NAS\\MotherClaude');
  });

  it('summarizes tool inputs by their salient argument', () => {
    expect(toolSummary('Bash', { command: 'npm test' })).toBe('npm test');
    expect(toolSummary('Bash', { command: 'npm test', description: 'Run tests' })).toBe(
      'Run tests',
    );
    expect(toolSummary('Edit', { file_path: '/a/b.ts', old_string: 'x' })).toBe('/a/b.ts');
    expect(toolSummary('Bash', { command: 'x'.repeat(300) }).length).toBeLessThanOrEqual(161);
    expect(toolSummary('TodoWrite', { todos: [] })).toBe('');
    expect(toolSummary('Agent', { description: 'Find the bug', prompt: 'long' })).toBe(
      'Find the bug',
    );
  });

  it('groups changed files by folder, root first', () => {
    const groups = groupByDir([
      fc('src/app/a.ts'),
      fc('README.md'),
      fc('src/app/b.ts'),
      fc('docs/x.md'),
    ]);
    expect(groups.map((g) => g.dir)).toEqual(['.', 'docs', 'src/app']);
    expect(groups[2].files.map((f) => f.path)).toEqual(['src/app/a.ts', 'src/app/b.ts']);
    expect(baseName('src/app/a.ts')).toBe('a.ts');
    expect(baseName('README.md')).toBe('README.md');
  });
});
