import { tr } from '../core/i18n/i18n';

/**
 * Safe markdown → HTML for transcript prose (messages, thinking, plans,
 * agents' results), close to what GitHub shows.
 *
 * Escape-first: ALL input is HTML-escaped before any markup is added, so
 * transcript content can never inject tags or attributes: every tag and
 * attribute in the result is one this file writes. The result is bound via
 * [innerHTML], where Angular's built-in sanitizer is a second layer; the
 * markup uses only what it keeps (plain elements, and `class`, `role`,
 * `tabindex`, `aria-label`, `title`, `start`, `href`, `target`, `rel`), so
 * it strips nothing.
 *
 * Blocks: headings (# → h3, so cards keep their hierarchy), paragraphs (a
 * single newline stays a line break), fenced code (``` or ~~~, inside list
 * items too; the info string's first word is its language, and a Copy
 * control that `CodeCopyDirective` works), GFM tables (alignment, `\|`,
 * pipes inside code spans, ragged rows; in a region that scrolls sideways),
 * lists (nested by indentation, continuation lines, an ordered list's start
 * number, `[ ]`/`[x]` task items), blockquotes holding their own blocks, and
 * rules.
 *
 * Inline: `code` (``double`` too), **bold**, *italics*, ***both***, __bold__
 * and _italics_ at word boundaries only (snake_case stays), ~~strike~~,
 * [links](https://…), <https://…> and bare http(s) URLs (trailing
 * punctuation left out), images as a link to the image (never loaded),
 * `<br>`, and backslash escapes of the characters above. Only http and https
 * URLs become links; any other target shows its text alone.
 *
 * Linear in the input, as transcripts render thousands of messages: no
 * regular expression that can backtrack, emphasis by CommonMark's delimiter
 * algorithm (with its openers-bottom bound), and quotes and lists nest at
 * most {@link MAX_DEPTH} deep. The few words in the markup (Copy, Table…)
 * are in the language shown, so a `computed` calling this follows a switch.
 */
export function renderMarkdown(text: string): string {
  const labels: Labels = {
    copy: escapeHtml(tr('Copy')),
    copyCode: escapeHtml(tr('Copy code')),
    table: escapeHtml(tr('Table')),
    done: escapeHtml(tr('Done')),
    todo: escapeHtml(tr('To do')),
  };
  const lines = text.replace(/\r\n?/g, '\n').split('\n');
  return blocks(lines, 0, false, labels).join('\n');
}

/** Quotes and list items nest at most this deep; deeper, their lines stay text. */
const MAX_DEPTH = 12;

/** What a backslash escapes. Before anything else it stays, so a path such
 * as C:\Users\me reads as written. */
const ESCAPABLE = '`*_~[]|#';

/** The words the markup carries, in the language shown, escaped. */
interface Labels {
  copy: string;
  copyCode: string;
  table: string;
  done: string;
  todo: string;
}

function escapeHtml(s: string): string {
  return s
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

// ───────────────────────────────────────────────────────────────── Blocks

/** `lines` as block-level HTML. `tight`: the paragraphs of a tight list's
 * item, which go without `<p>`. */
function blocks(lines: readonly string[], depth: number, tight: boolean, labels: Labels): string[] {
  const html: string[] = [];
  let para: string[] = [];
  const flush = (): void => {
    if (!para.length) return;
    const body = inline(escapeHtml(para.join('\n')), true);
    html.push(tight ? body : `<p>${body}</p>`);
    para = [];
  };
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const t = line.trim();
    if (!t) {
      flush();
      i++;
      continue;
    }
    const fence = fenceOpen(line);
    if (fence) {
      flush();
      let end = i + 1;
      while (end < lines.length && !fenceCloses(lines[end], fence)) end++;
      html.push(codeBlock(lines.slice(i + 1, end), fence, labels));
      i = end + 1; // past the closing fence; an unclosed one runs to the end
      continue;
    }
    const h = heading(t);
    if (h) {
      flush();
      const level = Math.min(h.level + 2, 6);
      html.push(`<h${level}>${inline(escapeHtml(h.text), true)}</h${level}>`);
      i++;
      continue;
    }
    if (isRule(t)) {
      flush();
      html.push('<hr>');
      i++;
      continue;
    }
    if (t.startsWith('>')) {
      flush();
      const quoted: string[] = [];
      for (; i < lines.length && lines[i].trimStart().startsWith('>'); i++) {
        quoted.push(unquote(lines[i]));
      }
      html.push(`<blockquote>${nested(quoted, depth, false, labels)}</blockquote>`);
      continue;
    }
    const aligns = tableHead(lines, i);
    if (aligns) {
      flush();
      const table = tableAt(lines, i, aligns, labels);
      html.push(table.html);
      i = table.end;
      continue;
    }
    if (listMarker(line)) {
      flush();
      const list = listAt(lines, i, depth, labels);
      html.push(list.html);
      i = list.end;
      continue;
    }
    para.push(t);
    i++;
  }
  flush();
  return html;
}

