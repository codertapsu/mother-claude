import { Injectable, computed, inject, signal } from '@angular/core';

import { ApiService } from './api.service';
import { tr } from './i18n/i18n';
import { NoticeService } from './notice.service';
import { PinsView, pinKey } from './pins';
import { RealtimeService } from './realtime.service';

/**
 * The pinned sessions (`GET /api/pins`) for every screen that shows them:
 * loaded on first use, refetched on each `{t:'pins'}` frame and after a
 * reconnect. Pinning answers with the new view, which replaces the copy
 * here at once.
 */
@Injectable({ providedIn: 'root' })
export class PinsService {
  private readonly api = inject(ApiService);
  private readonly realtime = inject(RealtimeService);
  private readonly notices = inject(NoticeService);

  readonly view = signal<PinsView | null>(null);
  /** When each pinned session was pinned, by `pinKey`. */
  readonly pinned = computed<ReadonlyMap<string, number>>(
    () => new Map((this.view()?.pins ?? []).map((p) => [pinKey(p.id, p.machine), p.at])),
  );
  /** A pin or unpin on its way (by `pinKey`). */
  readonly busy = signal<ReadonlySet<string>>(new Set());

  private started = false;
  private loading: Promise<void> | null = null;
  private again = false;

  constructor() {
    this.realtime.pins$.subscribe(() => {
      if (this.started) void this.load();
    });
    this.realtime.refetch$.subscribe(() => {
      if (this.started) void this.load();
    });
  }

  /** Load once; frames keep it current from then on. */
  ensureLoaded(): void {
    if (this.started) return;
    this.started = true;
    void this.load();
  }

  isPinned(id: string, machine?: string | null): boolean {
    return this.pinned().has(pinKey(id, machine));
  }

  /** When session `id` was pinned (undefined: it is not). */
  pinnedAt(id: string, machine?: string | null): number | undefined {
    return this.pinned().get(pinKey(id, machine));
  }

  /** Pin it, or unpin it; says so. `name`: the session's, for the notice. */
  async toggle(id: string, machine: string | null | undefined, name: string): Promise<void> {
    const key = pinKey(id, machine);
    if (this.busy().has(key)) return;
    const pin = !this.isPinned(id, machine);
    this.busy.update((b) => new Set(b).add(key));
    try {
      this.view.set(
        pin ? await this.api.pinSession(id, machine) : await this.api.unpinSession(id, machine),
      );
      this.notices.success(
        pin
          ? tr('{name} is pinned to the top of Sessions.', { name })
          : tr('{name} is unpinned.', { name }),
      );
    } catch (e) {
      this.notices.error(e, undefined, pin ? tr('Pin to the top') : tr('Unpin'));
    } finally {
      this.busy.update((b) => {
        const next = new Set(b);
        next.delete(key);
        return next;
      });
    }
  }

  /** The view (one load at a time; a request meanwhile asks again). */
  load(): Promise<void> {
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

  private async fetch(): Promise<void> {
    try {
      this.view.set(await this.api.getPins());
    } catch {
      // An older server without pins, or offline: nothing is pinned here.
    }
  }
}
