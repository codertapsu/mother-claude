// Acting on the page with synthetic events. Values are set through the
// element's own prototype setter and announced with `input` + `change`, so
// React's and Vue's bookkeeping sees them; text is inserted with
// `execCommand('insertText')` where the page allows it (the browser then
// fires real beforeinput/input events). `trusted: true` goes through the
// debugger in the service worker instead.

import { accessibleName } from './accname.ts';
import {
  containsDeep,
  deepActiveElement,
  inputType,
  isContentEditable,
  isElement,
  isHtml,
  isInput,
  isSelect,
  isTextEntry,
  parentElementOrHost,
  shadowRootOf,
  styleOf,
  tag,
  windowOf,
} from './dom.ts';
import { CommandError } from '../shared/errors.ts';
import { charKey, type KeyDef, type KeySpec } from '../shared/keys.ts';
import { roleOf } from './roles.ts';
import { isDisabled } from './snapshot.ts';

export interface Point {
  x: number;
  y: number;
}

type Win = Window & typeof globalThis;

function win(el: Node): Win {
  const w = windowOf(el);
  if (!w) throw new CommandError('stale_ref', 'the element is no longer in a page');
  return w;
}

/** Scroll the element (and its frames) into view when it isn't fully visible. */
export function scrollIntoViewIfNeeded(el: Element): void {
  const w = windowOf(el);
  if (!w) return;
  const frame = w.frameElement;
  if (frame) scrollIntoViewIfNeeded(frame);
  const r = el.getBoundingClientRect();
  const visible = r.top >= 0 && r.left >= 0 && r.bottom <= w.innerHeight && r.right <= w.innerWidth;
  if (!visible) el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
}

/** Centre of the element's first box, in its own frame's viewport. */
export function centerOf(el: Element): Point {
  const rects = el.getClientRects();
  const r = rects.length > 0 && rects[0] ? rects[0] : el.getBoundingClientRect();
  return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
}

/** A point of a frame's viewport in the top window's viewport (CSS px). */
export function toTopPoint(el: Element, p: Point): Point {
  let { x, y } = p;
  let w = windowOf(el);
  for (let i = 0; w?.frameElement && i < 16; i++) {
    const fe = w.frameElement;
    const fr = fe.getBoundingClientRect();
    const s = styleOf(fe);
    x += fr.left + fe.clientLeft + (Number.parseFloat(s?.paddingLeft ?? '0') || 0);
    y += fr.top + fe.clientTop + (Number.parseFloat(s?.paddingTop ?? '0') || 0);
    w = windowOf(fe);
  }
  return { x, y };
}

/** The topmost element at a point, looking inside shadow roots. */
function hitTest(doc: Document, p: Point): Element | null {
  let hit = doc.elementFromPoint(p.x, p.y);
  for (let i = 0; hit && i < 16; i++) {
    const root = shadowRootOf(hit);
    const inner = root?.elementFromPoint(p.x, p.y) ?? null;
    if (!inner || inner === hit) break;
    hit = inner;
  }
  return hit;
}

function describe(el: Element): string {
  const role = roleOf(el) ?? tag(el);
  const name = accessibleName(el).slice(0, 60);
  return name ? `${role} "${name}"` : role;
}

/**
 * Where a trusted click should land, in top-viewport CSS px. Refuses when
 * something else covers the element there (a cookie banner, a modal).
 */
export function trustedPoint(el: Element): Point {
  scrollIntoViewIfNeeded(el);
  const p = centerOf(el);
  const doc = el.ownerDocument;
  if (doc) {
    const hit = hitTest(doc, p);
    if (hit && hit !== el && !containsDeep(el, hit) && !containsDeep(hit, el)) {
      throw new CommandError(
        'unsupported',
        `the element is covered by ${describe(hit)} there: close or scroll it away first`,
      );
    }
  }
  return toTopPoint(el, p);
}

// ------------------------------------------------------------------ mouse

const NON_BUBBLING = new Set(['mouseenter', 'mouseleave', 'pointerenter', 'pointerleave']);
const lastHovered = new WeakMap<Document, Element>();

