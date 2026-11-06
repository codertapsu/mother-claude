// `read`: the page as text, Markdown (main content by default) or HTML, with
// the same redaction as snapshots.

import {
  collapse,
  deepQueryAll,
  frameDocument,
  inputType,
  isBlock,
  isElement,
  isIframe,
  isInput,
  isRendered,
  isSelect,
  isText,
  renderedChildren,
  tag,
  truncate,
  visibilityOf,
} from './dom.ts';
import { REDACTED, isSecretField, soundsSecret } from './redact.ts';
import { SelectorNotFound } from './snapshot.ts';

export type ReadMode = 'text' | 'markdown' | 'html';

export interface ReadParams {
  mode?: ReadMode;
  selector?: string;
  maxChars?: number;
}

export interface ReadResult {
  content: string;
  truncated: boolean;
}

export const DEFAULT_MAX_CHARS = 40_000;

export function read(doc: Document, p: ReadParams): ReadResult {
  const mode: ReadMode = p.mode === 'html' || p.mode === 'markdown' ? p.mode : 'text';
  const max =
    typeof p.maxChars === 'number' && Number.isFinite(p.maxChars)
      ? Math.max(1, Math.min(5_000_000, Math.floor(p.maxChars)))
      : DEFAULT_MAX_CHARS;
  let root: Element | null;
  if (p.selector) {
    root = deepQueryAll(doc, p.selector, 1)[0] ?? null;
    if (!root) throw new SelectorNotFound(p.selector);
  } else if (mode === 'markdown') {
    root = mainContent(doc);
  } else {
    root = doc.body ?? doc.documentElement;
  }
  if (!root) return { content: '', truncated: false };
  let content: string;
  if (mode === 'html') content = sanitizedHtml(root);
  else if (mode === 'markdown') content = toMarkdown(root, { chrome: !!p.selector });
  else content = visibleText(root);
  if (content.length > max) return { content: truncate(content, max), truncated: true };
  return { content, truncated: false };
}

// ------------------------------------------------------------------- text

