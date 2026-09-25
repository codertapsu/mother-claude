import { Signal, computed, signal } from '@angular/core';
import { Subscription } from 'rxjs';

import type { ApiService } from './api.service';
import { errorMessage } from './api.service';
import { UiEvent } from './models';
import type { RealtimeService } from './realtime.service';

export interface TranscriptStoreOptions {
  /** Show one subagent's transcript instead of the main conversation. */
  agentId?: string;
  /** Events per REST page (server max 300). Default 150. */
  limit?: number;
  /** Also refetch the tail on this interval while the page is visible
   * (agent transcripts, which the server may not push over the socket).
   * Change it later with `setPoll()`. */
  pollMs?: number;
}

const DEFAULT_LIMIT = 150;

/**
 * One session's (or one agent's) transcript as a signal, kept complete across
 * reconnects. Create it with `RealtimeService.openTranscript()`.
 *
 * - Loads the newest page over REST, then appends live `transcript` frames
 *   (buffering frames that race the first fetch). Everything is deduped by
 *   `UiEvent.id`; an event re-sent with a known id replaces the old copy.
 * - Main-conversation stores take events WITHOUT `agentId`; agent stores take
 *   only events whose `agentId` matches.
 * - On `refetch$` (reconnect / resync / resume / online) it refetches the
 *   tail over REST and merges it; if the gap is bigger than one page it
 *   replaces the list (older history is then paged in with `loadEarlier()`).
 * - `destroy()` releases the socket subscription and timers.
 */
export class TranscriptStore {
  private readonly eventsS = signal<UiEvent[]>([]);
  private readonly loadingS = signal(true);
  private readonly loadingEarlierS = signal(false);
  private readonly errorS = signal('');
  private readonly beforeS = signal<string | undefined>(undefined);

  /** Newest-last, deduped. */
  readonly events: Signal<UiEvent[]> = this.eventsS.asReadonly();
  readonly loading = this.loadingS.asReadonly();
  readonly loadingEarlier = this.loadingEarlierS.asReadonly();
  readonly error = this.errorS.asReadonly();
  readonly hasEarlier = computed(() => !!this.beforeS());

  private readonly ids = new Set<string>();
  private buffer: UiEvent[] | null = [];
  private readonly release: () => void;
  private readonly subs = new Subscription();
  private poll?: ReturnType<typeof setInterval>;
  private destroyed = false;
  private refreshing = false;
  private readonly limit: number;

  constructor(
    private readonly api: ApiService,
    private readonly realtime: RealtimeService,
    readonly sessionId: string,
    private readonly opts: TranscriptStoreOptions = {},
  ) {
    this.limit = Math.min(300, opts.limit ?? DEFAULT_LIMIT);
    this.release = realtime.subscribe(sessionId);
    this.subs.add(
      realtime.transcript$.subscribe((f) => {
        if (f.session !== sessionId) return;
        const mine = (f.events ?? []).filter((e) => this.accepts(e));
        if (!mine.length) return;
        if (this.buffer) this.buffer.push(...mine);
        else this.merge(mine);
      }),
    );
    this.subs.add(realtime.refetch$.subscribe(() => void this.refresh()));
    this.setPoll(opts.pollMs ?? 0);
    void this.initialLoad();
  }

  /** Refetch the tail every `ms` while the page is visible (0 = off). */
  setPoll(ms: number): void {
    clearInterval(this.poll);
    this.poll = undefined;
    if (this.destroyed || !(ms > 0)) return;
    this.poll = setInterval(() => {
      if (document.visibilityState === 'visible') void this.refresh();
    }, ms);
  }

  /** Page older events in front of the current list. */
  async loadEarlier(): Promise<void> {
    const before = this.beforeS();
    if (!before || this.loadingEarlierS() || this.destroyed) return;
    this.loadingEarlierS.set(true);
    try {
      const page = await this.api.getTranscript(this.sessionId, {
        before,
        limit: this.limit,
        agent: this.opts.agentId,
      });
      if (this.destroyed) return;
      const older = (page.events ?? []).filter((e) => this.accepts(e) && !this.ids.has(e.id));
      for (const e of older) this.ids.add(e.id);
      if (older.length) this.eventsS.update((prev) => [...older, ...prev]);
      this.beforeS.set(page.before);
    } catch (e) {
      if (!this.destroyed) this.errorS.set(errorMessage(e));
    } finally {
      this.loadingEarlierS.set(false);
    }
  }

