// ARIA roles: explicit (first valid token of role=) or implicit (HTML-AAM,
// simplified to what a snapshot needs). `null` means "generic": the element
// gets no line of its own and its children join its parent.

import {
  inputType,
  isElement,
  isHtml,
  isInput,
  isRendered,
  isSvg,
  styleOf,
  tag,
  parentElementOrHost,
} from './dom.ts';

const ARIA_ROLES = new Set([
  'alert',
  'alertdialog',
  'application',
  'article',
  'banner',
  'blockquote',
  'button',
  'caption',
  'cell',
  'checkbox',
  'code',
  'columnheader',
  'combobox',
  'complementary',
  'contentinfo',
  'definition',
  'deletion',
  'dialog',
  'directory',
  'document',
  'emphasis',
  'feed',
  'figure',
  'form',
  'generic',
  'grid',
  'gridcell',
  'group',
  'heading',
  'img',
  'image',
  'insertion',
  'link',
  'list',
  'listbox',
  'listitem',
  'log',
  'main',
  'mark',
  'marquee',
  'math',
  'menu',
  'menubar',
  'menuitem',
  'menuitemcheckbox',
  'menuitemradio',
  'meter',
  'navigation',
  'none',
  'note',
  'option',
  'paragraph',
  'presentation',
  'progressbar',
  'radio',
  'radiogroup',
  'region',
  'row',
  'rowgroup',
  'rowheader',
  'scrollbar',
  'search',
  'searchbox',
  'separator',
  'slider',
  'spinbutton',
  'status',
  'strong',
  'subscript',
  'superscript',
  'switch',
  'tab',
  'table',
  'tablist',
  'tabpanel',
  'term',
  'textbox',
  'time',
  'timer',
  'toolbar',
  'tooltip',
  'tree',
  'treegrid',
  'treeitem',
]);

/** Roles a user acts on: these get a [ref=…]. */
export const INTERACTIVE_ROLES = new Set([
  'button',
  'link',
  'textbox',
  'searchbox',
  'checkbox',
  'radio',
  'combobox',
  'listbox',
  'option',
  'slider',
  'spinbutton',
  'switch',
  'tab',
  'menuitem',
  'menuitemcheckbox',
  'menuitemradio',
  'treeitem',
  'gridcell',
  'scrollbar',
]);

/** Roles whose accessible name comes from their content (ARIA 1.2 §5.2.8.6). */
export const NAME_FROM_CONTENT = new Set([
  'button',
  'cell',
  'checkbox',
  'columnheader',
  'gridcell',
  'heading',
  'link',
  'menuitem',
  'menuitemcheckbox',
  'menuitemradio',
  'option',
  'radio',
  'row',
  'rowheader',
  'switch',
  'tab',
  'tooltip',
  'treeitem',
]);

/** Name-from-content roles whose text is fully told by the name (no text lines below). */
export const LEAF_TEXT_ROLES = new Set([
  'button',
  'checkbox',
  'heading',
  'link',
  'menuitem',
  'menuitemcheckbox',
  'menuitemradio',
  'option',
  'radio',
  'switch',
  'tab',
  'tooltip',
  'treeitem',
]);

/** Containers that give context to the interactive nodes inside them. */
export const CONTEXT_ROLES = new Set([
  'banner',
  'navigation',
  'main',
  'contentinfo',
  'complementary',
  'region',
  'search',
  'form',
  'dialog',
  'alertdialog',
  'menu',
  'menubar',
  'listbox',
  'tablist',
  'tabpanel',
  'tree',
  'treegrid',
  'grid',
  'table',
  'row',
  'toolbar',
  'radiogroup',
  'group',
  'iframe',
]);

/** Landmark-ish elements that `header`/`footer` lose their role inside. */
const SECTIONING = new Set(['article', 'aside', 'main', 'nav', 'section']);

function explicitRole(el: Element): string | null {
  const attr = el.getAttribute('role');
  if (!attr) return null;
  for (const token of attr.trim().toLowerCase().split(/\s+/)) {
    if (ARIA_ROLES.has(token)) return token;
  }
  return null;
}