function mouse(el: Element, type: string, p: Point, init: MouseEventInit = {}): boolean {
  const w = win(el);
  const bubbles = !NON_BUBBLING.has(type);
  const base: PointerEventInit = {
    bubbles,
    cancelable: bubbles,
    composed: true,
    view: w,
    clientX: p.x,
    clientY: p.y,
    screenX: p.x,
    screenY: p.y,
    ...init,
  };
  const event = type.startsWith('pointer')
    ? new (w.PointerEvent ?? w.MouseEvent)(type, {
        ...base,
        pointerId: 1,
        pointerType: 'mouse',
        isPrimary: true,
      })
    : new w.MouseEvent(type, base);
  return el.dispatchEvent(event);
}

function isFocusableNow(el: Element): boolean {
  const h = el as HTMLElement;
  if (typeof h.focus !== 'function' || isDisabled(el)) return false;
  if (isHtml(el) && el.isContentEditable) return true;
  return typeof h.tabIndex === 'number' && h.tabIndex >= 0;
}

/** What a real mousedown focuses: the nearest focusable ancestor, else nothing. */
function focusFor(el: Element): void {
  for (let e: Element | null = el; e; e = parentElementOrHost(e)) {
    if (isFocusableNow(e)) {
      (e as HTMLElement).focus({ preventScroll: true });
      return;
    }
  }
  const active = el.ownerDocument?.activeElement as HTMLElement | null | undefined;
  if (active && active !== el.ownerDocument?.body) active.blur?.();
}

export function hover(el: Element): void {
  const doc = el.ownerDocument;
  const p = centerOf(el);
  const previous = doc ? lastHovered.get(doc) : undefined;
  if (previous && previous !== el && previous.isConnected) {
    const q = centerOf(previous);
    mouse(previous, 'pointerout', q);
    mouse(previous, 'pointerleave', q);
    mouse(previous, 'mouseout', q);
    mouse(previous, 'mouseleave', q);
  }
  mouse(el, 'pointerover', p);
  mouse(el, 'pointerenter', p);
  mouse(el, 'mouseover', p);
  mouse(el, 'mouseenter', p);
  mouse(el, 'pointermove', p);
  mouse(el, 'mousemove', p);
  if (doc) lastHovered.set(doc, el);
}

/** The element a drag of `el` picks up: itself or an ancestor the browser
 * would drag (draggable="true", an image, a link). */
function draggableOf(el: Element): HTMLElement | null {
  for (let e: Element | null = el; e; e = parentElementOrHost(e)) {
    if (isHtml(e) && (e as HTMLElement).draggable) return e as HTMLElement;
  }
  return null;
}

const nextFrame = (): Promise<void> => new Promise((r) => setTimeout(r, 16));

export interface DragOutcome {
  /** HTML drag and drop (the source is draggable), else pointer events. */
  html5: boolean;
  /** HTML drag and drop: the target accepted the drop (its dragover said so). */
  dropped?: boolean;
}

/**
 * A synthetic drag of `from` onto `to`, as a user's would fire it: the HTML
 * drag-and-drop events (dragstart … drop, dragend) with one DataTransfer when
 * the source is draggable; else pointer and mouse events with the button
 * held, moved along the way in a few steps (sortable lists, sliders built on
 * pointer events).
 */
export async function drag(from: Element, to: Element): Promise<DragOutcome> {
  scrollIntoViewIfNeeded(from);
  const a = centerOf(from);
  hover(from);
  const press = { button: 0, buttons: 1 };
  mouse(from, 'pointerdown', a, press);
  mouse(from, 'mousedown', a, press);
  const source = draggableOf(from);
  if (source) {
    const w = win(from);
    const dt = new w.DataTransfer();
    const fire = (el: Element, type: string, p: Point): boolean =>
      el.dispatchEvent(
        new (win(el).DragEvent)(type, {
          bubbles: true,
          cancelable: type !== 'dragend' && type !== 'dragleave',
          composed: true,
          view: win(el),
          clientX: p.x,
          clientY: p.y,
          dataTransfer: dt,
        }),
      );
    if (!fire(source, 'dragstart', a)) {
      // The page refused the drag.
      mouse(from, 'pointerup', a, { button: 0, buttons: 0 });
      mouse(from, 'mouseup', a, { button: 0, buttons: 0 });
      return { html5: true, dropped: false };
    }
    fire(source, 'drag', a);
    await nextFrame();
    scrollIntoViewIfNeeded(to);
    const b = centerOf(to);
    fire(to, 'dragenter', b);
    // A drop target says yes by cancelling dragover.
    const accepted = !fire(to, 'dragover', b);
    if (accepted) fire(to, 'drop', b);
    else fire(to, 'dragleave', b);
    fire(source, 'dragend', b);
    return { html5: true, dropped: accepted };
  }
  scrollIntoViewIfNeeded(to);
  const start = centerOf(from);
  const b = centerOf(to);
  const doc = to.ownerDocument;
  const steps = 8;
  for (let i = 1; i <= steps; i++) {
    const p = {
      x: start.x + ((b.x - start.x) * i) / steps,
      y: start.y + ((b.y - start.y) * i) / steps,
    };
    const under = (doc && hitTest(doc, p)) || to;
    mouse(under, 'pointermove', p, { buttons: 1 });
    mouse(under, 'mousemove', p, { buttons: 1 });
    await nextFrame();
  }
  mouse(to, 'pointerup', b, { button: 0, buttons: 0 });
  mouse(to, 'mouseup', b, { button: 0, buttons: 0 });
  return { html5: false };
}

