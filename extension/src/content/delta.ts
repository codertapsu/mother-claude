// "A fresh snapshot of what changed, when cheap": after an action, the page's
// DOM changes are collected for a short settle window and the few subtrees
// that changed are rendered like a snapshot. Too many changes — or a page
// that rebuilt itself — means no delta at all; the model takes a snapshot.

import { containsDeep, isElement, parentElementOrHost, tag, windowOf } from './dom.ts';
import type { RefRegistry } from './refs.ts';
import { isInteractive, roleOf } from './roles.ts';
import { SnapshotBuilder, render } from './snapshot.ts';

const ATTRIBUTES = [
  'class',
  'style',
  'hidden',
  'open',
  'disabled',
  'checked',
  'selected',
  'value',
  'href',
  'aria-expanded',
  'aria-hidden',
  'aria-selected',
  'aria-checked',
  'aria-pressed',
  'aria-disabled',
  'aria-invalid',
  'aria-busy',
  'aria-current',
  'aria-label',
];

const MAX_TARGETS = 300;
const MAX_ROOTS = 5;
const MAX_LINES = 150;

export class ChangeTracker {
  private readonly targets = new Set<Node>();
  private readonly observer: MutationObserver | null;
  private readonly ignore: (n: Node) => boolean;
  private lastMutation = 0;
  private overflow = false;

  constructor(doc: Document, ignore: (n: Node) => boolean) {
    this.ignore = ignore;
    const w = windowOf(doc);
    this.observer = w
      ? new w.MutationObserver((records) => {
          for (const r of records) {
            if (this.ignore(r.target)) continue;
            if (r.type === 'childList') {
              const ours = [...Array.from(r.addedNodes), ...Array.from(r.removedNodes)].every((n) =>
                this.ignore(n),
              );
              if (ours && (r.addedNodes.length || r.removedNodes.length)) continue;
            }
            this.lastMutation = Date.now();
            if (this.targets.size >= MAX_TARGETS) this.overflow = true;
            else this.targets.add(r.target);
          }
        })
      : null;
    this.observer?.observe(doc, {
      subtree: true,
      childList: true,
      characterData: true,
      attributes: true,
      attributeFilter: ATTRIBUTES,
    });
  }

  /** Wait until the DOM has been quiet for `quietMs`, at most `maxMs` (or until cancelled). */
  async settle(
    quietMs: number,
    maxMs: number,
    deadline: number,
    signal?: AbortSignal,
  ): Promise<void> {
    const start = Date.now();
    const end = Math.min(deadline, start + maxMs);
    while (Date.now() < end && !signal?.aborted) {
      await new Promise((r) => setTimeout(r, 25));
      const since = Date.now() - (this.lastMutation || start);
      if (since >= quietMs) return;
    }
  }

  stop(): { targets: Node[]; overflow: boolean } {
    this.observer?.disconnect();
    return { targets: [...this.targets], overflow: this.overflow };
  }
}

function anchorOf(el: Element): Element | null {
  for (let e: Element | null = el; e; e = parentElementOrHost(e)) {
    const t = tag(e);
    if (t === 'body' || t === 'html') return null;
    const role = roleOf(e);
    if (role || isInteractive(e, role)) return e;
  }
  return null;
}

function topMost(list: Element[]): Element[] {
  return list.filter((e) => !list.some((o) => o !== e && containsDeep(o, e)));
}

/** The snapshot lines for what changed, or undefined when there is nothing cheap to say. */
export function deltaSnapshot(
  top: Document,
  refs: RefRegistry,
  changes: { targets: Node[]; overflow: boolean },
  interactiveOnly: boolean,
): string | undefined {
  if (changes.overflow || !changes.targets.length) return undefined;
  const elements = changes.targets
    .map((n) => (isElement(n) ? n : n.parentElement))
    .filter((e): e is Element => !!e && e.isConnected);
  const anchors: Element[] = [];
  for (const el of topMost([...new Set(elements)])) {
    const a = anchorOf(el);
    if (!a) return undefined; // the page itself changed: not cheap
    if (!anchors.includes(a)) anchors.push(a);
  }
  const roots = topMost(anchors);
  if (!roots.length || roots.length > MAX_ROOTS) return undefined;
  const builder = new SnapshotBuilder(top, refs, { interactiveOnly });
  const nodes = roots.flatMap((r) => builder.buildElement(r));
  const { text, truncated } = render(nodes, MAX_LINES);
  if (truncated || !text) return undefined;
  return text;
}