/** The blocks of a quote or a list item; past {@link MAX_DEPTH}, its text. */
function nested(lines: readonly string[], depth: number, tight: boolean, labels: Labels): string {
  if (depth < MAX_DEPTH) return blocks(lines, depth + 1, tight, labels).join('\n');
  const body = inline(escapeHtml(lines.map((l) => l.trim()).join('\n')), true);
  return tight ? body : `<p>${body}</p>`;
}

/** Columns of indentation (a tab reaches the next multiple of 4). */
function indentOf(line: string): number {
  let col = 0;
  for (let i = 0; i < line.length; i++) {
    const c = line.charCodeAt(i);
    if (c === 32) col++;
    else if (c === 9) col += 4 - (col % 4);
    else break;
  }
  return col;
}

/** `line` without up to `cols` columns of indentation. */
function dedent(line: string, cols: number): string {
  let col = 0;
  let i = 0;
  while (i < line.length && col < cols) {
    const c = line.charCodeAt(i);
    if (c === 32) col++;
    else if (c === 9) {
      const next = col + 4 - (col % 4);
      if (next > cols) return ' '.repeat(next - cols) + line.slice(i + 1);
      col = next;
    } else break;
    i++;
  }
  return line.slice(i);
}

function heading(t: string): { level: number; text: string } | null {
  let level = 0;
  while (t[level] === '#') level++;
  if (!level || level > 6) return null;
  if (level < t.length && t[level] !== ' ' && t[level] !== '\t') return null;
  let text = t.slice(level).trim();
  // An optional closing run of #s, after a space.
  let end = text.length;
  while (end > 0 && text[end - 1] === '#') end--;
  if (!end) text = '';
  else if (end < text.length && (text[end - 1] === ' ' || text[end - 1] === '\t')) {
    text = text.slice(0, end).trimEnd();
  }
  return { level, text };
}

/** `---`, `***` or `___` (three or more, spaces allowed between). */
function isRule(t: string): boolean {
  const c = t[0];
  if (c !== '-' && c !== '*' && c !== '_') return false;
  let n = 0;
  for (const ch of t) {
    if (ch === c) n++;
    else if (ch !== ' ' && ch !== '\t') return false;
  }
  return n >= 3;
}

/** A quoted line without its `>` and the space after it. */
function unquote(line: string): string {
  const s = line.trimStart().slice(1);
  return s.startsWith(' ') || s.startsWith('\t') ? s.slice(1) : s;
}

/** A line that starts a block of its own, which a paragraph doesn't continue
 * into. */
function startsBlock(lines: readonly string[], i: number): boolean {
  const t = lines[i].trim();
  return (
    !!fenceOpen(lines[i]) || !!heading(t) || isRule(t) || t.startsWith('>') || !!tableHead(lines, i)
  );
}

// ───────────────────────────────────────────────────────────── Fenced code

interface Fence {
  indent: number;
  char: string;
  len: number;
  lang: string;
}

/** A code fence's opening line: three or more ``` or ~~~, then its info
 * string (whose first word is the language). */
function fenceOpen(line: string): Fence | null {
  const t = line.trimStart();
  const char = t[0];
  if (char !== '`' && char !== '~') return null;
  let len = 1;
  while (t[len] === char) len++;
  if (len < 3) return null;
  const info = t.slice(len).trim();
  if (char === '`' && info.includes('`')) return null; // ```a``` is inline code
  return { indent: indentOf(line), char, len, lang: info.split(/\s/, 1)[0] };
}

