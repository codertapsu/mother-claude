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
  severityTone,
  shortPath,
  statusLabel,
  surfaceLabel,
  switchPlace,
  thinkingModeLabel,
  thinkingShare,
  thinkingSourceText,
  thinkingText,
  toolSummary,
  ultracodeLabel,
} from './util';
import { FileChange } from '../core/models';

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
