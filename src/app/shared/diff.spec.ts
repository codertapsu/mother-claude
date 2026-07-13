import { parseUnifiedDiff, toSplitRows } from './diff';

const PATCH = [
  'diff --git a/src/x.ts b/src/x.ts',
  'index 111..222 100644',
  '--- a/src/x.ts',
  '+++ b/src/x.ts',
  '@@ -10,4 +10,5 @@ fn header',
  ' keep one',
  '-old line',
  '+new line',
  '+added line',
  ' keep two',
  '@@ -30,2 +31,1 @@',
  ' tail',
  '-gone',
  '\\ No newline at end of file',
].join('\n');

describe('parseUnifiedDiff', () => {
  it('numbers old/new lines from hunk headers', () => {
    const rows = parseUnifiedDiff(PATCH);
    const kinds = rows.map((r) => r.kind);
    expect(kinds.slice(0, 5)).toEqual(['meta', 'meta', 'meta', 'meta', 'hunk']);

    const ctx1 = rows[5];
    expect(ctx1).toEqual({ kind: 'ctx', oldNo: 10, newNo: 10, text: 'keep one' });
    expect(rows[6]).toEqual({ kind: 'del', oldNo: 11, newNo: null, text: 'old line' });
    expect(rows[7]).toEqual({ kind: 'add', oldNo: null, newNo: 11, text: 'new line' });
    expect(rows[8]).toEqual({ kind: 'add', oldNo: null, newNo: 12, text: 'added line' });
    expect(rows[9]).toEqual({ kind: 'ctx', oldNo: 12, newNo: 13, text: 'keep two' });

    // Second hunk renumbers from its own header.
    expect(rows[11]).toEqual({ kind: 'ctx', oldNo: 30, newNo: 31, text: 'tail' });
    expect(rows[12]).toEqual({ kind: 'del', oldNo: 31, newNo: null, text: 'gone' });
    expect(rows[13].kind).toBe('meta'); // \ No newline marker
  });

  it('tolerates empty patches and blank context lines', () => {
    expect(parseUnifiedDiff('')).toEqual([]);
    const rows = parseUnifiedDiff('@@ -1,2 +1,2 @@\n \n-a\n+b');
    expect(rows[1]).toEqual({ kind: 'ctx', oldNo: 1, newNo: 1, text: '' });
  });
});

describe('toSplitRows', () => {
  it('pairs del/add runs and mirrors context', () => {
    const split = toSplitRows(parseUnifiedDiff(PATCH));
    const headers = split.filter((r) => r.header);
    expect(headers.length).toBe(7); // 4 preamble meta + 2 hunks + 1 no-newline marker

    // ctx mirrored
    const ctx = split.find((r) => r.left?.kind === 'ctx' && r.left.text === 'keep one')!;
    expect(ctx.right).toEqual({ no: 10, text: 'keep one', kind: 'ctx' });

    // 1 del pairs with first add; second add pads left with empty
    const pair1 = split.find((r) => r.left?.text === 'old line')!;
    expect(pair1.right?.text).toBe('new line');
    const pair2 = split.find((r) => r.right?.text === 'added line')!;
    expect(pair2.left?.kind).toBe('empty');

    // del with no following add pads right with empty
    const delOnly = split.find((r) => r.left?.text === 'gone')!;
    expect(delOnly.right?.kind).toBe('empty');
  });

  it('pairs across a no-newline marker between del and add runs', () => {
    const split = toSplitRows(
      parseUnifiedDiff('@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new'),
    );
    const pair = split.find((r) => r.left?.text === 'old')!;
    expect(pair.right?.text).toBe('new'); // one side-by-side row, not two halves
    expect(split.some((r) => r.header?.startsWith('\\'))).toBeTrue(); // marker kept
  });

  it('strips CRLF carriage returns from patch lines', () => {
    const rows = parseUnifiedDiff('@@ -1,1 +1,1 @@\r\n-a\r\n+b\r\n');
    expect(rows[1].text).toBe('a');
    expect(rows[2].text).toBe('b');
  });

  it('handles an add-only run (new file)', () => {
    const split = toSplitRows(parseUnifiedDiff('@@ -0,0 +1,2 @@\n+one\n+two'));
    expect(split[1].left?.kind).toBe('empty');
    expect(split[1].right).toEqual({ no: 1, text: 'one', kind: 'add' });
    expect(split[2].right).toEqual({ no: 2, text: 'two', kind: 'add' });
  });
});