function fenceCloses(line: string, f: Fence): boolean {
  const t = line.trim();
  if (t.length < f.len) return false;
  for (const c of t) if (c !== f.char) return false;
  return true;
}

/** A code block: its language as a chip, a Copy control (a `span`, as the
 * sanitizer strips buttons), and the code. The fence's indentation comes off
 * every line — the same amount from each, less when a line has less, so the
 * code keeps its shape. */
function codeBlock(body: readonly string[], f: Fence, labels: Labels): string {
  let cut = f.indent;
  for (const l of body) if (l.trim()) cut = Math.min(cut, indentOf(l));
  const code = body.map((l) => dedent(l, cut)).join('\n');
  const lang = f.lang ? `<span class="md-lang">${escapeHtml(f.lang.slice(0, 32))}</span>` : '';
  return (
    `<div class="md-pre${f.lang ? ' has-lang' : ''}"><div class="md-pre-bar">${lang}` +
    `<span class="md-copy" role="button" tabindex="0" aria-label="${labels.copyCode}"` +
    ` title="${labels.copyCode}">${labels.copy}</span></div>` +
    `<pre class="md-code"><code>${escapeHtml(code)}</code></pre></div>`
  );
}

// ────────────────────────────────────────────────────────────────── Tables

type Align = '' | 'left' | 'center' | 'right';

/** A table starts at `lines[i]`: a row with a pipe, then a delimiter row of
 * as many cells (`---`, `:---`, `:---:`, `---:`). Their alignments. */
function tableHead(lines: readonly string[], i: number): Align[] | null {
  const delimiter = lines[i + 1];
  if (delimiter === undefined || !lines[i].includes('|') || !delimiter.includes('|')) return null;
  const cells = splitRow(delimiter);
  const aligns: Align[] = [];
  for (const c of cells) {
    if (!/^:?-+:?$/.test(c)) return null;
    const left = c.startsWith(':');
    const right = c.endsWith(':');
    aligns.push(left && right ? 'center' : right ? 'right' : left ? 'left' : '');
  }
  return splitRow(lines[i]).length === aligns.length ? aligns : null;
}

/** The table at `lines[i]`, through its last row: rows go on until a blank
 * line, a line without a pipe, or another block. Each row has the header's
 * cells: missing ones are empty, extra ones dropped. */
function tableAt(
  lines: readonly string[],
  i: number,
  aligns: readonly Align[],
  labels: Labels,
): { html: string; end: number } {
  const cell = (tag: 'th' | 'td', text: string, k: number): string => {
    const cls = aligns[k] ? ` class="al-${aligns[k]}"` : '';
    return `<${tag}${cls}>${inline(escapeHtml(text), true)}</${tag}>`;
  };
  const head = splitRow(lines[i])
    .map((c, k) => cell('th', c, k))
    .join('');
  const rows: string[] = [];
  let end = i + 2;
  for (; end < lines.length; end++) {
    const l = lines[end];
    if (!l.includes('|') || !l.trim() || startsBlock(lines, end) || listMarker(l)) break;
    const cells = splitRow(l);
    rows.push(`<tr>${aligns.map((_, k) => cell('td', cells[k] ?? '', k)).join('')}</tr>`);
  }
  const body = rows.length ? `<tbody>${rows.join('')}</tbody>` : '';
  return {
    html:
      `<div class="md-table" role="region" tabindex="0" aria-label="${labels.table}">` +
      `<table><thead><tr>${head}</tr></thead>${body}</table></div>`,
    end,
  };
}

/** A row's cells: split at each `|` that is neither escaped nor inside a
 * code span, the outer pipes dropped, `\|` made a pipe, each trimmed. */
function splitRow(line: string): string[] {
  let s = line.trim();
  if (s.startsWith('|')) s = s.slice(1);
  if (s.endsWith('|') && !s.endsWith('\\|')) s = s.slice(0, -1);
  const closer = codeSpanCloser(s);
  const cells: string[] = [];
  let from = 0;
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (c === 92)
      i++; // \| and \` are text
    else if (c === 96) i = codeSpanEnd(s, i, closer) - 1;
    else if (c === 124) {
      cells.push(s.slice(from, i));
      from = i + 1;
    }
  }
  cells.push(s.slice(from));
  return cells.map((c) => c.replaceAll('\\|', '|').trim());
}

