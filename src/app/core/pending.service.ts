import { Injectable, Signal, computed, inject, signal } from '@angular/core';

import { ApiError, ApiService } from './api.service';
import {
  PendingItem,
  PendingOutcome,
  PendingPreview,
  PendingResolvedBy,
  RespondBody,
  SessionDetail,
} from './models';
import { RealtimeService } from './realtime.service';

/** How and by whom a pending item was settled. */
export interface PendingResolution {
  id: string;
  session: string;
  outcome: PendingOutcome;
  by?: PendingResolvedBy;
  at: number;
  /** Settled by this very client. */
  mine: boolean;
}

/** What `respond()` came back with. */
export interface RespondResult {
  /** Someone else answered first (409/410); `text` says who, if known. */
  alreadyDecided: boolean;
  text: string;
}

/** How long a resolution is remembered (cards show it, then collapse). */
const KEEP_RESOLVED_MS = 60_000;

/** A full item carries what previews never do. */
export function isFullItem(p: PendingPreview | PendingItem): p is PendingItem {
  return Array.isArray((p as PendingItem).suggestions) && typeof p.sessionId === 'string';
}

/** "Allowed on Marcus's iPhone", "Answered at the Mac", "Expired". */
export function resolutionText(r: PendingResolution): string {
  const verb: Record<PendingOutcome, string> = {
    allowed: 'Allowed',
    denied: 'Denied',
    answered: 'Answered',
    withdrawn: 'No longer needed',
    expired: 'Expired',
    cancelled: 'Cancelled',
  };
  const base = verb[r.outcome] ?? 'Settled';
  if (r.mine) return `${base} from here`;
  const by = r.by;
  if (!by) return r.outcome === 'withdrawn' ? `${base} (answered elsewhere)` : base;
  switch (by.kind) {
    case 'device':
      return `${base} on ${by.name || 'another device'}`;
    case 'desktop':
      return `${base} on the ${by.name || 'Mac'}`;
    case 'host':
      return r.outcome === 'withdrawn'
        ? `Answered at the Mac${by.name ? ` (${by.name})` : ''}`
        : `${base} at the Mac${by.name ? ` (${by.name})` : ''}`;
    case 'timeout':
      return 'Timed out';
    default:
      return base;
  }
}

/**
 * Pending items beyond the list previews the sessions store carries:
 *
 * - **Full items** (complete `input`, questions, plan, suggestions) from
 *   `pending{item}` frames, `GET /inbox` and `GET /sessions/{id}`
 *   (`pendingDetail`). `ensure()` loads one on demand.
 * - **Resolutions** from `pendingResolved` frames (and our own answers), so a
 *   card can say "Answered on the Mac" instead of vanishing.
 * - **In-flight answers**: one registry across the Inbox, the session page
 *   and the list, so the same item can't be submitted twice.
 */
@Injectable({ providedIn: 'root' })
export class PendingService {
  private api = inject(ApiService);
  private realtime = inject(RealtimeService);

  private readonly fullS = signal<ReadonlyMap<string, PendingItem>>(new Map());
  private readonly resolvedS = signal<ReadonlyMap<string, PendingResolution>>(new Map());
  private readonly inflightS = signal<ReadonlySet<string>>(new Set());
  readonly inflight = this.inflightS.asReadonly();
  private readonly loading = new Map<string, Promise<PendingItem | undefined>>();

  constructor() {
    this.realtime.pending$.subscribe((f) => this.put([f.item]));
    this.realtime.pendingResolved$.subscribe((f) =>
      this.noteResolved({
        id: f.id,
        session: f.session,
        outcome: f.outcome,
        by: f.by,
        at: f.at || Date.now(),
        mine: false,
      }),
    );
    // Forget full copies once the item left every preview list.
    this.realtime.sessions$.subscribe(() => {
      const live = new Set(this.realtime.pendingItems().map((p) => p.id));
      const full = this.fullS();
      if ([...full.keys()].some((id) => !live.has(id))) {
        this.fullS.set(new Map([...full].filter(([id]) => live.has(id))));
      }
      // Older servers embed full items in the summary: take them.
      const embedded = this.realtime
        .sessions()
        .flatMap((s) => s.pending ?? [])
        .filter((p): p is PendingItem => isFullItem(p) && !full.has(p.id));
      if (embedded.length) this.put(embedded);
    });
  }