export type Button = 'left' | 'middle' | 'right';

export interface ClickOutcome {
  /** A link that wanted a new tab (a synthetic click cannot open one). */
  openUrl?: string;
}

/** A full synthetic click (or double click) on the element's centre. */
export function click(el: Element, button: Button = 'left', count: 1 | 2 = 1): ClickOutcome {
  const p = centerOf(el);
  const b = button === 'middle' ? 1 : button === 'right' ? 2 : 0;
  const pressed = button === 'middle' ? 4 : button === 'right' ? 2 : 1;
  hover(el);
  const anchor = newTabAnchor(el);
  let defaultPrevented = false;
  for (let detail = 1; detail <= count; detail++) {
    mouse(el, 'pointerdown', p, { button: b, buttons: pressed, detail });
    if (mouse(el, 'mousedown', p, { button: b, buttons: pressed, detail })) focusFor(el);
    mouse(el, 'pointerup', p, { button: b, buttons: 0, detail });
    mouse(el, 'mouseup', p, { button: b, buttons: 0, detail });
    if (b === 0) {
      if (anchor) {
        // Let the page see the click, but open the tab ourselves: the popup
        // blocker drops a new tab that no user gesture asked for. The page's
        // own verdict is read at the window, after its handlers ran.
        const w = win(el);
        const onWindow = (e: Event): void => {
          defaultPrevented = e.defaultPrevented;
          e.preventDefault();
        };
        w.addEventListener('click', onWindow, { once: true });
        mouse(el, 'click', p, { button: 0, buttons: 0, detail });
        w.removeEventListener('click', onWindow);
      } else {
        mouse(el, 'click', p, { button: 0, buttons: 0, detail });
      }
    } else if (b === 1) {
      mouse(el, 'auxclick', p, { button: 1, buttons: 0, detail });
    } else {
      mouse(el, 'contextmenu', p, { button: 2, buttons: 0, detail });
    }
  }
  if (count === 2 && b === 0) mouse(el, 'dblclick', p, { button: 0, buttons: 0, detail: 2 });
  if (anchor && !defaultPrevented && anchor.href) return { openUrl: anchor.href };
  return {};
}

/** The link around `el` that opens in another browsing context, if any. */
function newTabAnchor(el: Element): HTMLAnchorElement | null {
  for (let e: Element | null = el; e; e = parentElementOrHost(e)) {
    if (isHtml(e) && e.localName === 'a' && (e as HTMLAnchorElement).href) {
      const target = (e.getAttribute('target') ?? '').toLowerCase();
      if (target && target !== '_self' && target !== '_parent' && target !== '_top') {
        return e as HTMLAnchorElement;
      }
      return null;
    }
  }
  return null;
}

// --------------------------------------------------------------- keyboard

function keyEvent(el: Element, type: string, def: KeyDef, spec?: KeySpec): boolean {
  const w = win(el);
  const mods = spec?.modifiers ?? [];
  const init: KeyboardEventInit & { keyCode?: number; which?: number; charCode?: number } = {
    key: def.key,
    code: def.code,
    bubbles: true,
    cancelable: true,
    composed: true,
    view: w,
    ctrlKey: mods.includes('Control'),
    shiftKey: mods.includes('Shift'),
    altKey: mods.includes('Alt'),
    metaKey: mods.includes('Meta'),
    keyCode: type === 'keypress' ? (def.text ? def.text.charCodeAt(0) : 0) : def.keyCode,
    which: type === 'keypress' ? (def.text ? def.text.charCodeAt(0) : 0) : def.keyCode,
    charCode: type === 'keypress' && def.text ? def.text.charCodeAt(0) : 0,
  };
  return el.dispatchEvent(new w.KeyboardEvent(type, init));
}