// ─────────────────────────────────────────────────────────────────── Lists

interface Marker {
  ordered: boolean;
  start: number;
  /** Columns before the marker, and before the item's text. */
  indent: number;
  content: number;
  /** The text after the marker. */
  rest: string;
}

/** A list item's first line: `-`, `*` or `+`, or 1–9 digits and `.` or
 * `)`, then a space or a tab and some text. */
function listMarker(line: string): Marker | null {
  const s = line.trimStart();
  let k = 0;
  let ordered = false;
  if (s[0] === '-' || s[0] === '*' || s[0] === '+') k = 1;
  else {
    while (k < 9 && s.charCodeAt(k) >= 48 && s.charCodeAt(k) <= 57) k++;
    if (!k || (s[k] !== '.' && s[k] !== ')')) return null;
    ordered = true;
    k++;
  }
  if (s[k] !== ' ' && s[k] !== '\t') return null;
  const rest = s.slice(k).trim();
  if (!rest || isRule(s)) return null;
  const indent = indentOf(line);
  const gap = indentOf(s.slice(k));
  return {
    ordered,
    start: ordered ? Number(s.slice(0, k - 1)) : 1,
    indent,
    content: indent + k + (gap > 4 ? 1 : gap),
    rest,
  };
}

const TASK = /^\[([ xX])\][ \t]+(\S.*)$/;

/** The list at `lines[i]`: its items, while the next one (after blank lines
 * or not) is the same kind of list. Blank lines between or inside items make
 * it loose: each item's paragraphs then get their `<p>`. */
function listAt(
  lines: readonly string[],
  i: number,
  depth: number,
  labels: Labels,
): { html: string; end: number } {
  const first = listMarker(lines[i]);
  if (!first) return { html: '', end: i + 1 };
  const items: { lines: string[]; done: boolean | null }[] = [];
  let loose = false;
  let at = i;
  for (let m: Marker | null = first; m;) {
    const task = TASK.exec(m.rest);
    const item = itemAt(lines, at, m, task ? task[2] : m.rest);
    items.push({ lines: item.lines, done: task ? task[1] !== ' ' : null });
    loose ||= item.blank;
    at = item.end;
    let next = at;
    while (next < lines.length && !lines[next].trim()) next++;
    const sibling = next < lines.length ? listMarker(lines[next]) : null;
    if (sibling?.ordered === first.ordered) {
      loose ||= next > at;
      at = next;
      m = sibling;
    } else m = null;
  }
  const tag = first.ordered ? 'ol' : 'ul';
  const start = first.ordered && first.start !== 1 ? ` start="${first.start}"` : '';
  const lis = items.map((it) => {
    const body = nested(it.lines, depth, !loose, labels);
    if (it.done === null) return `<li>${body}</li>`;
    return (
      `<li class="md-task${it.done ? ' done' : ''}"><span class="md-check" role="img"` +
      ` aria-label="${it.done ? labels.done : labels.todo}"></span>${body}</li>`
    );
  });
  return { html: `<${tag}${start}>${lis.join('')}</${tag}>`, end: at };
}

/**
 * One item from its marker line `lines[i]`: its lines without the item's
 * indentation, through the last that belongs to it — indented right of the
 * marker (a nested list may start anywhere there, 2–4 spaces or a tab), a
 * lazy continuation of its paragraph, or a fenced block it opened (to the
 * closing fence, however indented). `blank`: a blank line inside it.
 */