function tidy(raw: string): string {
  return raw
    .split('\n')
    .map((line) => line.replace(/[ \t\u00a0]+/g, ' ').trim())
    .join('\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

/** Visible text, laid out roughly as innerText would (blocks on their own lines). */
export function visibleText(root: Element): string {
  const parts: string[] = [];
  const walk = (node: Node, redacted: boolean, invisible: boolean): void => {
    for (const child of renderedChildren(node)) {
      if (isText(child)) {
        if (invisible) continue;
        if (redacted) {
          if (parts[parts.length - 1] !== REDACTED) parts.push(REDACTED);
        } else parts.push(child.data);
        continue;
      }
      if (!isElement(child)) continue;
      const vis = visibilityOf(child, { ariaHidden: false });
      if (vis === 'hidden') continue;
      const t = tag(child);
      if (t === 'br') {
        parts.push('\n');
        continue;
      }
      if (isIframe(child)) {
        const inner = frameDocument(child);
        const title = child.getAttribute('title')?.trim();
        if (inner?.body) {
          parts.push('\n');
          walk(inner.body, redacted, false);
          parts.push('\n');
        } else {
          parts.push(`\n[iframe${title ? ` "${title}"` : ''}: cross-origin, not read]\n`);
        }
        continue;
      }
      if (t === 'input' || t === 'select' || t === 'textarea' || t === 'img' || t === 'svg')
        continue;
      const block = isBlock(child);
      if (block) parts.push('\n');
      const red = redacted || child.hasAttribute('data-mc-redact');
      if (red && !redacted) parts.push(REDACTED);
      else walk(child, red, vis === 'invisible');
      if (t === 'td' || t === 'th') parts.push('\t');
      if (block) parts.push('\n');
    }
  };
  if (root.hasAttribute('data-mc-redact')) return REDACTED;
  walk(root, false, false);
  return tidy(parts.join(''));
}

// --------------------------------------------------------------- markdown

const POSITIVE = /article|body|content|entry|main|page|post|text|blog|story|prose|markdown/i;
const NEGATIVE =
  /comment|footer|footnote|masthead|media|meta|outbrain|promo|related|scroll|shoutbox|sidebar|sponsor|shopping|tags|tool|widget|nav|menu|share|social|banner|cookie|consent|subscribe|newsletter|breadcrumb/i;

function classWeight(el: Element): number {
  const s = `${el.getAttribute('class') ?? ''} ${el.id}`;
  let w = 0;
  if (POSITIVE.test(s)) w += 25;
  if (NEGATIVE.test(s)) w -= 25;
  return w;
}

function textLength(el: Element): number {
  return collapse(el.textContent ?? '').trim().length;
}

function linkDensity(el: Element): number {
  const total = textLength(el);
  if (!total) return 1;
  let links = 0;
  for (const a of Array.from(el.querySelectorAll('a'))) links += textLength(a);
  return Math.min(1, links / total);
}

/** The element that holds the page's main content (readability-style). */
export function mainContent(doc: Document): Element | null {
  const body = doc.body;
  if (!body) return doc.documentElement;
  const main = doc.querySelector('main, [role="main"]');
  if (main && isRendered(main) && textLength(main) >= 200) return main;
  const articles = Array.from(doc.querySelectorAll('article')).filter((a) => isRendered(a));
  const only = articles.length === 1 ? articles[0] : undefined;
  if (only && textLength(only) >= 200) return only;

  const scores = new Map<Element, number>();
  const add = (el: Element, points: number): void => {
    const base = scores.has(el) ? 0 : classWeight(el);
    scores.set(el, (scores.get(el) ?? 0) + base + points);
  };
  for (const p of Array.from(body.querySelectorAll('p, pre, td, blockquote'))) {
    const text = collapse(p.textContent ?? '').trim();
    if (text.length < 25) continue;
    const points = 1 + Math.min(3, Math.floor(text.length / 100)) + (text.split(',').length - 1);
    const parent = p.parentElement;
    if (parent) add(parent, points);
    const grand = parent?.parentElement;
    if (grand) add(grand, points / 2);
  }
  let best: Element | null = null;
  let bestScore = 0;
  for (const [el, score] of scores) {
    const adjusted = score * (1 - linkDensity(el));
    if (adjusted > bestScore) {
      best = el;
      bestScore = adjusted;
    }
  }
  return best && bestScore >= 5 ? best : body;
}

interface MdCtx {
  /** Include navigation, forms and other page chrome (an explicit selector). */
  chrome: boolean;
  pre: boolean;
  redacted: boolean;
}

const CHROME_TAGS = new Set(['nav', 'aside', 'footer', 'form', 'dialog', 'header', 'menu']);
const SKIP_TAGS = new Set([
  'button',
  'input',
  'select',
  'textarea',
  'svg',
  'canvas',
  'iframe',
  'frame',
  'video',
  'audio',
  'object',
  'embed',
  'template',
  'script',
  'style',
  'noscript',
  'datalist',
]);

function absoluteUrl(el: Element, raw: string | null): string | null {
  if (!raw) return null;
  try {
    const u = new URL(raw, el.baseURI);
    if (u.protocol === 'javascript:' || u.protocol === 'data:') return null;
    return u.href;
  } catch {
    return null;
  }
}

function block(s: string): string {
  const t = s.trim();
  return t ? `\n\n${t}\n\n` : '';
}

function wrapInline(s: string, mark: string): string {
  const t = s.trim();
  if (!t) return s;
  const lead = s.startsWith(' ') ? ' ' : '';
  const trail = s.endsWith(' ') ? ' ' : '';
  return `${lead}${mark}${t}${mark}${trail}`;
}

function mdChildren(el: Node, ctx: MdCtx): string {
  let out = '';
  for (const child of renderedChildren(el)) out += mdNode(child, ctx);
  return out;
}

function mdNode(node: Node, ctx: MdCtx): string {
  if (isText(node)) {
    if (ctx.redacted) return REDACTED;
    return ctx.pre ? node.data : collapse(node.data);
  }
  if (!isElement(node)) return '';
  const el = node;
  const vis = visibilityOf(el, { ariaHidden: false });
  if (vis === 'hidden') return '';
  const t = tag(el);
  if (SKIP_TAGS.has(t)) return '';
  if (!ctx.chrome && CHROME_TAGS.has(t)) return '';
  if (!ctx.chrome && t === 'header' && el.closest('article')) return '';
  if (el.hasAttribute('data-mc-redact') && !ctx.redacted) {
    return isBlock(el) ? block(REDACTED) : REDACTED;
  }
  if (vis === 'invisible') return mdChildren(el, ctx);

  switch (t) {
    case 'h1':
    case 'h2':
    case 'h3':
    case 'h4':
    case 'h5':
    case 'h6': {
      const text = collapse(mdChildren(el, ctx)).trim();
      return text ? block(`${'#'.repeat(Number(t.charAt(1)))} ${text}`) : '';
    }
    case 'br':
      return '\n';
    case 'hr':
      return block('---');
    case 'a': {
      const text = mdChildren(el, ctx);
      const href = el.getAttribute('href') ?? '';
      if (!text.trim()) return '';
      if (href.startsWith('#')) return text;
      const url = absoluteUrl(el, href);
      return url ? `[${collapse(text).trim()}](${url})` : text;
    }
    case 'img': {
      const alt = collapse(el.getAttribute('alt') ?? '').trim();
      if (!alt) return '';
      const src = absoluteUrl(el, el.getAttribute('src'));
      return src ? `![${alt}](${src})` : `![${alt}]`;
    }
    case 'strong':
    case 'b':
      return wrapInline(mdChildren(el, ctx), '**');
    case 'em':
    case 'i':
      return wrapInline(mdChildren(el, ctx), '_');
    case 'del':
    case 's':
      return wrapInline(mdChildren(el, ctx), '~~');
    case 'code':
    case 'kbd':
    case 'samp': {
      if (ctx.pre) return mdChildren(el, ctx);
      const text = el.textContent ?? '';
      if (!text.trim()) return '';
      const fence = text.includes('`') ? '``' : '`';
      return `${fence}${collapse(text)}${fence}`;
    }
    case 'pre': {
      const code = el.querySelector('code');
      const lang = /language-([\w+-]+)/.exec(code?.className ?? el.className)?.[1] ?? '';
      const text = (ctx.redacted ? REDACTED : (el.textContent ?? '')).replace(/\n+$/, '');
      return `\n\n\`\`\`${lang}\n${text}\n\`\`\`\n\n`;
    }
    case 'blockquote': {
      const inner = normalize(mdChildren(el, ctx));
      return block(
        inner
          .split('\n')
          .map((l) => (l ? `> ${l}` : '>'))
          .join('\n'),
      );
    }
    case 'ul':
    case 'ol':
      return list(el, ctx, t === 'ol');
    case 'li':
      return block(mdChildren(el, ctx));
    case 'table':
      return table(el, ctx);
    case 'dt':
      return `\n\n**${collapse(mdChildren(el, ctx)).trim()}**\n`;
    case 'dd':
      return `\n: ${collapse(mdChildren(el, ctx)).trim()}\n`;
    case 'details': {
      const summary = Array.from(el.children).find((c) => tag(c) === 'summary');
      const title = summary ? collapse(mdChildren(summary, ctx)).trim() : '';
      if (!(el as HTMLDetailsElement).open) return title ? block(`**${title}**`) : '';
      let rest = '';
      for (const c of renderedChildren(el)) if (c !== summary) rest += mdNode(c, ctx);
      return block(`${title ? `**${title}**\n\n` : ''}${normalize(rest)}`);
    }
    case 'summary':
      return block(`**${collapse(mdChildren(el, ctx)).trim()}**`);
    default: {
      const inner = mdChildren(el, ctx);
      return isBlock(el) ? block(inner) : inner;
    }
  }
}

function normalize(md: string): string {
  return md
    .split('\n')
    .map((l) => l.replace(/[ \t]+$/g, ''))
    .join('\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

function list(el: Element, ctx: MdCtx, ordered: boolean): string {
  let n = Number.parseInt(el.getAttribute('start') ?? '1', 10);
  if (Number.isNaN(n)) n = 1;
  const items: string[] = [];
  for (const li of Array.from(el.children)) {
    if (tag(li) !== 'li' || visibilityOf(li, { ariaHidden: false }) === 'hidden') continue;
    const prefix = ordered ? `${n++}. ` : '- ';
    const body = normalize(mdChildren(li, ctx)).replace(/\n{2,}/g, '\n');
    if (!body) continue;
    const pad = ' '.repeat(prefix.length);
    items.push(
      body
        .split('\n')
        .map((line, i) => (i === 0 ? prefix + line : pad + line))
        .join('\n'),
    );
  }
  return items.length ? `\n\n${items.join('\n')}\n\n` : '';
}

function cellText(cell: Element, ctx: MdCtx): string {
  return collapse(mdChildren(cell, ctx)).replace(/\|/g, '\\|').trim();
}

function table(el: Element, ctx: MdCtx): string {
  const rows = Array.from(el.querySelectorAll('tr')).filter((tr) => tr.closest('table') === el);
  const matrix = rows
    .map((tr) =>
      Array.from(tr.children)
        .filter((c) => tag(c) === 'td' || tag(c) === 'th')
        .map((c) => cellText(c, ctx)),
    )
    .filter((r) => r.length > 0);
  const width = Math.max(0, ...matrix.map((r) => r.length));
  if (width <= 1 || el.querySelector('table')) {
    // A layout table: its cells are just blocks.
    return block(mdChildren(el, ctx));
  }
  const pad = (r: string[]): string[] => [...r, ...Array<string>(width - r.length).fill('')];
  const [head = [], ...rest] = matrix;
  const lines = [
    `| ${pad(head).join(' | ')} |`,
    `| ${Array<string>(width).fill('---').join(' | ')} |`,
    ...rest.map((r) => `| ${pad(r).join(' | ')} |`),
  ];
  const caption = el.querySelector('caption');
  const title = caption ? `**${collapse(caption.textContent ?? '').trim()}**\n\n` : '';
  return block(title + lines.join('\n'));
}

/** Markdown of an element; page chrome (nav, forms…) left out unless asked. */
export function toMarkdown(root: Element, opts: { chrome: boolean }): string {
  const ctx: MdCtx = { chrome: opts.chrome, pre: false, redacted: false };
  const title = root.ownerDocument?.title?.trim();
  const body = normalize(mdNode(root, ctx));
  const isWholePage = !opts.chrome;
  if (isWholePage && title && !body.startsWith('# ')) return normalize(`# ${title}\n\n${body}`);
  return body;
}

// ------------------------------------------------------------------- html

/** The element's HTML with scripts removed and every secret value redacted. */
export function sanitizedHtml(root: Element): string {
  const clone = root.cloneNode(true) as Element;
  const originals = [root, ...Array.from(root.querySelectorAll('*'))];
  const copies = [clone, ...Array.from(clone.querySelectorAll('*'))];
  if (originals.length === copies.length) {
    for (let i = 0; i < originals.length; i++) {
      const o = originals[i];
      const c = copies[i];
      if (!o || !c) continue;
      redactCopy(o, c);
    }
  }
  for (const s of Array.from(clone.querySelectorAll('script, style, noscript, template')))
    s.remove();
  for (const el of Array.from(clone.querySelectorAll('[data-mc-redact]'))) {
    el.replaceChildren(REDACTED);
    for (const attr of Array.from(el.attributes)) {
      if (attr.name !== 'data-mc-redact' && attr.name !== 'class' && attr.name !== 'id')
        el.removeAttribute(attr.name);
    }
  }
  return clone.outerHTML;
}

function redactCopy(o: Element, c: Element): void {
  const t = tag(o);
  if (t === 'meta') {
    if (soundsSecret(o.getAttribute('name')) || soundsSecret(o.getAttribute('property')))
      c.setAttribute('content', REDACTED);
    return;
  }
  if (isInput(o)) {
    const type = inputType(o);
    if (isSecretField(o)) {
      c.setAttribute('value', REDACTED);
      return;
    }
    if (type === 'checkbox' || type === 'radio') {
      if (o.checked) c.setAttribute('checked', '');
      else c.removeAttribute('checked');
      return;
    }
    if (type !== 'file') c.setAttribute('value', o.value);
    return;
  }
  if (t === 'textarea') {
    c.textContent = isSecretField(o) ? REDACTED : (o as HTMLTextAreaElement).value;
    return;
  }
  if (isSelect(o)) {
    const secret = isSecretField(o);
    const opts = Array.from(c.querySelectorAll('option'));
    Array.from(o.options).forEach((opt, i) => {
      const copy = opts[i];
      if (!copy) return;
      if (secret) {
        copy.textContent = REDACTED;
        copy.setAttribute('value', REDACTED);
      }
      if (opt.selected) copy.setAttribute('selected', '');
      else copy.removeAttribute('selected');
    });
  }
}