/** The prototype's own `value` setter: bypasses the instance overrides frameworks add. */
export function setNativeValue(el: Element, value: string): void {
  let proto: object | null = Object.getPrototypeOf(el) as object | null;
  while (proto) {
    const desc = Object.getOwnPropertyDescriptor(proto, 'value');
    if (desc?.set) {
      desc.set.call(el, value);
      return;
    }
    proto = Object.getPrototypeOf(proto) as object | null;
  }
  (el as HTMLInputElement).value = value;
}

function fire(el: Element, type: 'input' | 'change', data?: string): void {
  const w = win(el);
  const event =
    type === 'input'
      ? new w.InputEvent('input', {
          bubbles: true,
          composed: true,
          inputType: 'insertText',
          ...(data !== undefined ? { data } : {}),
        })
      : new w.Event('change', { bubbles: true });
  el.dispatchEvent(event);
}

function editable(el: Element): boolean {
  if (isTextEntry(el)) return !(el as HTMLInputElement).readOnly && !isDisabled(el);
  return isContentEditable(el);
}

function focusElement(el: Element): void {
  (el as HTMLElement).focus?.({ preventScroll: true });
}

function caretToEnd(el: Element): void {
  if (isTextEntry(el)) {
    try {
      const n = el.value.length;
      el.setSelectionRange(n, n);
    } catch {
      // email/number inputs have no selection API
    }
    return;
  }
  if (isContentEditable(el)) {
    const doc = el.ownerDocument;
    const sel = windowOf(el)?.getSelection();
    if (!doc || !sel) return;
    const range = doc.createRange();
    range.selectNodeContents(el);
    range.collapse(false);
    sel.removeAllRanges();
    sel.addRange(range);
  }
}

function selectAll(el: Element): void {
  if (isTextEntry(el)) {
    try {
      el.select();
    } catch {
      // ignore
    }
    return;
  }
  const doc = el.ownerDocument;
  const sel = windowOf(el)?.getSelection();
  if (!doc || !sel) return;
  const range = doc.createRange();
  range.selectNodeContents(el);
  sel.removeAllRanges();
  sel.addRange(range);
}

function execCommand(el: Element, command: string, value?: string): boolean {
  const doc = el.ownerDocument;
  if (!doc || typeof doc.execCommand !== 'function') return false;
  try {
    return doc.execCommand(command, false, value);
  } catch {
    return false;
  }
}

/** Insert text at the caret as typing would. */
export function insertText(el: Element, text: string): void {
  if (execCommand(el, 'insertText', text)) return;
  if (isTextEntry(el)) {
    const v = el.value;
    let start = v.length;
    let end = v.length;
    try {
      start = el.selectionStart ?? v.length;
      end = el.selectionEnd ?? v.length;
    } catch {
      // no selection API
    }
    setNativeValue(el, v.slice(0, start) + text + v.slice(end));
    try {
      el.setSelectionRange(start + text.length, start + text.length);
    } catch {
      // no selection API
    }
    fire(el, 'input', text);
    return;
  }
  if (isContentEditable(el)) {
    const doc = el.ownerDocument;
    const sel = windowOf(el)?.getSelection();
    if (!doc || !sel) return;
    if (!sel.rangeCount) caretToEnd(el);
    const range = sel.getRangeAt(0);
    range.deleteContents();
    const node = doc.createTextNode(text);
    range.insertNode(node);
    range.setStartAfter(node);
    range.collapse(true);
    fire(el, 'input', text);
  }
}

function clearField(el: Element): void {
  selectAll(el);
  if (execCommand(el, 'delete')) {
    if (!isTextEntry(el) || el.value === '') return;
  }
  if (isTextEntry(el)) {
    setNativeValue(el, '');
    fire(el, 'input');
  } else if (isContentEditable(el)) {
    el.textContent = '';
    fire(el, 'input');
  }
}

