import {
  activityLabel,
  baseName,
  formatDuration,
  formatTokens,
  formatUsd,
  groupByDir,
  modeLabel,
  relativeTime,
  shortPath,
  statusLabel,
  surfaceLabel,
  toolSummary,
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
