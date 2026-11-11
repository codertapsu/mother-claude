import { Injectable, computed, inject, signal } from '@angular/core';

import { ApiService, describeError } from './api.service';
import { SyncPair, SyncView } from './machines';
import { samePath } from './paths';
import { RealtimeService } from './realtime.service';

/**
 * Code sync's projects (docs/PLAN-MACHINES.md §5.15) for every screen that
 * shows them — the Machines page's card, the setup's per-project options:
 * `GET /api/machines/sync`, refetched on each `{t:'machines', what:'sync'}`
 * frame (and when a computer comes and goes, or Machines turns on or off)
 * while a screen watches.
 */
@Injectable({ providedIn: 'root' })
export class SyncService {
  private readonly api = inject(ApiService);
  private readonly realtime = inject(RealtimeService);

  readonly view = signal<SyncView | null>(null);
  /** The last load's failure ('' when it worked). */
  readonly error = signal('');
  readonly pairs = computed(() => this.view()?.pairs ?? []);

  /** Screens showing code sync now. */
  private watchers = 0;
  private loading: Promise<SyncView | null> | null = null;
  private again = false;

  constructor() {
    this.realtime.machines$.subscribe((f) => {
      if (this.watchers > 0 && ['sync', 'peers', 'enabled'].includes(f.what)) {
        void this.load();
      }
    });
    this.realtime.refetch$.subscribe(() => {
      if (this.watchers > 0) void this.load();
    });
  }

  /** A screen shows code sync: loads it, and returns its release. */
  watch(): () => void {
    this.watchers += 1;
    void this.load();
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.watchers -= 1;
    };
  }

  /** `GET /api/machines/sync` (one at a time; a request meanwhile asks
   * again). */
  load(): Promise<SyncView | null> {
    if (this.loading) {
      this.again = true;
      return this.loading;
    }
    this.loading = this.fetch().finally(() => {
      this.loading = null;
      if (this.again) {
        this.again = false;
        void this.load();
      }
    });
    return this.loading;
  }

  private async fetch(): Promise<SyncView | null> {
    try {
      const v = await this.api.getSync();
      this.view.set(v);
      this.error.set('');
      return v;
    } catch (e) {
      this.error.set(describeError(e).text);
      return null;
    }
  }

  /** A mutation's answer is the new list. */
  apply(v: SyncView | null | undefined): void {
    if (v) this.view.set(v);
  }

  /** A mutation's answer is one pair. */
  applyPair(p: SyncPair | null | undefined): void {
    if (!p) return;
    this.view.update((v) => (v ? { ...v, pairs: v.pairs.map((x) => (x.id === p.id ? p : x)) } : v));
  }

  /** The pair of a project folder here. Reactive. */
  pairOf(folder: string | null | undefined): SyncPair | undefined {
    if (!folder) return undefined;
    return this.pairs().find((p) => samePath(p.folder, folder));
  }
}