function assertEditable(el: Element, what: string): void {
  if (!isTextEntry(el) && !isContentEditable(el)) {
    throw new CommandError('unsupported', `${what} is not a text field (${describe(el)})`);
  }
  if (!editable(el)) throw new CommandError('unsupported', `${what} is disabled or read-only`);
}

/** Type text key by key: keydown, keypress, the insertion, keyup. */
export function type(el: Element, text: string, opts: { clear?: boolean; what: string }): void {
  if (isInput(el) && DIRECT_VALUE_TYPES.has(inputType(el))) {
    const current = opts.clear ? '' : el.value;
    fillField(el, current + text, opts.what);
    return;
  }
  assertEditable(el, opts.what);
  focusElement(el);
  caretToEnd(el);
  if (opts.clear) clearField(el);
  for (const ch of Array.from(text)) {
    const def = charKey(ch);
    if (!keyEvent(el, 'keydown', def)) {
      keyEvent(el, 'keyup', def);
      continue;
    }
    if (def.key === 'Enter') {
      if (isTextEntry(el) && el.localName === 'input') {
        keyEvent(el, 'keypress', def);
      } else {
        keyEvent(el, 'keypress', def);
        if (!execCommand(el, 'insertLineBreak')) insertText(el, '\n');
      }
    } else if (keyEvent(el, 'keypress', def)) {
      insertText(el, ch);
    }
    keyEvent(el, 'keyup', def);
  }
  if (isTextEntry(el)) fire(el, 'change');
}

/** Submit the field's form the way pressing Enter would. */
export function submitFrom(el: Element): boolean {
  const form = (el as HTMLInputElement).form ?? el.closest('form');
  if (!form) return false;
  const button = form.querySelector<HTMLElement>(
    'button:not([type]), button[type="submit"], input[type="submit"], input[type="image"]',
  );
  if (button && !isDisabled(button)) {
    click(button);
    return true;
  }
  try {
    form.requestSubmit();
  } catch {
    form.submit();
  }
  return true;
}

export function pressEnterOn(el: Element): void {
  const def = charKey('\n');
  const go = keyEvent(el, 'keydown', def);
  if (go) keyEvent(el, 'keypress', def);
  keyEvent(el, 'keyup', def);
  if (go) submitFrom(el);
}

const TRUTHY = new Set(['true', 'on', 'yes', '1', 'checked', 'x']);

/** Inputs whose value is set at once (no caret to type into). */
const DIRECT_VALUE_TYPES = new Set([
  'range',
  'color',
  'date',
  'datetime-local',
  'month',
  'time',
  'week',
]);

/** Set one field's whole value at once (fill). */
export function fillField(el: Element, value: string, what: string): void {
  if (isSelect(el)) {
    selectOptions(el, [value], what);
    return;
  }
  if (isInput(el)) {
    const t = inputType(el);
    if (t === 'checkbox' || t === 'radio') {
      setChecked(el, TRUTHY.has(value.trim().toLowerCase()), what);
      return;
    }
    if (t === 'file') throw new CommandError('unsupported', `${what} is a file input: use upload`);
    if (DIRECT_VALUE_TYPES.has(t)) {
      if (isDisabled(el)) throw new CommandError('unsupported', `${what} is disabled`);
      focusElement(el);
      setNativeValue(el, value);
      fire(el, 'input');
      fire(el, 'change');
      return;
    }
  }
  assertEditable(el, what);
  focusElement(el);
  selectAll(el);
  if (value === '') clearField(el);
  else insertText(el, value);
  if (isTextEntry(el) && el.value !== value && el.localName !== 'input') {
    // A textarea whose page rejected the insertion keeps the native path.
    setNativeValue(el, value);
    fire(el, 'input', value);
  }
  if (isTextEntry(el)) fire(el, 'change');
}

