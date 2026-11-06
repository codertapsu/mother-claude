// Accessible name, an approximation of accname 1.2 good enough for a model to
// recognise controls: aria-labelledby, aria-label, <label for> / wrapping
// label, alt, legend/caption/figcaption, text content for the roles that take
// their name from it, then title and placeholder.

import {
  collapse,
  isBlock,
  isElement,
  isHtml,
  isInput,
  isSelect,
  isSvg,
  isText,
  inputType,
  renderedChildren,
  styleOf,
  tag,
} from './dom.ts';
import { isSecretField } from './redact.ts';
import { NAME_FROM_CONTENT, roleOf } from './roles.ts';

/** Enough text to fill a 120-character name after collapsing whitespace. */
const GATHER_LIMIT = 600;

interface State {
  visited: Set<Element>;
  /** Inside an aria-labelledby traversal (no second hop). */
  labelledBy: boolean;
  /** Directly or indirectly referenced by aria-labelledby: hidden text counts. */
  referenced: boolean;
  /** Computing a name from content (an ancestor's). */
  inner: boolean;
}

export function accessibleName(el: Element, role: string | null = roleOf(el)): string {
  const state: State = { visited: new Set(), labelledBy: false, referenced: false, inner: false };
  return clean(nameOf(el, role, state));
}

function clean(s: string): string {
  return collapse(s).trim();
}

function hiddenForName(el: Element): boolean {
  if (el.hasAttribute('hidden')) return true;
  if (el.getAttribute('aria-hidden') === 'true') return true;
  const s = styleOf(el);
  return !!s && (s.display === 'none' || s.visibility === 'hidden');
}

function byId(el: Element, id: string): Element | null {
  const root = el.getRootNode() as Document | ShadowRoot;
  if (typeof root.getElementById === 'function') return root.getElementById(id);
  return el.ownerDocument?.getElementById(id) ?? null;
}

function nameOf(el: Element, role: string | null, st: State): string {
  if (st.visited.has(el)) return '';
  st.visited.add(el);
  if (!st.referenced && hiddenForName(el)) return '';

  // 2B: aria-labelledby (one hop only).
  if (!st.labelledBy) {
    const ids = el.getAttribute('aria-labelledby')?.trim();
    if (ids) {
      const parts: string[] = [];
      for (const id of ids.split(/\s+/)) {
        const target = byId(el, id);
        if (!target) continue;
        const sub: State = { ...st, labelledBy: true, referenced: true, inner: true };
        // A label may name the element it labels: allow revisiting it once.
        if (target === el) parts.push(clean(nameFromContent(el, sub)));
        else parts.push(clean(nameOf(target, roleOf(target), sub)));
      }
      const joined = clean(parts.join(' '));
      if (joined) return joined;
    }
  }

  // 2C: an embedded control inside someone else's name gives its value.
  if (st.inner) {
    const value = embeddedValue(el, role);
    if (value !== null) return value;
  }

  // 2D: aria-label.
  const ariaLabel = el.getAttribute('aria-label')?.trim();
  if (ariaLabel) return ariaLabel;

  // 2E: native label sources.
  const native = nativeName(el, st);
  if (native) return native;

  // 2F: name from content.
  if (st.inner || (role !== null && NAME_FROM_CONTENT.has(role))) {
    const text = clean(nameFromContent(el, st));
    if (text) return text;
  }

  // 2I: tooltip, then placeholder.
  const title = el.getAttribute('title')?.trim();
  if (title) return title;
  if (!st.inner) {
    const placeholder =
      el.getAttribute('placeholder')?.trim() || el.getAttribute('aria-placeholder')?.trim();
    if (placeholder) return placeholder;
  }
  return '';
}

function nativeName(el: Element, st: State): string {
  if (isSvg(el)) {
    const title = Array.from(el.children).find((c) => c.localName === 'title');
    return title ? clean(title.textContent ?? '') : '';
  }
  if (!isHtml(el)) return '';
  const t = el.localName;
  if (t === 'input') {
    const input = el as HTMLInputElement;
    const type = inputType(input);
    if (type === 'button' || type === 'submit' || type === 'reset') {
      const v = input.getAttribute('value');
      if (v !== null && v.trim()) return v.trim();
      if (type === 'submit') return labelsText(input, st) || 'Submit';
      if (type === 'reset') return labelsText(input, st) || 'Reset';
      return labelsText(input, st);
    }
    if (type === 'image') {
      return input.getAttribute('alt')?.trim() || input.getAttribute('value')?.trim() || 'Submit';
    }
    return labelsText(input, st);
  }
  if (t === 'textarea' || t === 'select' || t === 'meter' || t === 'progress' || t === 'output') {
    return labelsText(el, st);
  }
  if (t === 'button') return labelsText(el, st);
  if (t === 'img' || t === 'area') return el.getAttribute('alt')?.trim() ?? '';
  if (t === 'fieldset') return firstChildText(el, 'legend', st);
  if (t === 'figure') return firstChildText(el, 'figcaption', st);
  if (t === 'table') return firstChildText(el, 'caption', st);
  return '';
}

function labelsText(el: Element, st: State): string {
  const labels = (el as HTMLInputElement).labels;
  if (!labels || labels.length === 0) return '';
  const parts: string[] = [];
  for (const label of Array.from(labels)) {
    parts.push(clean(nameFromContent(label, { ...st, inner: true })));
  }
  return clean(parts.join(' '));
}

function firstChildText(el: Element, childTag: string, st: State): string {
  const child = Array.from(el.children).find((c) => tag(c) === childTag);
  return child ? clean(nameFromContent(child, { ...st, inner: true })) : '';
}

function nameFromContent(el: Element, st: State): string {
  let out = '';
  for (const child of renderedChildren(el)) {
    if (out.length > GATHER_LIMIT) break;
    if (isText(child)) {
      out += child.data;
      continue;
    }
    if (!isElement(child)) continue;
    if (tag(child) === 'br') {
      out += ' ';
      continue;
    }
    if (!st.referenced && hiddenForName(child)) continue;
    const piece = nameOf(child, roleOf(child), { ...st, inner: true });
    out += isBlock(child) ? ` ${piece} ` : piece;
  }
  return out;
}

/** The value an embedded control contributes to an ancestor's name, or null. */
function embeddedValue(el: Element, role: string | null): string | null {
  if (isSecretField(el)) return '';
  if (isInput(el)) {
    const type = inputType(el);
    if (type === 'checkbox' || type === 'radio') return null;
    if (type === 'button' || type === 'submit' || type === 'reset' || type === 'image') return null;
    return el.value;
  }
  if (tag(el) === 'textarea') return (el as HTMLTextAreaElement).value;
  if (isSelect(el)) {
    return Array.from(el.selectedOptions)
      .map((o) => o.label || o.text)
      .join(' ');
  }
  if (role === 'slider' || role === 'spinbutton' || role === 'progressbar' || role === 'meter') {
    return el.getAttribute('aria-valuetext') ?? el.getAttribute('aria-valuenow') ?? '';
  }
  if (role === 'textbox' || role === 'searchbox') return clean(el.textContent ?? '');
  return null;
}