function hasAccessibleLabel(el: Element): boolean {
  return !!(
    el.getAttribute('aria-label')?.trim() ||
    el.getAttribute('aria-labelledby')?.trim() ||
    el.getAttribute('title')?.trim()
  );
}

function insideSectioning(el: Element): boolean {
  for (let p = parentElementOrHost(el); p; p = parentElementOrHost(p)) {
    if (SECTIONING.has(tag(p))) return true;
    const r = p.getAttribute('role');
    if (r && /\b(article|complementary|main|navigation|region)\b/.test(r)) return true;
  }
  return false;
}

export function implicitRole(el: Element): string | null {
  if (isSvg(el)) {
    if (tag(el) === 'svg') {
      const hasTitle = Array.from(el.children).some((c) => c.localName === 'title');
      return hasTitle || hasAccessibleLabel(el) ? 'img' : null;
    }
    return null;
  }
  if (!isHtml(el)) return null;
  switch (el.localName) {
    case 'a':
    case 'area':
      return el.hasAttribute('href') ? 'link' : null;
    case 'article':
      return 'article';
    case 'aside':
      return 'complementary';
    case 'blockquote':
      return 'blockquote';
    case 'button':
      return 'button';
    case 'datalist':
      return 'listbox';
    case 'dd':
      return 'definition';
    case 'details':
      return 'group';
    case 'dfn':
    case 'dt':
      return 'term';
    case 'dialog':
      return 'dialog';
    case 'fieldset':
      return 'group';
    case 'figure':
      return 'figure';
    case 'footer':
      return insideSectioning(el) ? null : 'contentinfo';
    case 'form':
      return 'form';
    case 'h1':
    case 'h2':
    case 'h3':
    case 'h4':
    case 'h5':
    case 'h6':
      return 'heading';
    case 'header':
      return insideSectioning(el) ? null : 'banner';
    case 'hr':
      return 'separator';
    case 'iframe':
    case 'frame':
      return 'iframe';
    case 'img': {
      const alt = el.getAttribute('alt');
      if (alt !== null && alt.trim() === '' && !hasAccessibleLabel(el)) return null;
      return 'img';
    }
    case 'input':
      return inputRole(el as HTMLInputElement);
    case 'li':
      return 'listitem';
    case 'main':
      return 'main';
    case 'math':
      return 'math';
    case 'menu':
    case 'ol':
    case 'ul':
      return 'list';
    case 'meter':
      return 'meter';
    case 'nav':
      return 'navigation';
    case 'optgroup':
      return 'group';
    case 'option':
      return 'option';
    case 'output':
      return 'status';
    case 'p':
      return 'paragraph';
    case 'progress':
      return 'progressbar';
    case 'search':
      return 'search';
    case 'section':
      return hasAccessibleLabel(el) ? 'region' : null;
    case 'select': {
      const s = el as HTMLSelectElement;
      return s.multiple || s.size > 1 ? 'listbox' : 'combobox';
    }
    case 'summary':
      return 'button';
    case 'table':
      return 'table';
    case 'td':
      return closestRole(el, ['grid', 'treegrid']) ? 'gridcell' : 'cell';
    case 'th':
      return el.getAttribute('scope') === 'row' ? 'rowheader' : 'columnheader';
    case 'textarea':
      return 'textbox';
    case 'tr':
      return 'row';
    default:
      return null;
  }
}

function inputRole(el: HTMLInputElement): string | null {
  const type = inputType(el);
  const hasList = el.hasAttribute('list');
  switch (type) {
    case 'button':
    case 'submit':
    case 'reset':
    case 'image':
    case 'file':
    case 'color':
      return 'button';
    case 'checkbox':
      return el.getAttribute('role') === 'switch' ? 'switch' : 'checkbox';
    case 'radio':
      return 'radio';
    case 'range':
      return 'slider';
    case 'number':
      return 'spinbutton';
    case 'search':
      return hasList ? 'combobox' : 'searchbox';
    case 'hidden':
      return null;
    default:
      return hasList ? 'combobox' : 'textbox';
  }
}