  /** Refetch the newest page and merge it (also runs on reconnects). */
  async refresh(): Promise<void> {
    if (this.destroyed || this.refreshing || this.buffer) return;
    this.refreshing = true;
    try {
      const page = await this.api.getTranscript(this.sessionId, {
        limit: this.limit,
        agent: this.opts.agentId,
      });
      if (this.destroyed) return;
      const fresh = dedupe((page.events ?? []).filter((e) => this.accepts(e)));
      const prev = this.eventsS();
      if (!fresh.length && prev.length) return; // nothing new to trust
      const overlaps = fresh.some((e) => this.ids.has(e.id));
      if (overlaps) {
        this.eventsS.set(spliceTail(prev, fresh));
      } else {
        // Empty so far, or we missed more than a page: start over from the
        // newest page (older history pages back in with loadEarlier()).
        this.eventsS.set(fresh);
        this.beforeS.set(page.before);
      }
      this.ids.clear();
      for (const e of this.eventsS()) this.ids.add(e.id);
      this.noteTail(page.tail);
      this.errorS.set('');
    } catch (e) {
      if (!this.destroyed) this.errorS.set(errorMessage(e));
    } finally {
      this.refreshing = false;
    }
  }

  destroy(): void {
    if (this.destroyed) return;
    this.destroyed = true;
    this.subs.unsubscribe();
    this.release();
    clearInterval(this.poll);
  }

  private async initialLoad(): Promise<void> {
    try {
      const page = await this.api.getTranscript(this.sessionId, {
        limit: this.limit,
        agent: this.opts.agentId,
      });
      if (this.destroyed) return;
      const events = (page.events ?? []).filter((e) => this.accepts(e));
      for (const e of events) this.ids.add(e.id);
      this.eventsS.set(dedupe(events));
      this.beforeS.set(page.before);
      this.noteTail(page.tail);
    } catch (e) {
      if (!this.destroyed) this.errorS.set(errorMessage(e));
    } finally {
      const buffered = this.buffer ?? [];
      this.buffer = null;
      if (!this.destroyed) {
        this.merge(buffered);
        this.loadingS.set(false);
      }
    }
  }

  private noteTail(tail: string | undefined): void {
    if (!this.opts.agentId) this.realtime.noteTail(this.sessionId, tail);
  }

  private accepts(e: UiEvent): boolean {
    if (!e || typeof e.id !== 'string') return false;
    return this.opts.agentId ? e.agentId === this.opts.agentId : !e.agentId;
  }

  /** Append unknown events; replace known ones in place. */
  private merge(events: UiEvent[]): void {
    if (!events.length) return;
    const replace = new Map<string, UiEvent>();
    const append: UiEvent[] = [];
    for (const e of events) {
      if (this.ids.has(e.id)) replace.set(e.id, e);
      else {
        this.ids.add(e.id);
        append.push(e);
      }
    }
    this.eventsS.update((prev) => {
      const updated = replace.size ? prev.map((e) => replace.get(e.id) ?? e) : prev;
      return append.length ? [...updated, ...append] : updated;
    });
  }
}

/** Replace the tail of `prev` with the authoritative page `fresh` (which
 * overlaps it), keeping any newer live events the page doesn't have yet. */
export function spliceTail(prev: UiEvent[], fresh: UiEvent[]): UiEvent[] {
  const freshIds = new Set(fresh.map((e) => e.id));
  const start = prev.findIndex((e) => e.id === fresh[0]?.id);
  if (start >= 0) {
    // [older…, page…, live events the page doesn't have yet]
    const newer = prev.slice(start).filter((e) => !freshIds.has(e.id));
    return [...prev.slice(0, start), ...fresh, ...newer];
  }
  // The page reaches further back than what we hold: it wins.
  return [...fresh, ...prev.filter((e) => !freshIds.has(e.id))];
}

function dedupe(events: UiEvent[]): UiEvent[] {
  const seen = new Set<string>();
  const out: UiEvent[] = [];
  for (const e of events) {
    if (seen.has(e.id)) continue;
    seen.add(e.id);
    out.push(e);
  }
  return out;
}