/** Choose options of a native <select> by value, then by visible label. */
export function selectOptions(el: Element, values: readonly string[], what: string): string[] {
  if (!isSelect(el)) {
    throw new CommandError(
      'unsupported',
      `${what} is not a native <select> (${describe(el)}): click it, then click the option`,
    );
  }
  if (isDisabled(el)) throw new CommandError('unsupported', `${what} is disabled`);
  const options = Array.from(el.options);
  const norm = (s: string): string => s.replace(/\s+/g, ' ').trim();
  const chosen: HTMLOptionElement[] = [];
  const missing: string[] = [];
  for (const v of values) {
    const o =
      options.find((x) => x.value === v) ??
      options.find((x) => norm(x.label || x.text) === norm(v)) ??
      options.find((x) => norm(x.label || x.text).toLowerCase() === norm(v).toLowerCase());
    if (o) chosen.push(o);
    else missing.push(v);
  }
  if (missing.length) {
    const available = options
      .slice(0, 20)
      .map((o) => JSON.stringify(norm(o.label || o.text)))
      .join(', ');
    throw new CommandError(
      'not_found',
      `no option ${missing.map((m) => JSON.stringify(m)).join(', ')} in ${what}; it has ${available}`,
    );
  }
  if (!el.multiple && chosen.length > 1) {
    throw new CommandError('unsupported', `${what} takes one value`);
  }
  focusElement(el);
  for (const o of options) o.selected = chosen.includes(o);
  fire(el, 'input');
  fire(el, 'change');
  return chosen.map((o) => o.value);
}

/** Check or uncheck a checkbox/radio (native or ARIA) by clicking it. */
export function setChecked(el: Element, checked: boolean, what: string): void {
  if (isInput(el) && (inputType(el) === 'checkbox' || inputType(el) === 'radio')) {
    if (el.checked === checked && !el.indeterminate) return;
    if (inputType(el) === 'radio' && !checked) {
      throw new CommandError(
        'unsupported',
        `a radio button cannot be unchecked: check another one`,
      );
    }
    if (isDisabled(el)) throw new CommandError('unsupported', `${what} is disabled`);
    click(el);
    if (el.checked !== checked) {
      throw new CommandError(
        'unsupported',
        `the page did not change ${what}: it may ignore synthetic clicks (try trusted: true)`,
      );
    }
    return;
  }
  const role = roleOf(el);
  if (role !== 'checkbox' && role !== 'switch' && role !== 'radio' && role !== 'menuitemcheckbox') {
    throw new CommandError('unsupported', `${what} is not a checkbox (${describe(el)})`);
  }
  if ((el.getAttribute('aria-checked') === 'true') === checked) return;
  click(el);
}

export function ariaChecked(el: Element): boolean | null {
  if (isInput(el)) return el.checked;
  const v = el.getAttribute('aria-checked');
  return v === null ? null : v === 'true';
}

// ------------------------------------------------------------------ press

function focusables(doc: Document): HTMLElement[] {
  const all = Array.from(
    doc.querySelectorAll<HTMLElement>(
      'a[href], button, input, select, textarea, summary, iframe, [tabindex], [contenteditable]',
    ),
  ).filter((e) => isFocusableNow(e) && e.tabIndex >= 0 && e.getClientRects().length > 0);
  const positive = all.filter((e) => e.tabIndex > 0).sort((a, b) => a.tabIndex - b.tabIndex);
  return [...positive, ...all.filter((e) => e.tabIndex === 0)];
}

function moveFocus(doc: Document, from: Element | null, back: boolean): void {
  const list = focusables(doc);
  if (!list.length) return;
  const i = from ? list.indexOf(from as HTMLElement) : -1;
  const next = back ? list[(i <= 0 ? list.length : i) - 1] : list[(i + 1) % list.length];
  next?.focus();
}

function isActivatable(el: Element): boolean {
  const t = tag(el);
  if (t === 'button' || t === 'summary') return true;
  if (t === 'a' && el.hasAttribute('href')) return true;
  if (isInput(el)) {
    const it = inputType(el);
    return ['button', 'submit', 'reset', 'image', 'checkbox', 'radio', 'file', 'color'].includes(
      it,
    );
  }
  const role = el.getAttribute('role');
  return role === 'button' || role === 'link' || role === 'checkbox' || role === 'switch';
}

function scrollPage(doc: Document, dy: number): void {
  const scroller = pageScroller(doc, dy);
  scroller.scrollBy({ top: dy, behavior: 'instant' });
}

