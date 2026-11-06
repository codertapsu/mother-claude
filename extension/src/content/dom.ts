// Realm-safe DOM helpers. Elements of a same-origin iframe belong to another
// realm, so nothing here uses `instanceof` with this window's constructors.

export const ELEMENT_NODE = 1;
export const TEXT_NODE = 3;
export const DOCUMENT_NODE = 9;
export const DOCUMENT_FRAGMENT_NODE = 11;
const HTML_NS = 'http://www.w3.org/1999/xhtml';
const SVG_NS = 'http://www.w3.org/2000/svg';

export function isElement(n: unknown): n is Element {
  return !!n && (n as Node).nodeType === ELEMENT_NODE;
}
export function isText(n: unknown): n is Text {
  return !!n && (n as Node).nodeType === TEXT_NODE;
}
export function isHtml(el: Element): el is HTMLElement {
  return el.namespaceURI === HTML_NS;
}
export function isSvg(el: Element): el is SVGElement {
  return el.namespaceURI === SVG_NS;
}
export function tag(el: Element): string {
  return isHtml(el) ? el.localName : el.localName.toLowerCase();
}

export function isInput(el: Element): el is HTMLInputElement {
  return isHtml(el) && el.localName === 'input';
}
export function isTextArea(el: Element): el is HTMLTextAreaElement {
  return isHtml(el) && el.localName === 'textarea';
}
export function isSelect(el: Element): el is HTMLSelectElement {
  return isHtml(el) && el.localName === 'select';
}
export function isOption(el: Element): el is HTMLOptionElement {
  return isHtml(el) && el.localName === 'option';
}
export function isIframe(el: Element): el is HTMLIFrameElement {
  return isHtml(el) && (el.localName === 'iframe' || el.localName === 'frame');
}
export function isAnchor(el: Element): el is HTMLAnchorElement {
  return isHtml(el) && el.localName === 'a';
}
export function isForm(el: Element): el is HTMLFormElement {
  return isHtml(el) && el.localName === 'form';
}

/** Input types that take typed text. */
const TEXT_INPUT_TYPES = new Set([
  '',
  'text',
  'search',
  'email',
  'tel',
  'url',
  'password',
  'number',
  'date',
  'datetime-local',
  'month',
  'time',
  'week',
]);

export function inputType(el: HTMLInputElement): string {
  return (el.getAttribute('type') ?? '').trim().toLowerCase();
}

export function isTextEntry(el: Element): el is HTMLInputElement | HTMLTextAreaElement {
  if (isTextArea(el)) return true;
  return isInput(el) && TEXT_INPUT_TYPES.has(inputType(el));
}

export function isContentEditable(el: Element): boolean {
  return isHtml(el) && el.isContentEditable;
}

/** The window an element lives in (its own realm). */
export function windowOf(node: Node): (Window & typeof globalThis) | null {
  const doc = node.nodeType === DOCUMENT_NODE ? (node as Document) : node.ownerDocument;
  return (doc?.defaultView as (Window & typeof globalThis) | null) ?? null;
}

export function styleOf(el: Element): CSSStyleDeclaration | null {
  const w = windowOf(el);
  if (!w) return null;
  try {
    return w.getComputedStyle(el);
  } catch {
    return null;
  }
}

/** The element's shadow root, open or closed (content scripts may see closed ones). */
export function shadowRootOf(el: Element): ShadowRoot | null {
  const open = typeof chrome !== 'undefined' ? chrome.dom?.openOrClosedShadowRoot : undefined;
  if (open && isHtml(el)) {
    try {
      return open(el) ?? null;
    } catch {
      // not a host
    }
  }
  return (el as HTMLElement).shadowRoot ?? null;
}

/**
 * The children that are actually rendered: a shadow host's shadow tree, a
 * slot's assigned nodes (or its fallback content), otherwise the child nodes.
 */
export function renderedChildren(node: Node): Node[] {
  if (isElement(node)) {
    const root = shadowRootOf(node);
    if (root) return Array.from(root.childNodes);
    if (node.localName === 'slot' && isHtml(node)) {
      const assigned = (node as HTMLSlotElement).assignedNodes({ flatten: true });
      if (assigned.length) return assigned;
    }
  }
  return Array.from(node.childNodes);
}

/** Collapse runs of whitespace to one space. */
export function collapse(s: string): string {
  return s.replace(/[\s\u00a0]+/g, ' ');
}

export type Visibility = 'visible' | 'invisible' | 'hidden';

/**
 * - hidden: not rendered (display:none, [hidden], closed <details> content…):
 *   skip the whole subtree;
 * - invisible: visibility:hidden — its own text is not shown but a child may
 *   be visible again;
 * - visible.
 */
