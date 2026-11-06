// JavaScript dialogs (alert/confirm/prompt/beforeunload) block their tab's
// page until answered. They are learned from the page hook (just before the
// dialog shows) or from the debugger, whichever comes first.

export type DialogKind = 'alert' | 'confirm' | 'prompt' | 'beforeunload';

export interface DialogInfo {
  kind: DialogKind;
  message: string;
  at: number;
}

const KINDS = new Set<string>(['alert', 'confirm', 'prompt', 'beforeunload']);

export function dialogKind(v: unknown): DialogKind {
  return typeof v === 'string' && KINDS.has(v) ? (v as DialogKind) : 'alert';
}

export class Dialogs {
  private readonly open = new Map<number, DialogInfo>();
  private readonly waiters = new Map<number, Set<(d: DialogInfo) => void>>();
  private readonly report: (tabId: number, d: DialogInfo) => void;

  constructor(report: (tabId: number, d: DialogInfo) => void) {
    this.report = report;
  }

  get(tabId: number): DialogInfo | undefined {
    return this.open.get(tabId);
  }

  opened(tabId: number, kind: DialogKind, message: string): void {
    const known = this.open.get(tabId);
    if (known && known.kind === kind && known.message === message) return;
    const d: DialogInfo = { kind, message, at: Date.now() };
    this.open.set(tabId, d);
    this.report(tabId, d);
    for (const w of this.waiters.get(tabId) ?? []) w(d);
  }

  closed(tabId: number): void {
    this.open.delete(tabId);
  }

  /** Resolves when a dialog opens in the tab; `cancel` stops waiting. */
  next(tabId: number): { promise: Promise<DialogInfo>; cancel(): void } {
    let fn!: (d: DialogInfo) => void;
    const promise = new Promise<DialogInfo>((resolve) => (fn = resolve));
    const set = this.waiters.get(tabId) ?? new Set();
    set.add(fn);
    this.waiters.set(tabId, set);
    return {
      promise,
      cancel: () => {
        set.delete(fn);
        if (!set.size) this.waiters.delete(tabId);
      },
    };
  }
}