function itemAt(
  lines: readonly string[],
  i: number,
  m: Marker,
  first: string,
): { lines: string[]; end: number; blank: boolean } {
  const out: string[] = [];
  let blank = false;
  let lazy = false; // the last line was text a less indented line may continue
  let j = i;
  while (j < lines.length) {
    const line = j === i ? ' '.repeat(m.content) + first : lines[j];
    if (j > i && !line.trim()) {
      let k = j + 1;
      while (k < lines.length && !lines[k].trim()) k++;
      const ind = k < lines.length ? indentOf(lines[k]) : -1;
      if (ind >= m.content || (ind > m.indent && listMarker(lines[k]))) {
        for (; j < k; j++) out.push('');
        blank = true;
        lazy = false;
        continue;
      }
      break;
    }
    const ind = indentOf(line);
    let cut: number;
    if (j === i || ind > m.indent) cut = Math.min(ind, m.content);
    else if (lazy && !listMarker(line) && !startsBlock(lines, j)) cut = ind;
    else break;
    const text = dedent(line, cut);
    const fence = fenceOpen(text);
    if (fence) {
      let close = j + 1;
      while (close < lines.length && !fenceCloses(lines[close], fence)) close++;
      const last = Math.min(close, lines.length - 1);
      let common = cut;
      for (let k = j + 1; k <= last; k++) {
        if (lines[k].trim()) common = Math.min(common, indentOf(lines[k]));
      }
      out.push(dedent(line, common));
      for (let k = j + 1; k <= last; k++) out.push(dedent(lines[k], common));
      j = last + 1;
      lazy = false;
      continue;
    }
    out.push(text);
    lazy = !heading(text.trim()) && !isRule(text.trim());
    j++;
  }
  return { lines: out, end: j, blank };
}

// ────────────────────────────────────────────────────────────────── Inline

/** A run of `*`, `_` or `~~` that may open or close emphasis
 * (CommonMark's delimiter run; GitHub's strikethrough for `~~`). */
interface Delim {
  ch: number;
  /** Its place in the output. */
  slot: number;
  /** Characters left, and the run's length. */
  count: number;
  orig: number;
  canOpen: boolean;
  canClose: boolean;
  prev: Delim | null;
  next: Delim | null;
  /** Tags after its characters (opening) and before them (closing). */
  open: string[];
  close: string[];
}

const BR = /^&lt;br\s*\/?&gt;/i;
const ESCAPED_ENTITIES = ['&lt;', '&gt;'];

/**
 * Inline markup on already-escaped text: runs of text are copied as they
 * are, so the only tags in the result are the ones written here. `links`:
 * false inside a link's text, where neither a link nor an image makes an
 * anchor (an image shows its alt text).
 */
function inline(s: string, links: boolean): string {
  const out: string[] = [];
  const delims: Delim[] = [];
  const closer = codeSpanCloser(s);
  const brackets = s.includes('[') ? matchBrackets(s) : null;
  let from = 0; // where the text not yet copied starts
  const put = (at: number, html: string, end: number): number => {
    if (at > from) out.push(s.slice(from, at).replaceAll('\n', '<br>'));
    out.push(html);
    from = end;
    return end;
  };
  let i = 0;
  while (i < s.length) {
    const c = s.charCodeAt(i);
    if (c === 92) {
      // \* and the like; \< and \> keep their character too.
      const next = s[i + 1];
      if (next !== undefined && ESCAPABLE.includes(next)) {
        i = put(i, next, i + 2);
        continue;
      }
      const entity = ESCAPED_ENTITIES.find((e) => s.startsWith(e, i + 1));
      if (entity) {
        i = put(i, entity, i + 1 + entity.length);
        continue;
      }
    } else if (c === 96) {
      let j = i + 1;
      while (s.charCodeAt(j) === 96) j++;
      const end = closer(j, j - i);
      if (end >= 0) {
        i = put(i, `<code>${codeText(s.slice(j, end))}</code>`, end + (j - i));
        continue;
      }
      i = j; // no closing run: the backticks are text
      continue;
    } else if (c === 42 || c === 95 || c === 126) {
      let j = i + 1;
      while (s.charCodeAt(j) === c) j++;
      const d = delimiter(s, i, j, c);
      if (d) {
        put(i, '', j); // its slot, filled in once emphasis is settled
        d.slot = out.length - 1;
        const last = delims[delims.length - 1];
        if (last) {
          last.next = d;
          d.prev = last;
        }
        delims.push(d);
      }
      i = j;
      continue;
    } else if (brackets && (c === 91 || (c === 33 && s.charCodeAt(i + 1) === 91))) {
      const image = c === 33;
      const link = links || image ? linkAt(s, image ? i + 1 : i, brackets) : null;
      if (link) {
        i = put(i, linkHtml(link.label, link.dest, image, links), link.end);
        continue;
      }
    } else if (c === 38) {
      const br = BR.exec(s.slice(i, i + 16));
      if (br) {
        i = put(i, '<br>', i + br[0].length);
        continue;
      }
      if (links && s.startsWith('&lt;http', i)) {
        const end = angleUrlEnd(s, i + 4);
        if (end > 0) {
          const url = s.slice(i + 4, end);
          i = put(i, anchor(url, url), end + 4);
          continue;
        }
      }
    } else if (
      c === 104 &&
      links &&
      (s.startsWith('http://', i) || s.startsWith('https://', i)) &&
      !/[\p{L}\p{N}]/u.test(charBefore(s, i))
    ) {
      const end = urlEnd(s, i);
      if (end > 0) {
        const url = s.slice(i, end);
        i = put(i, anchor(url, url), end);
        continue;
      }
    }
    i++;
  }
  if (from < s.length) out.push(s.slice(from).replaceAll('\n', '<br>'));
  emphasis(delims[0] ?? null);
  for (const d of delims) {
    out[d.slot] = d.close.join('') + String.fromCharCode(d.ch).repeat(d.count) + d.open.join('');
  }
  return out.join('');
}

