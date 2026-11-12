import { Injectable, computed, inject, signal } from '@angular/core';

import { ApiService, describeError } from './api.service';
import { CopiesView, HeldCopy } from './copies';
import { RealtimeService } from './realtime.service';

/**
 * Copies kept current between computers (docs/PLAN-MACHINES.md §5.9) for
 * every screen that shows them — the Machines page's card, the *Lives on …*
 * banner's *Continue here from the copy of 14:32*: `GET
 * /api/machines/copies`, refetched on each `{t:'machines', what:'copies'}`
 * frame (and when a computer comes and goes) while a screen watches.
 */
@Injectable({ providedIn: 'root' })
export class CopiesService {
  private readonly api = inject(ApiService);
  private readonly realtime = inject(RealtimeService);

  readonly view = signal<CopiesView | null>(null);
  /** The last load's failure ('' when it worked). */
  readonly error = signal('');
  readonly held = computed(() => this.view()?.held ?? []);
  readonly kept = computed(() => this.view()?.kept ?? []);

  private watchers = 0;
  private loading: Promise<CopiesView | null> | null = null;
  private again = false;

  constructor() {
    this.realtime.machines$.subscribe((f) => {
      if (this.watchers > 0 && ['copies', 'peers', 'enabled', 'settings'].includes(f.what)) {
        void this.load();
      }
    });
    this.realtime.refetch$.subscribe(() => {
      if (this.watchers > 0) void this.load();
    });
  }

  /** A screen shows copies: loads them, and returns its release. */
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

  /** `GET /api/machines/copies` (one at a time; a request meanwhile asks
   * again). */
  load(): Promise<CopiesView | null> {
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

  private async fetch(): Promise<CopiesView | null> {
    try {
      const v = await this.api.getCopies();
      this.view.set(v);
      this.error.set('');
      return v;
    } catch (e) {
      this.error.set(describeError(e).text);
      return null;
    }
  }

  /** A mutation's answer is the new view. */
  apply(v: CopiesView | null | undefined): void {
    if (v) this.view.set(v);
  }

  /** The copy of `session` this computer holds for `machine` (any computer
   * when null). Reactive. */
  heldOf(session: string, machine: string | null = null): HeldCopy | undefined {
    return this.held().find(
      (h) => h.session === session && (machine === null || h.machine === machine),
    );
  }
}