function closestRole(el: Element, roles: string[]): boolean {
  for (let p = parentElementOrHost(el); p; p = parentElementOrHost(p)) {
    const r = explicitRole(p) ?? (tag(p) === 'table' ? 'table' : null);
    if (r && roles.includes(r)) return true;
    if (tag(p) === 'table') return false;
  }
  return false;
}

/** Is the element focusable by keyboard (tabindex ≥ 0 or natively)? */
export function isFocusable(el: Element): boolean {
  if (!isHtml(el) && !isSvg(el)) return false;
  const ti = el.getAttribute('tabindex');
  if (ti !== null) {
    const n = Number.parseInt(ti, 10);
    if (!Number.isNaN(n)) return n >= 0;
  }
  if (!isHtml(el)) return false;
  const t = el.localName;
  if (t === 'a' || t === 'area') return el.hasAttribute('href');
  if (t === 'button' || t === 'input' || t === 'select' || t === 'textarea') {
    return !(el as HTMLButtonElement).disabled && !(isInput(el) && inputType(el) === 'hidden');
  }
  if (t === 'summary' || t === 'iframe') return true;
  return el.isContentEditable;
}

/** The role used in the snapshot, or null for a generic element. */
export function roleOf(el: Element): string | null {
  const explicit = explicitRole(el);
  if (explicit === 'none' || explicit === 'presentation') {
    // A focusable element keeps its native role (ARIA conflict resolution).
    return isFocusable(el) ? implicitRole(el) : null;
  }
  if (explicit === 'generic') return null;
  if (explicit === 'image') return 'img';
  if (explicit) return explicit;
  if (isHtml(el) && el.isContentEditable && isEditingHost(el)) return 'textbox';
  return implicitRole(el);
}

/** The top of a contenteditable region (its parent is not editable). */
export function isEditingHost(el: HTMLElement): boolean {
  if (!el.isContentEditable) return false;
  const parent = parentElementOrHost(el);
  return !(parent && isHtml(parent) && parent.isContentEditable);
}

/** Inside a native <select> or <datalist> (options there are not clicked). */
function inNativeList(el: Element): boolean {
  const p = el.parentElement;
  const t = p ? tag(p) : '';
  if (t === 'select' || t === 'datalist') return true;
  if (t === 'optgroup') return tag(p?.parentElement ?? el) === 'select';
  return false;
}

/**
 * Could a user act on it? Interactive roles, focusable elements, contenteditable
 * hosts, onclick handlers, and the outermost element with `cursor: pointer`.
 */
export function isInteractive(el: Element, role: string | null): boolean {
  if (role === 'option') return !inNativeList(el);
  if (role === 'gridcell') return isFocusable(el);
  if (role && INTERACTIVE_ROLES.has(role)) return true;
  if (isInput(el)) return inputType(el) !== 'hidden';
  const t = tag(el);
  if (t === 'button' || t === 'select' || t === 'textarea' || t === 'summary') return true;
  if (t === 'a' && el.hasAttribute('href')) return true;
  if (isFocusable(el)) return true;
  if (el.hasAttribute('onclick')) return true;
  if (isHtml(el) && el.isContentEditable && isEditingHost(el)) return true;
  if (t === 'label') {
    // A styled label standing in for a checkbox whose real input is not rendered.
    const control = (el as HTMLLabelElement).control;
    return !!control && !isRendered(control);
  }
  if (t === 'body' || t === 'html') return false;
  return hasPointerCursor(el);
}

function hasPointerCursor(el: Element): boolean {
  const s = styleOf(el);
  if (!s || s.cursor !== 'pointer') return false;
  const parent = parentElementOrHost(el);
  if (!parent || !isElement(parent)) return true;
  return styleOf(parent)?.cursor !== 'pointer';
}

/** Heading level for a heading role. */
export function headingLevel(el: Element): number | null {
  const aria = Number.parseInt(el.getAttribute('aria-level') ?? '', 10);
  if (!Number.isNaN(aria) && aria > 0) return aria;
  const m = /^h([1-6])$/.exec(tag(el));
  return m?.[1] ? Number(m[1]) : 2;
}