  /** The full item for `id`, when known. */
  item(id: string): Signal<PendingItem | undefined> {
    return computed(() => this.fullS().get(id));
  }

  get(id: string): PendingItem | undefined {
    return this.fullS().get(id);
  }

  /** How `id` was settled, if it was (kept for a minute). */
  resolution(id: string): Signal<PendingResolution | undefined> {
    return computed(() => this.resolvedS().get(id));
  }

  /** Reactive read (call inside a computed/template). */
  resolutionOf(id: string): PendingResolution | undefined {
    return this.resolvedS().get(id);
  }

  isInflight(id: string): Signal<boolean> {
    return computed(() => this.inflightS().has(id));
  }

  put(items: PendingItem[]): void {
    const valid = items.filter((i) => i?.id);
    if (!valid.length) return;
    this.fullS.update((prev) => {
      const next = new Map(prev);
      for (const i of valid) {
        const old = next.get(i.id);
        // Never replace a complete input with a truncated one.
        if (old && !old.inputTruncated && i.inputTruncated) continue;
        next.set(i.id, i);
      }
      return next;
    });
  }

  /** Take the full items a detail response carries. */
  putFromDetail(d: SessionDetail): void {
    const items = (d.pendingDetail ?? []).map((p) => ({ ...p, sessionId: p.sessionId || d.id }));
    const embedded = (d.pending ?? []).filter(isFullItem);
    this.put([...embedded, ...items]);
  }

  /** Load the full item (from the session detail, then the inbox). */
  ensure(sessionId: string, id: string): Promise<PendingItem | undefined> {
    const have = this.get(id);
    if (have && !have.inputTruncated) return Promise.resolve(have);
    const running = this.loading.get(id);
    if (running) return running;
    const p = (async () => {
      try {
        const d = await this.api.getSession(sessionId);
        this.putFromDetail(d);
        if (this.get(id)) return this.get(id);
        const inbox = await this.api.getInbox();
        this.put(inbox.items ?? []);
        return this.get(id);
      } finally {
        this.loading.delete(id);
      }
    })();
    this.loading.set(id, p);
    return p;
  }

  /** `GET /inbox`: every full item (the Inbox page and the polling mode). */
  async loadInbox(): Promise<PendingItem[]> {
    const inbox = await this.api.getInbox();
    const items = inbox.items ?? [];
    this.put(items);
    return items;
  }

  /**
   * Answer an item. Resolves with `alreadyDecided` when someone else was
   * first (409/410); rejects with the ApiError otherwise. A second call for
   * the same item while one is in flight is ignored (resolves null).
   */
  async respond(
    ref: { id: string; sessionId: string },
    body: Omit<RespondBody, 'requestId'>,
    doneText: string,
  ): Promise<RespondResult | null> {
    if (this.inflightS().has(ref.id)) return null;
    this.inflightS.update((prev) => new Set(prev).add(ref.id));
    try {
      await this.api.respond(ref.sessionId, { requestId: ref.id, ...body });
      this.noteResolved({
        id: ref.id,
        session: ref.sessionId,
        outcome: body.answers ? 'answered' : body.decision === 'allow' ? 'allowed' : 'denied',
        at: Date.now(),
        mine: true,
      });
      return { alreadyDecided: false, text: doneText };
    } catch (e) {
      if (e instanceof ApiError && (e.status === 409 || e.status === 410)) {
        const known = this.resolvedS().get(ref.id);
        return {
          alreadyDecided: true,
          text: known ? resolutionText(known) : e.message || 'Already answered elsewhere',
        };
      }
      throw e;
    } finally {
      this.inflightS.update((prev) => {
        const next = new Set(prev);
        next.delete(ref.id);
        return next;
      });
    }
  }

  private noteResolved(r: PendingResolution): void {
    this.resolvedS.update((prev) => {
      const now = Date.now();
      const next = new Map([...prev].filter(([, v]) => now - v.at < KEEP_RESOLVED_MS));
      // Our own answer wins over the echo of it.
      if (!(next.get(r.id)?.mine && !r.mine)) next.set(r.id, r);
      return next;
    });
  }
}
