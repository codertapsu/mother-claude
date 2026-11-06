// Element references. A ref names one element for as long as that element is
// in the page: the same element keeps its ref across snapshots, and a ref is
// never reused for another element in the same tab — the service worker hands
// each new document the next free number, so a ref from before a navigation
// can only come back as `stale_ref`, never as some other element.
//
// Refs of same-origin iframes carry the frame's number: `f1e3`.

export type Resolved =
  { ok: true; el: Element } | { ok: false; code: 'stale_ref' | 'not_found'; message: string };

const REF_RE = /^(?:f(\d+))?e(\d+)$/;

export class RefRegistry {
  private readonly byRef = new Map<string, WeakRef<Element>>();
  private readonly byElement = new WeakMap<Element, string>();
  private readonly frameIndex = new WeakMap<Document, number>();
  private nextRef = 1;
  private nextFrame = 1;

  /** Continue numbering from the tab's counters (a new document). */
  continueFrom(nextRef: number, nextFrame: number): void {
    if (Number.isInteger(nextRef) && nextRef > this.nextRef) this.nextRef = nextRef;
    if (Number.isInteger(nextFrame) && nextFrame > this.nextFrame) this.nextFrame = nextFrame;
  }

  get counters(): { nextRef: number; nextFrame: number } {
    return { nextRef: this.nextRef, nextFrame: this.nextFrame };
  }

  /** The number of a same-origin frame's document (0 for the top document). */
  frameOf(doc: Document, top: Document): number {
    if (doc === top) return 0;
    let n = this.frameIndex.get(doc);
    if (n === undefined) {
      n = this.nextFrame++;
      this.frameIndex.set(doc, n);
    }
    return n;
  }

  refFor(el: Element, frame: number): string {
    const existing = this.byElement.get(el);
    if (existing) {
      // Back in the page after a prune (re-parented): the same ref again.
      if (!this.byRef.has(existing)) this.byRef.set(existing, new WeakRef(el));
      return existing;
    }
    const ref = `${frame ? `f${frame}` : ''}e${this.nextRef++}`;
    this.byElement.set(el, ref);
    this.byRef.set(ref, new WeakRef(el));
    return ref;
  }

  resolve(ref: string): Resolved {
    const trimmed = ref.trim();
    if (!REF_RE.test(trimmed)) {
      return {
        ok: false,
        code: 'not_found',
        message: `"${ref}" is not a ref: use one from a snapshot`,
      };
    }
    const weak = this.byRef.get(trimmed);
    const el = weak?.deref();
    if (!el || !el.isConnected) {
      if (weak) this.byRef.delete(trimmed);
      return { ok: false, code: 'stale_ref', message: `${trimmed} is gone: take a new snapshot` };
    }
    return { ok: true, el };
  }

  /** Forget refs whose elements left the page. */
  prune(): void {
    for (const [ref, weak] of this.byRef) {
      const el = weak.deref();
      if (!el || !el.isConnected) this.byRef.delete(ref);
    }
  }
}
