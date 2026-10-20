import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';
import { RouterLink } from '@angular/router';

import { DEFAULT_ACCOUNT } from '../core/accounts';
import { AuthService } from '../core/auth.service';
import { NowService } from '../core/now.service';
import { planIntervalLabel } from '../core/plan-usage';
import { PlanUsageService } from '../core/plan-usage.service';
import { RealtimeService } from '../core/realtime.service';
import { IconComponent } from './icon.component';
import { PlanMeterComponent } from './plan-meter.component';
import { formatCountdown, formatMinor, relativeTime } from './util';

/**
 * The Claude plan's usage in full: every window the server reports (with
 * its meter and reset time), extra usage, how fresh the numbers are and when
 * the next check is due, and a Refresh button that counts down while the
 * server would refuse. On the desktop it links to the refresh interval in
 * Settings; phones show the interval read-only. Renders nothing until the
 * server has reported plan usage. `account` picks whose usage (the Default
 * account's unless it names another).
 */
@Component({
  selector: 'mc-plan-usage',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, IconComponent, PlanMeterComponent],
  template: `
    @if (usage(); as u) {
      @if (!u.available) {
        <p class="muted small">
          This Claude account has no plan usage to show (an API key or a cloud provider has no plan
          limits).
        </p>
      } @else {
        <div class="meters">
          @for (w of u.windows; track w.key) {
            <mc-plan-meter [window]="w" />
          } @empty {
            <p class="muted small">No usage has been read yet.</p>
          }
        </div>

        @if (extra(); as x) {
          <div class="extra">
            <span class="xl">Extra usage</span>
            <span class="xv">{{ x }}</span>
          </div>
        }

        @if (u.error) {
          <p class="notice warn small" role="status">Last check failed: {{ u.error }}</p>
        }

        <div class="status">
          <span class="muted small">{{ status() }}</span>
          <button
            class="btn small"
            type="button"
            [disabled]="refreshing() || waitSecs() > 0"
            (click)="plan.refresh(account())"
          >
            <mc-icon name="refresh" [size]="14" />
            {{
              refreshing()
                ? 'Checking…'
                : waitSecs() > 0
                  ? 'Refresh in ' + waitSecs() + 's'
                  : 'Refresh'
            }}
          </button>
        </div>

        @if (settingsLink()) {
          <p class="muted small interval">
            {{ interval() }}
            @if (isDesktop()) {
              <a routerLink="/settings">Change in Settings</a>
            }
          </p>
        }
      }
    }
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.85rem;
      }
      .meters {
        display: flex;
        flex-direction: column;
        gap: 0.85rem;
      }
      .extra {
        display: flex;
        justify-content: space-between;
        gap: 0.75rem;
        padding-top: 0.6rem;
        border-top: 1px solid var(--border);
        font-size: 0.875rem;
      }
      .xl {
        color: var(--muted);
      }
      .xv {
        text-align: right;
        font-variant-numeric: tabular-nums;
      }
      .status {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        justify-content: space-between;
        gap: 0.5rem 0.75rem;
      }
      .notice,
      .interval {
        margin: 0;
      }
    `,
  ],
})
export class PlanUsageComponent {
  protected readonly plan = inject(PlanUsageService);
  private clock = inject(NowService);
  private realtime = inject(RealtimeService);
  protected readonly isDesktop = inject(AuthService).isDesktop;

  /** Say how often it checks (and link to Settings on the desktop). */
  readonly settingsLink = input(true);
  /** Whose plan usage: an account id (the Default's by default). */
  readonly account = input<string>(DEFAULT_ACCOUNT);

  protected readonly usage = computed(() => this.plan.usageOf(this.account()));
  protected readonly refreshing = computed(() => this.plan.isRefreshing(this.account()));
  protected readonly waitSecs = computed(() => this.plan.waitSecsOf(this.account()));

  protected readonly extra = computed(() => {
    const x = this.usage()?.extraUsage;
    if (!x) return '';
    if (!x.enabled) return 'Off';
    const used = formatMinor(x.usedMinor, x.currency, x.decimals);
    if (x.limitMinor === undefined) return `${used} used`;
    const limit = formatMinor(x.limitMinor, x.currency, x.decimals);
    const pct = x.utilization !== undefined ? ` (${Math.round(x.utilization)}%)` : '';
    return `${used} of ${limit}${pct}`;
  });

  protected readonly status = computed(() => {
    const u = this.usage();
    if (!u) return '';
    const now = this.clock.now();
    const ago = (ms: number) => relativeTime(ms, now);
    let updated: string;
    if (!u.fetchedAt) {
      updated = 'Not read yet';
    } else if (u.streamAt && u.streamAt > u.fetchedAt) {
      // A running session moved the 5-hour and weekly windows since.
      const rest = u.source === 'cache' ? "Claude Code's last reading" : 'the rest';
      updated = `Updated ${ago(u.streamAt)} by a running session (${rest} ${ago(u.fetchedAt)})`;
    } else {
      const from = u.source === 'stream' ? ' by a running session' : '';
      const cached = u.source === 'cache' ? " (Claude Code's last reading)" : '';
      updated = `Updated ${ago(u.fetchedAt)}${from}${cached}`;
    }
    let next: string;
    if (this.refreshing()) next = 'checking now…';
    else if (u.intervalSecs <= 0) next = 'automatic checks are off';
    else if (!u.nextRefreshAt) {
      // No read scheduled: the Mac reads only while a dashboard is connected
      // live. Connected, this is a moment before it schedules one (an older
      // server also leaves it out while nothing is active, and reads every
      // 30 min, or up to an hour after a failure).
      if (this.realtime.connection() !== 'open')
        next = 'automatic checks wait for a live connection';
      else if (u.error) next = 'next try within an hour';
      else next = 'next check within 30 min, sooner when a session is active';
    } else if (u.nextRefreshAt <= now) next = 'next check soon';
    else next = `next check in ${formatCountdown(u.nextRefreshAt - now)}`;
    return `${updated} · ${next}`;
  });

  protected readonly interval = computed(() => {
    const secs = this.usage()?.intervalSecs ?? 0;
    return secs > 0
      ? `Checks ${planIntervalLabel(secs).toLowerCase()} while a session is active, and every 30 min otherwise.`
      : 'Automatic checks are off: it is read when you tap Refresh, and by a session started here when a dashboard connects or a limit is hit.';
  });
}
