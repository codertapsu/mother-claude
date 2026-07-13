import {
  baseName,
  classifyPatchLine,
  formatTokens,
  groupByDir,
  relativeTime,
  stateLabel,
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
    expect(formatTokens(2_500_000)).toBe('2.50M');
  });

  it('formats relative time', () => {
    expect(relativeTime(undefined)).toBe('—');
    expect(relativeTime(Date.now() - 5_000)).toMatch(/s ago$/);
    expect(relativeTime(Date.now() - 3 * 60_000)).toMatch(/m ago$/);
  });

  it('labels states and surfaces', () => {
    expect(stateLabel('needs-input')).toBe('needs input');
    expect(surfaceLabel('vs-code')).toBe('VS Code');
    expect(surfaceLabel('cli')).toBe('CLI');
    expect(surfaceLabel('unknown')).toBe('Unknown');
  });

  it('summarizes tool inputs by their salient argument', () => {
    expect(toolSummary('Bash', { command: 'npm test' })).toBe('npm test');
    expect(toolSummary('Edit', { file_path: '/a/b.ts', old_string: 'x' })).toBe('/a/b.ts');
    expect(toolSummary('Bash', { command: 'x'.repeat(300) }).length).toBeLessThanOrEqual(161);
    expect(toolSummary('TodoWrite', { todos: [] })).toBe('');
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

  it('classifies unified-diff lines', () => {
    expect(classifyPatchLine('+added')).toBe('add');
    expect(classifyPatchLine('-removed')).toBe('del');
    expect(classifyPatchLine('+++ b/file')).toBe('meta');
    expect(classifyPatchLine('--- a/file')).toBe('meta');
    expect(classifyPatchLine('@@ -1,3 +1,4 @@')).toBe('hunk');
    expect(classifyPatchLine('diff --git a/x b/x')).toBe('meta');
    expect(classifyPatchLine(' context')).toBe('ctx');
    // content lines that merely start with the header characters
    expect(classifyPatchLine('+++i;')).toBe('add');
    expect(classifyPatchLine('--- foo')).toBe('meta');
    expect(classifyPatchLine('---foo')).toBe('del');
  });
});