/** Press a key on the focused element, with the default actions a page relies on. */
export function press(doc: Document, spec: KeySpec): void {
  const target = deepActiveElement(doc) ?? doc.body ?? doc.documentElement;
  if (!target) return;
  const { def, modifiers } = spec;
  const command =
    modifiers.includes('Control') || modifiers.includes('Meta') || modifiers.includes('Alt');
  const proceed = keyEvent(target, 'keydown', def, spec);
  let activateOnKeyUp = false;
  if (proceed) {
    const printable = def.text !== undefined && def.key !== 'Enter' && !command;
    if (printable) {
      if (keyEvent(target, 'keypress', def, spec)) {
        if (editable(target)) insertText(target, def.text ?? '');
        else if (def.key === ' ' && isActivatable(target)) activateOnKeyUp = true;
        else if (def.key === ' ') scrollPage(doc, (windowOf(doc)?.innerHeight ?? 600) * 0.8);
      }
    } else if (def.key === 'Enter') {
      if (keyEvent(target, 'keypress', def, spec)) {
        if (isTextEntry(target) && target.localName === 'input') submitFrom(target);
        else if (editable(target)) {
          if (!execCommand(target, 'insertLineBreak')) insertText(target, '\n');
        } else if (isActivatable(target)) click(target);
      }
    } else if (def.key === 'Tab') {
      moveFocus(doc, target, modifiers.includes('Shift'));
    } else if (def.key === 'Backspace' && editable(target)) {
      execCommand(target, 'delete');
    } else if (def.key === 'Delete' && editable(target)) {
      execCommand(target, 'forwardDelete');
    } else if (command && def.key.toLowerCase() === 'a' && editable(target)) {
      selectAll(target);
    } else if (!editable(target)) {
      const h = windowOf(doc)?.innerHeight ?? 600;
      const step: Record<string, number> = {
        ArrowDown: 40,
        ArrowUp: -40,
        PageDown: h * 0.8,
        PageUp: -h * 0.8,
        End: 1e9,
        Home: -1e9,
      };
      const dy = step[def.key];
      if (dy !== undefined) scrollPage(doc, dy);
    }
  }
  keyEvent(target, 'keyup', def, spec);
  if (activateOnKeyUp) click(target);
}

// ----------------------------------------------------------------- scroll

function canScroll(el: Element, dx: number, dy: number): boolean {
  const s = styleOf(el);
  const scrollable = (o: string | undefined): boolean =>
    o === 'auto' || o === 'scroll' || o === 'overlay';
  const isRoot = el === el.ownerDocument?.scrollingElement;
  if (dy) {
    if (el.scrollHeight <= el.clientHeight + 1) return false;
    return isRoot || scrollable(s?.overflowY);
  }
  if (dx) {
    if (el.scrollWidth <= el.clientWidth + 1) return false;
    return isRoot || scrollable(s?.overflowX);
  }
  return false;
}

/** The element that scrolls the page: the document, or the app's own scroller. */
export function pageScroller(doc: Document, dy: number, dx = 0): Element {
  const root = doc.scrollingElement ?? doc.documentElement;
  if (canScroll(root, dx, dy)) return root;
  const w = windowOf(doc);
  if (w) {
    let e = doc.elementFromPoint(w.innerWidth / 2, w.innerHeight / 2);
    for (; e; e = parentElementOrHost(e)) if (canScroll(e, dx, dy)) return e;
  }
  let best: Element = root;
  let bestArea = 0;
  for (const el of Array.from(doc.querySelectorAll('*'))) {
    if (!canScroll(el, dx, dy)) continue;
    const area = el.clientWidth * el.clientHeight;
    if (area > bestArea) {
      best = el;
      bestArea = area;
    }
  }
  return best;
}

export function scroll(
  doc: Document,
  el: Element | null,
  direction: string | undefined,
  amount: number,
): { atEnd: boolean } {
  if (el && !direction) {
    el.scrollIntoView({ block: 'center', inline: 'nearest', behavior: 'instant' });
    return { atEnd: false };
  }
  const dir = direction ?? 'down';
  const dx = dir === 'left' ? -amount : dir === 'right' ? amount : 0;
  const dy = dir === 'up' ? -amount : dir === 'down' ? amount : 0;
  if (!dx && !dy) throw new CommandError('unsupported', `unknown direction "${dir}"`);
  let target: Element | null = null;
  if (el) {
    for (let e: Element | null = el; e; e = parentElementOrHost(e)) {
      if (canScroll(e, dx, dy)) {
        target = e;
        break;
      }
    }
  }
  target ??= pageScroller(doc, dy, dx);
  const before = { x: target.scrollLeft, y: target.scrollTop };
  target.scrollBy({ left: dx, top: dy, behavior: 'instant' });
  const moved = target.scrollLeft !== before.x || target.scrollTop !== before.y;
  const atEnd = dy
    ? dy > 0
      ? target.scrollTop + target.clientHeight >= target.scrollHeight - 1
      : target.scrollTop <= 0
    : dx > 0
      ? target.scrollLeft + target.clientWidth >= target.scrollWidth - 1
      : target.scrollLeft <= 0;
  return { atEnd: atEnd || !moved };
}

