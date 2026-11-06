// The snapshot: a compact text tree of the page's accessibility roles, one
// node per line, two spaces of indent per level, interactive nodes carrying a
// ref (docs/BROWSER-PROTOCOL.md, "Snapshot format"):
//
//   - main
//     - heading "New issue" [level=1]
//     - textbox "Title" [ref=e3] value=""
//     - button "Submit new issue" [ref=e5] [disabled]
//
// Generic elements (div, span…) get no line: their children join the parent.
// Text that no name already tells becomes `- text "…"`. Every quoted string
// is truncated to 120 characters.

import { accessibleName } from './accname.ts';
import {
  collapse,
  deepActiveElement,
  deepQueryAll,
  frameDocument,
  inputType,
  isBlock,
  isElement,
  isHtml,
  isIframe,
  isInput,
  isSelect,
  isText,
  renderedChildren,
  tag,
  truncate,
  visibilityOf,
  windowOf,
} from './dom.ts';
import { REDACTED, isSecretField } from './redact.ts';
import type { RefRegistry } from './refs.ts';
import { CONTEXT_ROLES, LEAF_TEXT_ROLES, headingLevel, isInteractive, roleOf } from './roles.ts';

export const NAME_MAX = 120;
export const DEFAULT_MAX_NODES = 1500;
/** Options of a <select> listed under it when there are at most this many. */
const MAX_LISTED_OPTIONS = 25;
/** Hard ceiling on elements visited, whatever the page. */
const MAX_VISITED = 200_000;

export interface SnapshotOptions {
  scope?: 'viewport' | 'page';
  selector?: string;
  interactiveOnly?: boolean;
  maxNodes?: number;
  /** Epoch ms after which the walk stops and reports `truncated`. */
  deadline?: number;
}

export interface SnapshotResult {
  snapshot: string;
  truncated: boolean;
  nodes: number;
}

export interface SNode {
  role: string;
  name?: string;
  /** The name (or text) is replaced by «redacted». */
  redacted?: boolean;
  ref?: string;
  states: string[];
  /** Form control value; null renders «redacted». */
  value?: string | null;
  suffix?: string;
  children: SNode[];
  /** Listed under a select, never pruned. */
  options?: SNode[];
  interactive: boolean;
  isText?: boolean;
  inViewport?: boolean;
  el?: Element;
}

/** Roles that are table/grid structure: their content shows as children, not as a name. */
const STRUCTURAL_CONTENT_ROLES = new Set(['row', 'cell', 'gridcell', 'columnheader', 'rowheader']);

/** Roles whose own text is already told (name or value): no text lines below them. */
const NO_TEXT_ROLES = new Set([
  ...LEAF_TEXT_ROLES,
  'textbox',
  'searchbox',
  'combobox',
  'listbox',
  'slider',
  'spinbutton',
  'progressbar',
  'meter',
  'img',
  'separator',
  'scrollbar',
]);

/** Roles that show their only text in place of a name (not landmarks or containers). */
const FOLDABLE = new Set([
  'paragraph',
  'listitem',
  'cell',
  'gridcell',
  'columnheader',
  'rowheader',
  'term',
  'definition',
  'status',
  'alert',
  'note',
  'log',
  'timer',
  'marquee',
  'blockquote',
  'caption',
  'code',
  'emphasis',
  'strong',
  'time',
  'mark',
  'deletion',
  'insertion',
  'subscript',
  'superscript',
  'generic',
  'article',
  'figure',
]);

/** Elements whose children are never walked. */
const OPAQUE_TAGS = new Set([
  'textarea',
  'select',
  'svg',
  'canvas',
  'video',
  'audio',
  'object',
  'embed',
  'math',
  'datalist',
  'option',
  'optgroup',
  'picture',
  'img',
  'input',
]);

interface Ctx {
  /** Pending text of the current level. */
  text: string[];
  collectText: boolean;
  /** Below a leaf role (button, link…): only interactive descendants get lines. */
  interactiveBelow: boolean;
  redacted: boolean;
  invisible: boolean;
  frame: number;
  textInViewport: boolean;
}

export class SnapshotBuilder {
  private visited = 0;
  stoppedEarly = false;
  private readonly top: Document;
  private readonly refs: RefRegistry;
  private readonly opts: SnapshotOptions;
  private readonly active: Element | null;
  private readonly interactiveOnly: boolean;
  private readonly viewport: boolean;

