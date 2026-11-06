// `find`: matching elements by text, role, name or selector, each with a ref.
// A cheap alternative to a full snapshot.

import { accessibleName } from './accname.ts';
import {
  collapse,
  deepQueryAll,
  frameDocument,
  isElement,
  isIframe,
  isRendered,
  isText,
  parentElementOrHost,
  renderedChildren,
  truncate,
  visibilityOf,
} from './dom.ts';
import { REDACTED, inRedactedSubtree, isSecretField } from './redact.ts';
import type { RefRegistry } from './refs.ts';
import { isInteractive, roleOf } from './roles.ts';
import { NAME_MAX } from './snapshot.ts';

export interface FindParams {
  text?: string;
  role?: string;
  name?: string;
  selector?: string;
  limit?: number;
}

export interface FindMatch {
  ref: string;
  role: string;
  name: string;
  text: string;
}

/** Every rendered element under a document, through shadow roots and same-origin frames. */
export function renderedElements(doc: Document, max = 50_000): Element[] {
  const out: Element[] = [];
  const visit = (node: Node): void => {
    for (const child of renderedChildren(node)) {
      if (out.length >= max) return;
      if (!isElement(child)) continue;
      if (visibilityOf(child, { ariaHidden: false }) === 'hidden') continue;
      out.push(child);
      if (isIframe(child)) {
        const inner = frameDocument(child);
        if (inner?.body) visit(inner.body);
        continue;
      }
      visit(child);
    }
  };
  if (doc.body) visit(doc.body);
  return out;
}

/** Text directly inside an element (its own text nodes). */
function ownText(el: Element): string {
  let s = '';
  for (const n of renderedChildren(el)) if (isText(n)) s += n.data;
  return collapse(s).toLowerCase();
}

function textOf(el: Element): string {
  const inner = (el as HTMLElement).innerText;
  return collapse(typeof inner === 'string' ? inner : (el.textContent ?? '')).trim();
}

/** The element that holds `wanted` most tightly, lifted to a clickable ancestor. */
function textHolder(el: Element, wanted: string): boolean {
  if (ownText(el).includes(wanted)) return true;
  const full = collapse(el.textContent ?? '').toLowerCase();
  if (!full.includes(wanted)) return false;
  // Split over inline children ("Hello <b>world</b>") but no single child has it all.
  for (const child of Array.from(el.children)) {
    if (
      collapse(child.textContent ?? '')
        .toLowerCase()
        .includes(wanted)
    )
      return false;
  }
  return true;
}

function liftToInteractive(el: Element): Element {
  let e: Element | null = el;
  for (let i = 0; e && i < 4; i++, e = parentElementOrHost(e)) {
    if (isInteractive(e, roleOf(e))) return e;
  }
  return el;
}

export function find(doc: Document, refs: RefRegistry, p: FindParams): FindMatch[] {
  const limit = Math.max(1, Math.min(200, Math.floor(p.limit ?? 20)));
  const roleWanted = p.role?.trim().toLowerCase() || undefined;
  const nameWanted = p.name ? collapse(p.name).trim().toLowerCase() : undefined;
  const textWanted = p.text ? collapse(p.text).trim().toLowerCase() : undefined;

  const candidates = p.selector
    ? deepQueryAll(doc, p.selector, 5000).filter((el) => isRendered(el))
    : renderedElements(doc);

  const picked: Element[] = [];
  for (const candidate of candidates) {
    if (picked.length >= limit) break;
    let el = candidate;
    if (textWanted) {
      if (!textHolder(el, textWanted)) continue;
      el = liftToInteractive(el);
    }
    const role = roleOf(el);
    if (roleWanted && (role ?? 'generic') !== roleWanted) continue;
    if (nameWanted && !accessibleName(el, role).toLowerCase().includes(nameWanted)) continue;
    if (!textWanted && !nameWanted && !roleWanted && !p.selector) continue;
    if (!picked.includes(el)) picked.push(el);
  }

  return picked.map((el) => {
    const role = roleOf(el) ?? 'generic';
    const doc2 = el.ownerDocument ?? doc;
    const ref = refs.refFor(el, refs.frameOf(doc2, doc));
    const hidden = inRedactedSubtree(el);
    const name = hidden ? REDACTED : truncate(accessibleName(el, roleOf(el)), NAME_MAX);
    const text = hidden || isSecretField(el) ? REDACTED : truncate(textOf(el), NAME_MAX);
    return { ref, role, name, text };
  });
}