export function visibilityOf(el: Element, opts: { ariaHidden: boolean }): Visibility {
  if (el.hasAttribute('hidden') && el.getAttribute('hidden') !== 'until-found') return 'hidden';
  if (opts.ariaHidden && el.getAttribute('aria-hidden') === 'true') return 'hidden';
  if (el.hasAttribute('inert') && opts.ariaHidden) return 'hidden';
  const t = tag(el);
  if (
    t === 'script' ||
    t === 'style' ||
    t === 'template' ||
    t === 'noscript' ||
    t === 'head' ||
    t === 'meta' ||
    t === 'link' ||
    t === 'title'
  )
    return 'hidden';
  if (isInput(el) && inputType(el) === 'hidden') return 'hidden';
  const s = styleOf(el);
  if (s) {
    if (s.display === 'none') return 'hidden';
    if (s.contentVisibility === 'hidden') return 'hidden';
    if (s.visibility === 'hidden' || s.visibility === 'collapse') return 'invisible';
  }
  return 'visible';
}

/** Is the element's box laid out (Chrome's checkVisibility, else styles only)? */
export function isRendered(el: Element): boolean {
  const check = (el as Element & { checkVisibility?: (o?: object) => boolean }).checkVisibility;
  if (typeof check === 'function') {
    try {
      if (!check.call(el, { visibilityProperty: true })) return false;
    } catch {
      // older engines
    }
  }
  for (let e: Element | null = el; e; e = parentElementOrHost(e)) {
    if (visibilityOf(e, { ariaHidden: false }) === 'hidden') return false;
  }
  return true;
}

/** Parent element, crossing a shadow root to its host. */
export function parentElementOrHost(el: Element): Element | null {
  if (el.parentElement) return el.parentElement;
  const root = el.parentNode;
  if (root && root.nodeType === DOCUMENT_FRAGMENT_NODE && (root as ShadowRoot).host)
    return (root as ShadowRoot).host;
  return null;
}

/** Is `inner` inside `outer`, through shadow roots and same-origin frames? */
export function containsDeep(outer: Element, inner: Element): boolean {
  for (let e: Element | null = inner; e;) {
    if (e === outer) return true;
    const parent = parentElementOrHost(e);
    if (parent) {
      e = parent;
      continue;
    }
    const frame: Element | null = windowOf(e)?.frameElement ?? null;
    e = frame;
  }
  return false;
}

const BLOCK_DISPLAYS = new Set([
  'block',
  'flex',
  'grid',
  'table',
  'list-item',
  'flow-root',
  'table-row',
  'table-row-group',
  'table-header-group',
  'table-footer-group',
  'table-caption',
]);

const BLOCK_TAGS = new Set([
  'address',
  'article',
  'aside',
  'blockquote',
  'dd',
  'details',
  'dialog',
  'div',
  'dl',
  'dt',
  'fieldset',
  'figcaption',
  'figure',
  'footer',
  'form',
  'h1',
  'h2',
  'h3',
  'h4',
  'h5',
  'h6',
  'header',
  'hr',
  'li',
  'main',
  'nav',
  'ol',
  'p',
  'pre',
  'section',
  'summary',
  'table',
  'tr',
  'ul',
]);

/** Does the element start a new line of text (block-level)? */
export function isBlock(el: Element): boolean {
  const s = styleOf(el);
  const d = s?.display;
  if (d) return BLOCK_DISPLAYS.has(d) || d.startsWith('block') || d.startsWith('flex');
  return BLOCK_TAGS.has(tag(el));
}

/** The deepest focused element, through shadow roots and same-origin frames. */
export function deepActiveElement(doc: Document): Element | null {
  let active: Element | null = doc.activeElement;
  for (let i = 0; active && i < 32; i++) {
    const root = shadowRootOf(active);
    if (root?.activeElement) {
      active = root.activeElement;
      continue;
    }
    if (isIframe(active)) {
      const inner = frameDocument(active);
      if (inner?.activeElement && inner.activeElement !== inner.body) {
        active = inner.activeElement;
        continue;
      }
    }
    break;
  }
  return active;
}

/** Same-origin document of an iframe, or null (cross-origin, sandboxed, not loaded). */
export function frameDocument(frame: HTMLIFrameElement): Document | null {
  try {
    const doc = frame.contentDocument;
    return doc && doc.documentElement ? doc : null;
  } catch {
    return null;
  }
}

/** querySelectorAll that also searches shadow roots and same-origin iframes. */
export function deepQueryAll(root: Document | Element, selector: string, limit = 1000): Element[] {
  const out: Element[] = [];
  const visit = (scope: Document | Element | ShadowRoot): void => {
    if (out.length >= limit) return;
    let matches: NodeListOf<Element>;
    try {
      matches = scope.querySelectorAll(selector);
    } catch {
      throw new SyntaxError(`invalid selector: ${selector}`);
    }
    for (const m of Array.from(matches)) {
      if (out.length >= limit) return;
      if (!out.includes(m)) out.push(m);
    }
    const all = scope.querySelectorAll('*');
    for (const el of Array.from(all)) {
      const sr = shadowRootOf(el);
      if (sr) visit(sr);
      if (isIframe(el)) {
        const doc = frameDocument(el);
        if (doc) visit(doc);
      }
    }
  };
  if (isElement(root) && safeMatches(root, selector)) out.push(root);
  visit(root);
  return out;
}

function safeMatches(el: Element, selector: string): boolean {
  try {
    return el.matches(selector);
  } catch {
    return false;
  }
}

/** Truncate to `max` code points, ending in "…". */
export function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  const points = Array.from(s);
  if (points.length <= max) return s;
  return points.slice(0, max - 1).join('') + '…';
}
