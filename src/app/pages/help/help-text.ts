/** One run of a help string: plain text, `**bold**` or `` `code` ``. */
export interface InlineRun {
  kind: 'text' | 'strong' | 'code';
  text: string;
}

const INLINE = /\*\*(.+?)\*\*|`([^`]+)`/g;

/**
 * Splits the `**bold**` and `` `code` `` marks out of a help string, so the
 * template renders them as elements with interpolated text — help content is
 * never bound as HTML.
 */
export function inlineRuns(source: string): InlineRun[] {
  const runs: InlineRun[] = [];
  let last = 0;
  for (const m of source.matchAll(INLINE)) {
    const at = m.index ?? 0;
    if (at > last) runs.push({ kind: 'text', text: source.slice(last, at) });
    runs.push(m[1] !== undefined ? { kind: 'strong', text: m[1] } : { kind: 'code', text: m[2] });
    last = at + m[0].length;
  }
  if (last < source.length) runs.push({ kind: 'text', text: source.slice(last) });
  return runs;
}

/** The string without its marks (search, ids). */
export function plainText(source: string): string {
  return source.replace(INLINE, (_m, bold: string | undefined, code: string | undefined) =>
    String(bold ?? code ?? ''),
  );
}

/** A heading as a fragment id: "Pull a backup set" → "pull-a-backup-set". */
export function slugify(text: string): string {
  const slug = plainText(text)
    .toLowerCase()
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return slug || 'section';
}

/** The words of a search, lower-cased; every one has to match. */
export function searchTerms(query: string): string[] {
  return query.toLowerCase().split(/\s+/).filter(Boolean);
}
