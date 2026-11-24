import { Injectable, computed, inject, signal } from '@angular/core';

import { ApiService, describeError } from './api.service';
import { RealtimeService } from './realtime.service';
import { UsageGuardPatch, UsageGuardView, UsagePause } from './usage-guard';

/**
 * The usage guard (Settings › Usage limits) for every screen that shows it:
 * `GET /api/usage-guard`, refetched on each `{t:'usageGuard'}` frame (and
 * after a reconnect) while a screen watches. Its changes answer with the
 * view, which replaces the copy here at once.
 */
@Injectable({ providedIn: 'root' })
export class UsageGuardService {
  private readonly api = inject(ApiService);
  private readonly realtime = inject(RealtimeService);

  readonly view = signal<UsageGuardView | null>(null);
  /** The last load's failure ('' when it worked). */
  readonly error = signal('');
  /** The pauses that hold work now. */
  readonly active = computed<UsagePause[]>(() =>
    (this.view()?.pauses ?? []).filter((p) => !p.ended),
  );
  /** The last few that ended. */
  readonly ended = computed<UsagePause[]>(() =>
    (this.view()?.pauses ?? []).filter((p) => !!p.ended),
  );

  private watchers = 0;
  private loading: Promise<void> | null = null;
  private again = false;

  constructor() {
    this.realtime.usageGuard$.subscribe(() => {
      if (this.watchers > 0) void this.load();
    });
    this.realtime.refetch$.subscribe(() => {
      if (this.watchers > 0) void this.load();
    });
  }

  /** A screen shows it: loads it, and returns its release. */
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

  async setSettings(patch: UsageGuardPatch): Promise<void> {
    this.apply(await this.api.setUsageGuardSettings(patch));
  }

  /** *Resume now*: one pause, or every active one. */
  async resume(pause?: string): Promise<void> {
    this.apply(await this.api.resumeUsagePause(pause));
  }

  /** *Don't resume* (`true`), or let it resume by itself again. */
  async hold(pause: string, hold: boolean): Promise<void> {
    this.apply(await this.api.holdUsagePause(pause, hold));
  }

  /** Forget a pause without resuming its sessions. */
  async dismiss(pause: string): Promise<void> {
    this.apply(await this.api.dismissUsagePause(pause));
  }

  private apply(v: UsageGuardView): void {
    this.view.set(v);
    this.error.set('');
  }

  private async fetch(): Promise<void> {
    try {
      this.apply(await this.api.getUsageGuard());
    } catch (e) {
      this.error.set(describeError(e).text);
    }
  }
}