  constructor(top: Document, refs: RefRegistry, opts: SnapshotOptions = {}) {
    this.top = top;
    this.refs = refs;
    this.opts = opts;
    this.active = deepActiveElement(top);
    this.interactiveOnly = opts.interactiveOnly === true;
    this.viewport = opts.scope === 'viewport';
  }

  /** The nodes for a whole document. */
  buildDocument(): SNode[] {
    const root = this.top.body ?? this.top.documentElement;
    const out: SNode[] = [];
    if (!root) return out;
    const ctx = this.rootCtx(0);
    this.childrenOf(root, out, ctx);
    this.flush(out, ctx);
    return this.finish(out);
  }

  /** The nodes for one element and what is inside it. */
  buildElement(el: Element): SNode[] {
    const out: SNode[] = [];
    const doc = el.ownerDocument;
    const ctx = this.rootCtx(doc ? this.refs.frameOf(doc, this.top) : 0);
    this.element(el, out, ctx);
    this.flush(out, ctx);
    return this.finish(out);
  }

  private rootCtx(frame: number): Ctx {
    return {
      text: [],
      collectText: !this.interactiveOnly,
      interactiveBelow: false,
      redacted: false,
      invisible: false,
      frame,
      textInViewport: true,
    };
  }

  private finish(nodes: SNode[]): SNode[] {
    let out = nodes;
    if (this.interactiveOnly || this.viewport)
      out = prune(out, this.interactiveOnly, this.viewport);
    return collapseWrappers(out);
  }

  private stop(): boolean {
    if (this.stoppedEarly) return true;
    if (this.visited > MAX_VISITED) this.stoppedEarly = true;
    else if (this.opts.deadline && this.visited % 256 === 0 && Date.now() > this.opts.deadline)
      this.stoppedEarly = true;
    return this.stoppedEarly;
  }

  private childrenOf(parent: Node, out: SNode[], ctx: Ctx): void {
    let kids: Node[];
    if (isElement(parent) && tag(parent) === 'details' && !(parent as HTMLDetailsElement).open) {
      // Only the summary of a closed <details> is rendered.
      kids = Array.from(parent.children).filter((c) => tag(c) === 'summary');
    } else {
      kids = renderedChildren(parent);
    }
    for (const child of kids) {
      if (this.stop()) return;
      if (isText(child)) {
        this.text(child, ctx);
        continue;
      }
      if (isElement(child)) this.element(child, out, ctx);
    }
  }

  private text(node: Text, ctx: Ctx): void {
    if (!ctx.collectText || ctx.invisible) return;
    const data = node.data;
    if (!data.trim()) {
      if (data.length) ctx.text.push(' ');
      return;
    }
    if (this.viewport && !textInViewport(node)) return;
    if (ctx.redacted) {
      if (ctx.text[ctx.text.length - 1] !== REDACTED) ctx.text.push(REDACTED);
      return;
    }
    ctx.text.push(data);
  }

  private flush(out: SNode[], ctx: Ctx): void {
    if (!ctx.text.length) return;
    const joined = collapse(ctx.text.join('')).trim();
    ctx.text.length = 0;
    if (!joined) return;
    out.push({
      role: 'text',
      name: joined,
      ...(joined === REDACTED ? { redacted: true } : {}),
      states: [],
      children: [],
      interactive: false,
      isText: true,
      inViewport: true,
    });
  }

  private emits(role: string, ctx: Ctx): boolean {
    if (ctx.interactiveBelow) return role === 'iframe';
    if (this.interactiveOnly) return CONTEXT_ROLES.has(role);
    return true;
  }