// -------------------------------------------------------------- highlight

export const HIGHLIGHT_TAG = 'mother-claude-highlight';

/** Outline the element for `ms` so the user sees what is about to be touched. */
export function highlight(el: Element, ms: number): void {
  scrollIntoViewIfNeeded(el);
  const doc = el.ownerDocument;
  if (!doc?.documentElement) return;
  const r = el.getBoundingClientRect();
  const host = doc.createElement(HIGHLIGHT_TAG);
  host.style.cssText =
    'all:initial;position:fixed;inset:0;pointer-events:none;z-index:2147483647;contain:strict;';
  const root = host.attachShadow({ mode: 'closed' });
  const box = doc.createElement('div');
  box.style.cssText = [
    'position:fixed',
    `left:${r.left - 4}px`,
    `top:${r.top - 4}px`,
    `width:${r.width + 8}px`,
    `height:${r.height + 8}px`,
    'box-sizing:border-box',
    'border:3px solid #7c5cc9',
    'border-radius:6px',
    'background:rgba(124,92,201,0.12)',
    'box-shadow:0 0 0 4px rgba(124,92,201,0.25)',
  ].join(';');
  root.append(box);
  doc.documentElement.append(host);
  const w = windowOf(el);
  (w ?? globalThis).setTimeout(() => host.remove(), Math.max(50, Math.min(10_000, ms)));
}

export function isOurs(node: Node): boolean {
  for (let n: Node | null = node; n; n = n.parentNode ?? (n as ShadowRoot).host ?? null) {
    if (isElement(n) && n.localName === HIGHLIGHT_TAG) return true;
  }
  return false;
}

// ----------------------------------------------------------------- upload

export interface UploadFile {
  name: string;
  mimeType?: string;
  data: string;
}

function bytesOf(base64: string): Uint8Array<ArrayBuffer> {
  let raw: string;
  try {
    raw = atob(base64);
  } catch {
    throw new CommandError('unsupported', 'a file is not valid base64');
  }
  const bytes = new Uint8Array(new ArrayBuffer(raw.length));
  for (let i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);
  return bytes;
}

function fileInputFor(el: Element): HTMLInputElement | null {
  if (isInput(el) && inputType(el) === 'file') return el;
  if (tag(el) === 'label') {
    const control = (el as HTMLLabelElement).control;
    if (control && isInput(control) && inputType(control) === 'file') return control;
  }
  const inside = el.querySelector<HTMLInputElement>('input[type="file"]');
  if (inside) return inside;
  const root = shadowRootOf(el);
  return root?.querySelector<HTMLInputElement>('input[type="file"]') ?? null;
}

/**
 * Put files the desktop sent into a file input (or drop them on a drop zone).
 * The bytes come from the command; nothing is read from the disk.
 */
export function upload(el: Element, files: readonly UploadFile[], what: string): number {
  if (!files.length) throw new CommandError('unsupported', 'no files to upload');
  const w = win(el);
  const list = files.map(
    (f) =>
      new w.File([bytesOf(f.data)], f.name || 'file', {
        type: f.mimeType || 'application/octet-stream',
      }),
  );
  const dt = new w.DataTransfer();
  for (const f of list) dt.items.add(f);
  const input = fileInputFor(el);
  if (input) {
    if (isDisabled(input)) throw new CommandError('unsupported', `${what} is disabled`);
    if (!input.multiple && list.length > 1)
      throw new CommandError('unsupported', `${what} takes one file`);
    input.files = dt.files;
    input.dispatchEvent(new w.Event('input', { bubbles: true, composed: true }));
    input.dispatchEvent(new w.Event('change', { bubbles: true }));
    return list.length;
  }
  const p = centerOf(el);
  for (const t of ['dragenter', 'dragover', 'drop']) {
    el.dispatchEvent(
      new w.DragEvent(t, {
        bubbles: true,
        cancelable: true,
        composed: true,
        clientX: p.x,
        clientY: p.y,
        dataTransfer: dt,
      }),
    );
  }
  return list.length;
}