/** A code span's text: line breaks become spaces, and one space comes off
 * each end when both have one (``` `` `a` `` ``` shows `a`). */
function codeText(c: string): string {
  const t = c.replaceAll('\n', ' ');
  return t.length > 2 && t.startsWith(' ') && t.endsWith(' ') && t.trim() ? t.slice(1, -1) : t;
}

/**
 * Finds code spans' closing runs: `closer(from, n)` is where the first run
 * of exactly `n` backticks at or after `from` starts, or -1. A run is
 * paired with the next run of the same length; asked left to right, each
 * length's cursor only moves forward, so all lookups take linear time.
 */
function codeSpanCloser(s: string): (from: number, n: number) => number {
  const runs = new Map<number, number[]>();
  for (let i = s.indexOf('`'); i >= 0;) {
    let j = i + 1;
    while (s.charCodeAt(j) === 96) j++;
    const list = runs.get(j - i);
    if (list) list.push(i);
    else runs.set(j - i, [i]);
    i = s.indexOf('`', j);
  }
  const cursor = new Map<number, number>();
  return (from, n) => {
    const list = runs.get(n);
    if (!list) return -1;
    let k = cursor.get(n) ?? 0;
    while (k < list.length && list[k] < from) k++;
    cursor.set(n, k);
    return k < list.length ? list[k] : -1;
  };
}

/** Past the code span (or the lone backtick run) that starts at `s[i]`. */
function codeSpanEnd(s: string, i: number, closer: (from: number, n: number) => number): number {
  let j = i + 1;
  while (s.charCodeAt(j) === 96) j++;
  const end = closer(j, j - i);
  return end < 0 ? j : end + (j - i);
}

/** Each `[` with a matching `]` (balanced; escaped ones and those in code
 * spans don't count) → that `]`. */
function matchBrackets(s: string): Map<number, number> {
  const pairs = new Map<number, number>();
  const open: number[] = [];
  const closer = codeSpanCloser(s);
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (c === 92) i++;
    else if (c === 96) i = codeSpanEnd(s, i, closer) - 1;
    else if (c === 91) open.push(i);
    else if (c === 93) {
      const o = open.pop();
      if (o !== undefined) pairs.set(o, i);
    }
  }
  return pairs;
}