  private element(el: Element, out: SNode[], ctx: Ctx): void {
    this.visited++;
    const vis = visibilityOf(el, { ariaHidden: true });
    if (vis === 'hidden') return;
    const t = tag(el);
    if (t === 'br') {
      if (ctx.collectText) ctx.text.push(' ');
      return;
    }
    if (isIframe(el)) {
      this.flush(out, ctx);
      this.frame(el, out, ctx);
      return;
    }
    const role = roleOf(el);
    const interactive = vis === 'visible' && isInteractive(el, role);
    const redacted = ctx.redacted || el.hasAttribute('data-mc-redact');
    const emit = interactive || (role !== null && this.emits(role, ctx));

    if (!emit) {
      if (OPAQUE_TAGS.has(t)) return;
      const block = ctx.collectText && isBlock(el);
      if (block) this.flush(out, ctx);
      const prevRedacted = ctx.redacted;
      const prevInvisible = ctx.invisible;
      const prevCollect = ctx.collectText;
      ctx.redacted = redacted;
      ctx.invisible = vis === 'invisible';
      // A label's text is its control's name already: say it once.
      if (namesSomething(el)) ctx.collectText = false;
      this.childrenOf(el, out, ctx);
      ctx.redacted = prevRedacted;
      ctx.invisible = prevInvisible;
      ctx.collectText = prevCollect;
      if (block) this.flush(out, ctx);
      return;
    }

    this.flush(out, ctx);
    const node = this.makeNode(el, role ?? 'generic', interactive, redacted, ctx);
    const leaf = ctx.interactiveBelow || (role !== null && NO_TEXT_ROLES.has(role));
    const childCtx: Ctx = {
      text: [],
      collectText: ctx.collectText && !leaf,
      interactiveBelow: leaf,
      redacted,
      invisible: vis === 'invisible',
      frame: ctx.frame,
      textInViewport: true,
    };
    if (!OPAQUE_TAGS.has(t)) {
      this.childrenOf(el, node.children, childCtx);
      this.flush(node.children, childCtx);
    }
    // A node whose only content is text shows it in place of a name.
    const only = node.children.length === 1 ? node.children[0] : undefined;
    if (only?.isText && !node.name && FOLDABLE.has(node.role)) {
      node.name = only.name ?? '';
      if (redacted) node.redacted = true;
      node.children = [];
    }
    if (node.redacted === undefined && redacted && node.name) node.redacted = true;
    out.push(node);
  }

  private makeNode(
    el: Element,
    role: string,
    interactive: boolean,
    redacted: boolean,
    ctx: Ctx,
  ): SNode {
    const node: SNode = { role, states: [], children: [], interactive, el };
    // Rows and cells show their content as children; a name from content would
    // repeat it all (and make interactiveOnly bigger than the full snapshot).
    const contentNameless = STRUCTURAL_CONTENT_ROLES.has(role);
    const name = contentNameless ? explicitName(el) : accessibleName(el, role);
    if (name) node.name = name;
    if (redacted && name) node.redacted = true;
    if (interactive) node.ref = this.refs.refFor(el, ctx.frame);
    node.states = statesOf(el, role, interactive && el === this.active);
    const value = valueOf(el, role);
    if (value !== undefined) node.value = redacted || isSecretField(el) ? null : value;
    if (isSelect(el)) node.options = optionsOf(el);
    if (this.viewport) node.inViewport = elementInViewport(el);
    return node;
  }

  private frame(el: HTMLIFrameElement, out: SNode[], ctx: Ctx): void {
    const name = accessibleName(el, 'iframe');
    const doc = frameDocument(el);
    const node: SNode = { role: 'iframe', states: [], children: [], interactive: false, el };
    if (name) node.name = name;
    if (this.viewport) node.inViewport = elementInViewport(el);
    if (!doc) {
      node.suffix = '(cross-origin, not read)';
      out.push(node);
      return;
    }
    const frame = this.refs.frameOf(doc, this.top);
    const body = doc.body ?? doc.documentElement;
    if (body) {
      const sub: Ctx = { ...this.rootCtx(frame), redacted: ctx.redacted };
      this.childrenOf(body, node.children, sub);
      this.flush(node.children, sub);
    }
    out.push(node);
  }
}

/** Text that is another element's accessible name: a label, legend, caption. */
function namesSomething(el: Element): boolean {
  const t = tag(el);
  if (t === 'label') return !!(el as HTMLLabelElement).control;
  const parent = el.parentElement ? tag(el.parentElement) : '';
  return (
    (t === 'legend' && parent === 'fieldset') ||
    (t === 'caption' && parent === 'table') ||
    (t === 'figcaption' && parent === 'figure')
  );
}

/** aria-labelledby / aria-label / title only (no content). */
function explicitName(el: Element): string {
  const label = el.getAttribute('aria-label')?.trim();
  if (label) return label;
  if (el.getAttribute('aria-labelledby')) return accessibleName(el, 'region');
  return el.getAttribute('title')?.trim() ?? '';
}

const CHECKABLE = new Set(['checkbox', 'radio', 'switch', 'menuitemcheckbox', 'menuitemradio']);
const SELECTABLE = new Set(['option', 'tab', 'treeitem', 'gridcell', 'row', 'cell']);

