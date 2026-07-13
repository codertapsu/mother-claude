/**
 * Minimal, safe markdown → HTML for transcript prose.
 *
 * Escape-first: ALL input is HTML-escaped before any markdown markup is
 * translated, so transcript content can never inject tags. The result is bound
 * via [innerHTML], where Angular's built-in sanitizer provides a second layer.
 *
 * Supported (what Claude's answers actually use): headings, paragraphs,
 * fenced/inline code, bold, italics, links, bullet & numbered lists,
 * blockquotes, and horizontal rules. Everything else renders as plain text.
 */

function escapeHtml(s: string): string {
  return s
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

/** Inline markup on an already-escaped line: code, bold, italics, links. */
function inline(escaped: string): string {
  let out = escaped;
  // `code` first so its contents are exempt from bold/italic/link parsing.
  const codes: string[] = [];
  out = out.replace(/`([^`]+)`/g, (_m, c: string) => {
    codes.push(`<code>${c}</code>`);
    return `\uE000${codes.length - 1}\uE000`;
  });
  out = out.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  out = out.replace(/(^|[^*\w])\*([^*\s][^*]*)\*/g, '$1<em>$2</em>');
  // [text](http…): only http/https URLs become links; anything else stays text.
  out = out.replace(
    /\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g,
    '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>',
  );
  return out.replace(/\uE000(\d+)\uE000/g, (_m, i: string) => codes[Number(i)] ?? '');
}

export function renderMarkdown(text: string): string {
  const lines = text.replaceAll('\r\n', '\n').split('\n');
  const html: string[] = [];
  let paragraph: string[] = [];
  let list: 'ul' | 'ol' | null = null;
  let fence: string[] | null = null;

  const flushParagraph = (): void => {
    if (paragraph.length) {
      html.push(`<p>${paragraph.map((l) => inline(escapeHtml(l))).join('<br>')}</p>`);
      paragraph = [];
    }
  };
  const closeList = (): void => {
    if (list) {
      html.push(`</${list}>`);
      list = null;
    }
  };

  for (const raw of lines) {
    if (fence) {
      if (raw.trimEnd().startsWith('```')) {
        html.push(`<pre class="md-code"><code>${escapeHtml(fence.join('\n'))}</code></pre>`);
        fence = null;
      } else {
        fence.push(raw);
      }
      continue;
    }
    const line = raw.trimEnd();
    const trimmed = line.trim();

    if (trimmed.startsWith('```')) {
      flushParagraph();
      closeList();
      fence = [];
      continue;
    }
    if (!trimmed) {
      flushParagraph();
      closeList();
      continue;
    }
    const heading = /^(#{1,6})\s+(.*)$/.exec(trimmed);
    if (heading) {
      flushParagraph();
      closeList();
      const level = Math.min(heading[1].length + 2, 6); // #→h3 … so cards keep hierarchy
      html.push(`<h${level}>${inline(escapeHtml(heading[2]))}</h${level}>`);
      continue;
    }
    if (/^(-{3,}|\*{3,}|_{3,})$/.test(trimmed)) {
      flushParagraph();
      closeList();
      html.push('<hr>');
      continue;
    }
    const bullet = /^[-*+]\s+(.*)$/.exec(trimmed);
    const numbered = /^\d+[.)]\s+(.*)$/.exec(trimmed);
    if (bullet || numbered) {
      flushParagraph();
      const want: 'ul' | 'ol' = bullet ? 'ul' : 'ol';
      if (list !== want) {
        closeList();
        html.push(`<${want}>`);
        list = want;
      }
      html.push(`<li>${inline(escapeHtml((bullet ?? numbered)![1]))}</li>`);
      continue;
    }
    if (trimmed.startsWith('>')) {
      flushParagraph();
      closeList();
      html.push(`<blockquote>${inline(escapeHtml(trimmed.replace(/^>\s?/, '')))}</blockquote>`);
      continue;
    }
    paragraph.push(line);
  }
  if (fence) {
    html.push(`<pre class="md-code"><code>${escapeHtml(fence.join('\n'))}</code></pre>`);
  }
  flushParagraph();
  closeList();
  return html.join('\n');
}
