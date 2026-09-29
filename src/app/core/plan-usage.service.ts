import { Injectable, computed, inject, signal } from '@angular/core';

import { ApiError, ApiService } from './api.service';
import { NoticeService } from './notice.service';
import { NowService } from './now.service';
import { RealtimeService } from './realtime.service';

/** The server reads the account at most once a minute (it answers 429). */
const MIN_READ_GAP_MS = 60_000;
/** Without live updates, look for the refresh's result this often. */
const FOLLOW_UP_MS = [2_000, 3_000, 5_000, 10_000];

/**
 * The Claude plan's usage for every view that shows it: the data (kept by
 * {@link RealtimeService}: `planUsage` frames, or REST while Limited), a
 * throttled manual refresh and the refresh interval (desktop only).
 *
 * `waitSecs` counts down while a refresh would be refused: after a 429
 * (its Retry-After), and for a minute after a read of the account, which
 * the server won't repeat sooner.
 */
@Injectable({ providedIn: 'root' })
export class PlanUsageService {
  private api = inject(ApiService);
  private realtime = inject(RealtimeService);
  private clock = inject(NowService);
  private notices = inject(NoticeService);

  readonly usage = this.realtime.planUsage;
  /** There is plan usage to show (hide every plan-usage view otherwise). */
  readonly available = computed(() => this.usage()?.available === true);
  /** A refresh request is on its way. */
  readonly requesting = signal(false);
  /** The server is reading the account, or we asked it to. */
  readonly refreshing = computed(() => this.requesting() || this.usage()?.refreshing === true);
  private readonly retryAtS = signal(0);
  /** Seconds before Refresh makes sense again (0: now). */
  readonly waitSecs = computed(() => {
    const u = this.usage();
    let until = this.retryAtS();
    if (u?.source === 'live' && u.fetchedAt) {
      // `fetchedAt` is the Mac's clock: also count from when the numbers
      // got here (stamped on arrival), so a device whose clock lags never
      // waits over a minute.
      const received = this.realtime.planUsageReceivedAt() || u.fetchedAt;
      until = Math.max(until, Math.min(u.fetchedAt, received) + MIN_READ_GAP_MS);
    }
    return Math.max(0, Math.ceil((until - this.clock.now()) / 1000));
  });

  private loading?: Promise<void>;

  /** Fetch the server's copy once when nothing arrived yet (a page opened
   * before the first frame, or while the socket is down). */
  ensureLoaded(): Promise<void> {
    if (this.usage() || this.realtime.connection() === 'open') return Promise.resolve();
    this.loading ??= this.load().finally(() => (this.loading = undefined));
    return this.loading;
  }

  /** Ask the server for a fresh reading now. */
  async refresh(): Promise<void> {
    if (this.requesting() || this.waitSecs() > 0) return;
    this.requesting.set(true);
    const mark = this.realtime.planUsageMark();
    try {
      const u = await this.api.refreshPlanUsage();
      if (u) this.realtime.mergePlanUsage(u, mark);
      // Without the socket no frame brings the result: look for it.
      if (this.realtime.connection() !== 'open') void this.followUp();
    } catch (e) {
      if (e instanceof ApiError && e.status === 429) {
        this.retryAtS.set(this.clock.now() + (e.retryAfter ?? 60) * 1000);
      } else {
        this.notices.error(e, undefined, 'Plan usage');
      }
    } finally {
      this.requesting.set(false);
    }
  }

  /** Desktop only: how often the server reads while a session is active. */
  async setIntervalSecs(secs: number): Promise<boolean> {
    const mark = this.realtime.planUsageMark();
    try {
      const u = await this.api.setPlanUsageSettings(secs);
      if (u && !this.realtime.mergePlanUsage(u, mark)) {
        // A frame came first; the interval is all this answer confirms (its
        // next read time predates the rescheduling: frames bring that).
        this.realtime.patchPlanUsage({ intervalSecs: u.intervalSecs });
      } else if (!u) {
        this.realtime.patchPlanUsage({ intervalSecs: secs });
      }
      return true;
    } catch (e) {
      this.notices.error(e, undefined, 'Plan usage refresh');
      return false;
    }
  }

  private async load(): Promise<void> {
    const mark = this.realtime.planUsageMark();
    try {
      this.realtime.mergePlanUsage(await this.api.getPlanUsage(), mark);
    } catch {
      // Not reachable, or an older server without plan usage: stay hidden.
    }
  }

  private async followUp(): Promise<void> {
    for (const wait of FOLLOW_UP_MS) {
      await new Promise((r) => setTimeout(r, wait));
      if (this.realtime.connection() === 'open') return;
      await this.load();
      if (!this.usage()?.refreshing) return;
    }
  }
}