/** `[label](destination "title")` with its `[` at `s[i]`. */
function linkAt(
  s: string,
  i: number,
  brackets: ReadonlyMap<number, number>,
): { label: string; dest: string; end: number } | null {
  const close = brackets.get(i);
  if (close === undefined || s[close + 1] !== '(') return null;
  let j = close + 2;
  while (s[j] === ' ' || s[j] === '\t') j++;
  let dest: string;
  if (s.startsWith('&lt;', j)) {
    const end = angleUrlEnd(s, j + 4);
    if (end < 0) return null;
    dest = s.slice(j + 4, end);
    j = end + 4;
  } else {
    // Up to a space or the `)` that closes it; parentheses inside balance.
    // Never past a `[` (no URL needs one), which keeps every scan short.
    const start = j;
    let depth = 0;
    for (; j < s.length; j++) {
      const c = s[j];
      if (c === ' ' || c === '\t' || c === '\n' || c === '[' || c === '`') break;
      if (c === '(') depth++;
      else if (c === ')') {
        if (!depth) break;
        depth--;
      }
    }
    if (depth) return null;
    dest = s.slice(start, j);
  }
  // An optional title, in quotes or parentheses.
  let k = j;
  while (s[k] === ' ' || s[k] === '\t' || s[k] === '\n') k++;
  if (k > j) {
    for (const [o, c] of [
      ['&quot;', '&quot;'],
      ['&#39;', '&#39;'],
      ['(', ')'],
    ]) {
      if (!s.startsWith(o, k)) continue;
      const end = s.slice(k + o.length, k + o.length + 500).indexOf(c);
      if (end < 0) return null;
      k += o.length + end + c.length;
      while (s[k] === ' ' || s[k] === '\t' || s[k] === '\n') k++;
      break;
    }
  }
  if (s[k] !== ')') return null;
  return { label: s.slice(i + 1, close), dest, end: k + 1 };
}

/** A link: an anchor for an http(s) URL; any other target shows its text
 * alone (a relative path keeps itself as a tooltip). An image is a link to
 * it, showing its alt text: it is never loaded. Inside a link's text, only
 * the text. */
function linkHtml(label: string, dest: string, image: boolean, links: boolean): string {
  const text = inline(label, false);
  if (!links) return text;
  if (isWebUrl(dest)) return anchor(dest, text || dest, image ? 'md-img' : '');
  if (!text) return '';
  const title = /^[a-z][a-z0-9+.-]+:/i.test(dest) || dest.length > 300 ? '' : ` title="${dest}"`;
  return `<span class="md-link"${title}>${text}</span>`;
}

function anchor(url: string, text: string, cls = ''): string {
  const c = cls ? ` class="${cls}"` : '';
  return `<a${c} href="${url}" target="_blank" rel="noopener noreferrer">${text}</a>`;
}

/** An http(s) URL with something after the scheme and no white space. */
function isWebUrl(url: string): boolean {
  const scheme = /^https?:\/\//i.exec(url);
  return !!scheme && url.length > scheme[0].length && !/\s/.test(url);
}

/** Where `&lt;http…&gt;`'s URL (starting at `s[i]`) ends: the `&gt;`, or
 * -1 when white space or another `&lt;` comes first. */
function angleUrlEnd(s: string, i: number): number {
  for (let j = i; j < s.length; j++) {
    const c = s[j];
    if (/\s/.test(c)) return -1;
    if (c === '&') {
      if (s.startsWith('&gt;', j)) return isWebUrl(s.slice(i, j)) ? j : -1;
      if (s.startsWith('&lt;', j)) return -1;
    }
  }
  return -1;
}

/** Where a bare URL starting at `s[i]` ends (GitHub's rules): at white
 * space, a backtick, `<`, `>` or `"`, less trailing punctuation, a quote,
 * and a `)` or `]` its URL doesn't open. -1: not a URL after all. */
