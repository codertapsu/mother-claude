import { Injectable, computed, inject, signal } from '@angular/core';

import { DEFAULT_ACCOUNT } from './accounts';
import { AccountsService } from './accounts.service';
import { ApiError, ApiService } from './api.service';
import { PlanUsage } from './models';
import { NoticeService } from './notice.service';
import { NowService } from './now.service';
import { storageGet, storageSet } from './platform';
import { RealtimeService } from './realtime.service';

/** The server reads an account at most once a minute (it answers 429). */
const MIN_READ_GAP_MS = 60_000;
/** Without live updates, look for the refresh's result this often. */
const FOLLOW_UP_MS = [2_000, 3_000, 5_000, 10_000];
/** The account the compact meter shows (an account id: no secret). */
const METER_KEY = 'mc_plan_account';

/**
 * The Claude plan's usage for every view that shows it: the data (kept by
 * {@link RealtimeService}: `planUsage` frames for the Default account,
 * `accountUsage` frames for the others, or REST while Limited), a throttled
 * manual refresh per account and the refresh interval (desktop only).
 *
 * `usage`, `available`, `refreshing`, `waitSecs` and `refresh()` are the
 * Default account's, as before accounts; the `…Of(account)` forms take any
 * account. `meterAccount` is the account the compact meter shows (picked
 * with its switcher, remembered on this device; the Default otherwise).
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
  private accounts = inject(AccountsService);

  readonly usage = this.realtime.planUsage;
  /** There is plan usage to show (hide every plan-usage view otherwise). */
  readonly available = computed(() => this.usage()?.available === true);
  private readonly requestingS = signal<ReadonlySet<string>>(new Set());
  /** A refresh request (the Default's) is on its way. */
  readonly requesting = computed(() => this.requestingS().has(DEFAULT_ACCOUNT));
  /** The server is reading the account, or we asked it to. */
  readonly refreshing = computed(() => this.isRefreshing(DEFAULT_ACCOUNT));
  private readonly retryAtS = signal<ReadonlyMap<string, number>>(new Map());
  /** Seconds before Refresh makes sense again (0: now). */
  readonly waitSecs = computed(() => this.waitSecsOf(DEFAULT_ACCOUNT));

  private readonly meterS = signal(storageGet(METER_KEY) || DEFAULT_ACCOUNT);
  /** The account the compact meter shows: the one picked while it exists
   * (and more than one does), else the Default. */
  readonly meterAccount = computed(() => {
    const id = this.meterS();
    return this.accounts.multi() && this.accounts.byId(id) ? id : DEFAULT_ACCOUNT;
  });

  private loading?: Promise<void>;

  /** Show this account's usage in the compact meter. */
  selectMeter(id: string): void {
    this.meterS.set(id);
    storageSet(METER_KEY, id);
  }

  usageOf(account: string = DEFAULT_ACCOUNT): PlanUsage | null {
    return account === DEFAULT_ACCOUNT
      ? this.realtime.planUsage()
      : (this.realtime.accountUsage().get(account) ?? null);
  }

  availableOf(account: string = DEFAULT_ACCOUNT): boolean {
    return this.usageOf(account)?.available === true;
  }

  isRefreshing(account: string = DEFAULT_ACCOUNT): boolean {
    return this.requestingS().has(account) || this.usageOf(account)?.refreshing === true;
  }

  waitSecsOf(account: string = DEFAULT_ACCOUNT): number {
    const u = this.usageOf(account);
    let until = this.retryAtS().get(account) ?? 0;
    if (u?.source === 'live' && u.fetchedAt) {
      // `fetchedAt` is the Mac's clock: also count from when the numbers
      // got here (stamped on arrival), so a device whose clock lags never
      // waits over a minute.
      const at =
        account === DEFAULT_ACCOUNT
          ? this.realtime.planUsageReceivedAt()
          : (this.realtime.accountUsageReceivedAt().get(account) ?? 0);
      const received = at || u.fetchedAt;
      until = Math.max(until, Math.min(u.fetchedAt, received) + MIN_READ_GAP_MS);
    }
    return Math.max(0, Math.ceil((until - this.clock.now()) / 1000));
  }

  /** Fetch the server's copy once when nothing arrived yet (a page opened
   * before the first frame, or while the socket is down). */
  ensureLoaded(): Promise<void> {
    if (this.usage() || this.realtime.connection() === 'open') return Promise.resolve();
    this.loading ??= this.load().finally(() => (this.loading = undefined));
    return this.loading;
  }

  /** Ask the server for a fresh reading of an account now. */
  async refresh(account: string = DEFAULT_ACCOUNT): Promise<void> {
    if (this.requestingS().has(account) || this.waitSecsOf(account) > 0) return;
    this.setRequesting(account, true);
    const mark = this.realtime.planUsageMark();
    const accountMark = this.realtime.accountUsageMark();
    try {
      const u = await this.api.refreshPlanUsage(account === DEFAULT_ACCOUNT ? undefined : account);
      if (u && account === DEFAULT_ACCOUNT) this.realtime.mergePlanUsage(u, mark);
      else if (u) this.realtime.mergeAccountUsages({ accounts: { [account]: u } }, accountMark);
      // Without the socket no frame brings the result: look for it.
      if (this.realtime.connection() !== 'open') void this.followUp(account);
    } catch (e) {
      if (e instanceof ApiError && e.status === 429) {
        const at = this.clock.now() + (e.retryAfter ?? 60) * 1000;
        this.retryAtS.update((prev) => new Map(prev).set(account, at));
      } else {
        this.notices.error(e, undefined, 'Plan usage');
      }
    } finally {
      this.setRequesting(account, false);
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

  private setRequesting(account: string, on: boolean): void {
    this.requestingS.update((prev) => {
      const next = new Set(prev);
      if (on) next.add(account);
      else next.delete(account);
      return next;
    });
  }

  private async load(): Promise<void> {
    const mark = this.realtime.planUsageMark();
    const accountMark = this.realtime.accountUsageMark();
    try {
      const reply = await this.api.getPlanUsage();
      this.realtime.mergePlanUsage(reply, mark);
      this.realtime.mergeAccountUsages(reply, accountMark);
    } catch {
      // Not reachable, or an older server without plan usage: stay hidden.
    }
  }

  private async followUp(account: string): Promise<void> {
    for (const wait of FOLLOW_UP_MS) {
      await new Promise((r) => setTimeout(r, wait));
      if (this.realtime.connection() === 'open') return;
      await this.load();
      if (!this.usageOf(account)?.refreshing) return;
    }
  }
}
