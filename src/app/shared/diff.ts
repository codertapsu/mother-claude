/**
 * Unified-diff parsing for the Changes panel's two views:
 *  - Unified (line-by-line): one column with old/new line-number gutters.
 *  - Split (side-by-side): old file left, new file right, del/add runs paired
 *    row-by-row the way GitHub aligns them.
 *
 * Pure functions over `git diff` text; tolerant of preamble lines
 * (diff/index/---/+++) and "\ No newline at end of file" markers.
 */

export type DiffKind = 'add' | 'del' | 'ctx' | 'hunk' | 'meta';

export interface UnifiedRow {
  kind: DiffKind;
  /** 1-based line number in the old file (del/ctx rows). */
  oldNo: number | null;
  /** 1-based line number in the new file (add/ctx rows). */
  newNo: number | null;
  /** Line content WITHOUT the +/-/space marker (hunk/meta keep full text). */
  text: string;
}

export interface SplitCell {
  no: number | null;
  text: string;
  kind: 'add' | 'del' | 'ctx' | 'empty';
}

export interface SplitRow {
  /** Full-width header row (hunk/meta) when set; cells are null. */
  header?: string;
  headerKind?: 'hunk' | 'meta';
  left: SplitCell | null;
  right: SplitCell | null;
}

const HUNK = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;
const EMPTY: SplitCell = { no: null, text: '', kind: 'empty' };

/** Parse a unified diff into typed rows with old/new line numbers. */
export function parseUnifiedDiff(patch: string): UnifiedRow[] {
  // CRLF-file diffs carry a trailing \r per line — strip it (it would render
  // as a stray space / forced break under white-space: pre).
  const lines = patch.split('\n').map((l) => (l.endsWith('\r') ? l.slice(0, -1) : l));
  if (lines.length && lines[lines.length - 1] === '') lines.pop(); // trailing \n
  const rows: UnifiedRow[] = [];
  let oldNo = 0;
  let newNo = 0;
  let inHunk = false;
  for (const line of lines) {
    const h = HUNK.exec(line);
    if (h) {
      oldNo = Number(h[1]);
      newNo = Number(h[2]);
      inHunk = true;
      rows.push({ kind: 'hunk', oldNo: null, newNo: null, text: line });
      continue;
    }
    if (!inHunk) {
      rows.push({ kind: 'meta', oldNo: null, newNo: null, text: line });
      continue;
    }
    // Inside a hunk every line starts with '+', '-', ' ', or '\'; anything
    // else (a new file section in a multi-file patch) ends the hunk.
    if (line.startsWith('+')) {
      rows.push({ kind: 'add', oldNo: null, newNo: newNo++, text: line.slice(1) });
    } else if (line.startsWith('-')) {
      rows.push({ kind: 'del', oldNo: oldNo++, newNo: null, text: line.slice(1) });
    } else if (line.startsWith('\\')) {
      rows.push({ kind: 'meta', oldNo: null, newNo: null, text: line });
    } else if (line.startsWith(' ') || line === '') {
      // (some tools trim fully-blank context lines to '')
      rows.push({ kind: 'ctx', oldNo: oldNo++, newNo: newNo++, text: line.slice(1) });
    } else {
      inHunk = false;
      rows.push({ kind: 'meta', oldNo: null, newNo: null, text: line });
    }
  }
  return rows;
}

/** Pair unified rows into side-by-side rows: context mirrors both sides; a
 * run of deletions pairs index-by-index with the additions that follow it,
 * padding the shorter side with empty cells. */
export function toSplitRows(rows: UnifiedRow[]): SplitRow[] {
  const out: SplitRow[] = [];
  let i = 0;
  while (i < rows.length) {
    const r = rows[i];
    if (r.kind === 'hunk' || r.kind === 'meta') {
      out.push({ header: r.text, headerKind: r.kind, left: null, right: null });
      i++;
      continue;
    }
    if (r.kind === 'ctx') {
      out.push({
        left: { no: r.oldNo, text: r.text, kind: 'ctx' },
        right: { no: r.newNo, text: r.text, kind: 'ctx' },
      });
      i++;
      continue;
    }
    const dels: UnifiedRow[] = [];
    const adds: UnifiedRow[] = [];
    const markers: UnifiedRow[] = [];
    while (i < rows.length && rows[i].kind === 'del') dels.push(rows[i++]);
    // git emits "\ No newline at end of file" BETWEEN a del run and its add
    // run — buffer it so it doesn't split the pairing, and emit it after.
    while (i < rows.length && rows[i].kind === 'meta' && rows[i].text.startsWith('\\')) {
      markers.push(rows[i++]);
    }
    while (i < rows.length && rows[i].kind === 'add') adds.push(rows[i++]);
    for (let k = 0; k < Math.max(dels.length, adds.length); k++) {
      out.push({
        left: dels[k] ? { no: dels[k].oldNo, text: dels[k].text, kind: 'del' } : EMPTY,
        right: adds[k] ? { no: adds[k].newNo, text: adds[k].text, kind: 'add' } : EMPTY,
      });
    }
    for (const m of markers) {
      out.push({ header: m.text, headerKind: 'meta', left: null, right: null });
    }
  }
  return out;
}