function statesOf(el: Element, role: string, focused: boolean): string[] {
  const states: string[] = [];
  if (role === 'heading') states.push(`level=${headingLevel(el) ?? 2}`);
  if (CHECKABLE.has(role)) {
    if (isInput(el)) {
      if (el.indeterminate) states.push('checked=mixed');
      else if (el.checked) states.push('checked');
    } else {
      const c = el.getAttribute('aria-checked');
      if (c === 'true') states.push('checked');
      else if (c === 'mixed') states.push('checked=mixed');
    }
  }
  if (SELECTABLE.has(role)) {
    if (
      tag(el) === 'option'
        ? (el as HTMLOptionElement).selected
        : el.getAttribute('aria-selected') === 'true'
    )
      states.push('selected');
  }
  const pressed = el.getAttribute('aria-pressed');
  if (pressed === 'true') states.push('pressed');
  else if (pressed === 'mixed') states.push('pressed=mixed');
  const expanded = expandedOf(el);
  if (expanded === true) states.push('expanded');
  else if (expanded === false) states.push('expanded=false');
  if (isDisabled(el)) states.push('disabled');
  if ((el as HTMLInputElement).required === true || el.getAttribute('aria-required') === 'true')
    states.push('required');
  if (isReadOnly(el)) states.push('readonly');
  const invalid = el.getAttribute('aria-invalid');
  if (invalid && invalid !== 'false') states.push('invalid');
  if (focused) states.push('focused');
  return states;
}

function expandedOf(el: Element): boolean | null {
  const attr = el.getAttribute('aria-expanded');
  if (attr === 'true') return true;
  if (attr === 'false') return false;
  if (tag(el) === 'summary' && el.parentElement && tag(el.parentElement) === 'details')
    return (el.parentElement as HTMLDetailsElement).open;
  return null;
}

export function isDisabled(el: Element): boolean {
  if (el.getAttribute('aria-disabled') === 'true') return true;
  if (!isHtml(el)) return false;
  try {
    return el.matches(':disabled');
  } catch {
    return (el as HTMLButtonElement).disabled === true;
  }
}

function isReadOnly(el: Element): boolean {
  if (el.getAttribute('aria-readonly') === 'true') return true;
  if (isInput(el) || tag(el) === 'textarea') return (el as HTMLInputElement).readOnly === true;
  return false;
}

const VALUE_ROLES = new Set([
  'textbox',
  'searchbox',
  'combobox',
  'listbox',
  'spinbutton',
  'slider',
]);

/** The value shown for a form control, or undefined when it has none. */
export function valueOf(el: Element, role: string): string | undefined {
  if (isInput(el)) {
    const type = inputType(el);
    if (type === 'checkbox' || type === 'radio') return undefined;
    if (type === 'button' || type === 'submit' || type === 'reset' || type === 'image')
      return undefined;
    if (type === 'file')
      return Array.from(el.files ?? [])
        .map((f) => f.name)
        .join(', ');
    return el.value;
  }
  if (tag(el) === 'textarea') return (el as HTMLTextAreaElement).value;
  if (isSelect(el)) {
    return Array.from(el.selectedOptions)
      .map((o) => collapse(o.label || o.text).trim())
      .join(', ');
  }
  if (!VALUE_ROLES.has(role)) return undefined;
  if (role === 'slider' || role === 'spinbutton') {
    return el.getAttribute('aria-valuetext') ?? el.getAttribute('aria-valuenow') ?? undefined;
  }
  if (role === 'textbox' || role === 'searchbox') return collapse(el.textContent ?? '').trim();
  return undefined;
}

function optionsOf(select: HTMLSelectElement): SNode[] | undefined {
  const options = Array.from(select.options);
  if (!options.length || options.length > MAX_LISTED_OPTIONS) return undefined;
  return options.map((o) => {
    const node: SNode = {
      role: 'option',
      name: collapse(o.label || o.text).trim(),
      states: [],
      children: [],
      interactive: false,
    };
    if (o.selected) node.states.push('selected');
    if (o.disabled) node.states.push('disabled');
    return node;
  });
}

function rectInViewport(r: DOMRect, w: Window): boolean {
  if (r.width === 0 && r.height === 0) return false;
  return r.bottom > 0 && r.right > 0 && r.top < w.innerHeight && r.left < w.innerWidth;
}

/** Does the element intersect the viewport (and its frames' viewports)? */
export function elementInViewport(el: Element): boolean {
  const w = windowOf(el);
  if (!w) return false;
  if (!rectInViewport(el.getBoundingClientRect(), w)) return false;
  const frame = w.frameElement;
  return frame ? elementInViewport(frame) : true;
}