function urlEnd(s: string, i: number): number {
  let e = i;
  let parens = 0;
  let squares = 0;
  for (; e < s.length; e++) {
    const c = s[e];
    if (c === '`' || /\s/.test(c)) break;
    if (
      c === '&' &&
      (s.startsWith('&lt;', e) || s.startsWith('&gt;', e) || s.startsWith('&quot;', e))
    ) {
      break;
    }
    if (c === '(') parens++;
    else if (c === ')') parens--;
    else if (c === '[') squares++;
    else if (c === ']') squares--;
  }
  for (;;) {
    if (s.startsWith('&amp;', e - 5)) break; // a & of its own: an entity's ; is not punctuation
    if (s.startsWith('&#39;', e - 5)) {
      e -= 5;
      continue;
    }
    const c = s[e - 1];
    if ('?!.,:;*_~'.includes(c)) e--;
    else if (c === ')' && parens < 0) {
      parens++;
      e--;
    } else if (c === ']' && squares < 0) {
      squares++;
      e--;
    } else break;
  }
  const host = s.indexOf('//', i) + 2;
  return e > host && /[\p{L}\p{N}[]/u.test(s[host]) ? e : -1;
}

/** The character before `s[i]` (a whole surrogate pair), '' at the start. */
function charBefore(s: string, i: number): string {
  if (i <= 0) return '';
  const lo = s.charCodeAt(i - 1);
  if (lo >= 0xdc00 && lo <= 0xdfff && i >= 2) {
    const hi = s.charCodeAt(i - 2);
    if (hi >= 0xd800 && hi <= 0xdbff) return s.slice(i - 2, i);
  }
  return s[i - 1];
}

/** The character at `s[i]` (a whole surrogate pair), '' at the end. */
function charAt(s: string, i: number): string {
  const cp = s.codePointAt(i);
  return cp === undefined ? '' : String.fromCodePoint(cp);
}

const isSpace = (c: string): boolean => !c || /\s/.test(c);
const isPunct = (c: string): boolean => /[\p{P}\p{S}]/u.test(c);

/** The delimiter run `s[i, j)` of `c`, when it can open or close: `*` by
 * CommonMark's flanking rules, `_` only at a word's boundary (so snake_case
 * stays as it is), `~` only as a pair. */
function delimiter(s: string, i: number, j: number, c: number): Delim | null {
  const n = j - i;
  if (c === 126 && n !== 2) return null;
  const before = charBefore(s, i);
  const after = charAt(s, j);
  const left = !isSpace(after) && (!isPunct(after) || isSpace(before) || isPunct(before));
  const right = !isSpace(before) && (!isPunct(before) || isSpace(after) || isPunct(after));
  const canOpen = c === 95 ? left && (!right || isPunct(before)) : left;
  const canClose = c === 95 ? right && (!left || isPunct(after)) : right;
  if (!canOpen && !canClose) return null;
  return {
    ch: c,
    slot: 0,
    count: n,
    orig: n,
    canOpen,
    canClose,
    prev: null,
    next: null,
    open: [],
    close: [],
  };
}

/** Pairs openers with closers, CommonMark's "process emphasis": each closer
 * takes the nearest opener of its kind below it; two characters each make
 * `<strong>`, one `<em>`, `~~` `<del>`. Delimiters between a pair can no
 * longer pair. A closer that found nothing marks how far down the next one
 * like it needs to look, which keeps this linear. */
function emphasis(head: Delim | null): void {
  const bottom = new Map<string, Delim | null>();
  let closer = head;
  while (closer) {
    if (!closer.canClose) {
      closer = closer.next;
      continue;
    }
    const key = `${closer.ch}:${closer.canOpen}:${closer.orig % 3}`;
    const floor = bottom.get(key);
    let opener = closer.prev;
    while (opener && opener !== floor && !pairs(opener, closer)) opener = opener.prev;
    if (!opener || opener === floor) {
      bottom.set(key, closer.prev);
      const next: Delim | null = closer.next;
      if (!closer.canOpen) unlink(closer);
      closer = next;
      continue;
    }
    const strike = closer.ch === 126;
    const use = strike || (opener.count >= 2 && closer.count >= 2) ? 2 : 1;
    const tag = strike ? 'del' : use === 2 ? 'strong' : 'em';
    opener.count -= use;
    closer.count -= use;
    opener.open.unshift(`<${tag}>`);
    closer.close.push(`</${tag}>`);
    opener.next = closer;
    closer.prev = opener;
    if (!opener.count) unlink(opener);
    if (!closer.count) {
      const next: Delim | null = closer.next;
      unlink(closer);
      closer = next;
    }
  }
}

/** `opener` may pair with `closer`: same character, and for `*` and `_`
 * CommonMark's rule of three (a run that can both open and close pairs only
 * when the two runs' lengths don't add up to a multiple of 3, unless both
 * are one). */
function pairs(opener: Delim, closer: Delim): boolean {
  if (opener.ch !== closer.ch || !opener.canOpen || !opener.count) return false;
  if (closer.ch === 126) return opener.count === closer.count;
  if (!(closer.canOpen || opener.canClose)) return true;
  return closer.orig % 3 === 0 || (opener.orig + closer.orig) % 3 !== 0;
}

function unlink(d: Delim): void {
  if (d.prev) d.prev.next = d.next;
  if (d.next) d.next.prev = d.prev;
}