function textInViewport(node: Text): boolean {
  const doc = node.ownerDocument;
  const w = windowOf(node);
  if (!doc || !w) return false;
  const range = doc.createRange();
  range.selectNodeContents(node);
  const r = range.getBoundingClientRect();
  if (!rectInViewport(r, w)) return false;
  const frame = w.frameElement;
  return frame ? elementInViewport(frame) : true;
}

function keepNode(n: SNode, interactiveOnly: boolean, viewport: boolean): boolean {
  if (viewport && !n.inViewport) return false;
  if (interactiveOnly) return n.interactive || n.suffix !== undefined;
  return n.interactive || n.isText === true || !!n.name || n.suffix !== undefined;
}

/** Drop what the options leave out; keep a container when something inside it stays. */
function prune(nodes: SNode[], interactiveOnly: boolean, viewport: boolean): SNode[] {
  const out: SNode[] = [];
  for (const n of nodes) {
    n.children = prune(n.children, interactiveOnly, viewport);
    if (keepNode(n, interactiveOnly, viewport) || n.children.length > 0) {
      if (
        !keepNode(n, interactiveOnly, viewport) &&
        interactiveOnly &&
        !CONTEXT_ROLES.has(n.role)
      ) {
        out.push(...n.children);
        continue;
      }
      out.push(n);
    }
  }
  return out;
}

/** A nameless list item or group around a single node adds a line and says nothing. */
function collapseWrappers(nodes: SNode[]): SNode[] {
  const out: SNode[] = [];
  for (const n of nodes) {
    n.children = collapseWrappers(n.children);
    const only = n.children.length === 1 ? n.children[0] : undefined;
    if (
      only &&
      (n.role === 'listitem' || n.role === 'group') &&
      !n.name &&
      !n.ref &&
      n.states.length === 0 &&
      !only.isText
    ) {
      out.push(only);
      continue;
    }
    out.push(n);
  }
  return out;
}

function quote(s: string): string {
  return JSON.stringify(truncate(s, NAME_MAX));
}

export function renderLine(n: SNode, depth: number): string {
  let s = `${'  '.repeat(depth)}- ${n.role}`;
  if (n.name) s += ` ${n.redacted ? REDACTED : quote(n.name)}`;
  if (n.ref) s += ` [ref=${n.ref}]`;
  for (const st of n.states) s += ` [${st}]`;
  if (n.value !== undefined) s += ` value=${n.value === null ? REDACTED : quote(n.value)}`;
  if (n.suffix) s += ` ${n.suffix}`;
  return s;
}

/** Render nodes as the snapshot text, stopping at maxNodes lines. */
export function render(
  nodes: SNode[],
  maxNodes: number = DEFAULT_MAX_NODES,
): { text: string; count: number; truncated: boolean } {
  const lines: string[] = [];
  let truncated = false;
  const walk = (list: SNode[], depth: number): void => {
    for (const n of list) {
      if (lines.length >= maxNodes) {
        truncated = true;
        return;
      }
      lines.push(renderLine(n, depth));
      if (n.options) walk(n.options, depth + 1);
      walk(n.children, depth + 1);
      if (truncated) return;
    }
  };
  walk(nodes, 0);
  return { text: lines.join('\n'), count: lines.length, truncated };
}

/** Build and render a snapshot of a document. */
export function snapshot(
  doc: Document,
  refs: RefRegistry,
  opts: SnapshotOptions = {},
): SnapshotResult {
  const builder = new SnapshotBuilder(doc, refs, opts);
  let nodes: SNode[];
  if (opts.selector) {
    const root = deepQueryAll(doc, opts.selector, 1)[0];
    if (!root) throw new SelectorNotFound(opts.selector);
    nodes = builder.buildElement(root);
  } else {
    nodes = builder.buildDocument();
  }
  const max = clampMaxNodes(opts.maxNodes);
  const { text, count, truncated } = render(nodes, max);
  refs.prune();
  return { snapshot: text, truncated: truncated || builder.stoppedEarly, nodes: count };
}

export function clampMaxNodes(v: unknown): number {
  if (typeof v !== 'number' || !Number.isFinite(v)) return DEFAULT_MAX_NODES;
  return Math.max(1, Math.min(20_000, Math.floor(v)));
}

export class SelectorNotFound extends Error {
  readonly selector: string;
  constructor(selector: string) {
    super(`nothing matches ${selector}`);
    this.name = 'SelectorNotFound';
    this.selector = selector;
  }
}
