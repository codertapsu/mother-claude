import { ChangeDetectionStrategy, Component, computed, inject, output } from '@angular/core';
import { RouterLink } from '@angular/router';

import { FleetSummary, SessionSummary } from '../core/models';
import { NowService } from '../core/now.service';
import { RealtimeService } from '../core/realtime.service';
import { IconComponent } from './icon.component';
import { formatTokens, formatUsd } from './util';

/** The fleet summary with the server's rules (`GET /agents` `summary`), from
 * the active session store, plus what the cost fallback needs. */
export interface FleetCounts extends Pick<FleetSummary, 'working' | 'agentsRunning' | 'needYou'> {
  costToday: number;
  /** Sessions active since midnight. */
  todaySessions: number;
  /** Some session active today has a known price. */
  priced: boolean;
  /** Input + output tokens of today's sessions (shown when none is priced). */
  tokensToday: number;
}

/**
 * Same rules as the server's fleet summary:
 * - working: live sessions whose status is `working`;
 * - agentsRunning: running subagents and workflow agents (shells and
 *   monitors are background work, not agents);
 * - needYou: sessions with at least one request answerable from here;
 * - costToday: `costUsd` of sessions whose last activity is after midnight.
 */
export function fleetCounts(sessions: readonly SessionSummary[], midnight: number): FleetCounts {
  const out: FleetCounts = {
    working: 0,
    agentsRunning: 0,
    needYou: 0,
    costToday: 0,
    todaySessions: 0,
    priced: false,
    tokensToday: 0,
  };
  for (const s of sessions) {
    if (s.live && s.status === 'working') out.working += 1;
    if ((s.pending ?? []).some((p) => p.answerable)) out.needYou += 1;
    out.agentsRunning += s.agents?.running ?? 0;
    if ((s.lastActivityAt ?? 0) >= midnight) {
      out.todaySessions += 1;
      if (typeof s.costUsd === 'number') {
        out.priced = true;
        out.costToday += s.costUsd;
      }
      out.tokensToday += (s.usage?.input ?? 0) + (s.usage?.output ?? 0);
    }
  }
  out.costToday = Math.round(out.costToday * 100) / 100;
  return out;
}

/**
 * "3 working · 7 agents running · 2 need you · $4.12 today" across every
 * session. Each part is a shortcut (a list filter, the Agents page). Cost is
 * the sum for sessions active since local midnight, falling back to tokens
 * when no session's price is known. Counted like the server's fleet summary
 * ({@link fleetCounts}), so the phone and the fleet agree.
 */
@Component({
  selector: 'mc-fleet-summary',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, IconComponent],
  template: `
    <div class="fleet" role="group" aria-label="Right now">
      <button type="button" class="seg" (click)="filter.emit('live')">
        <mc-icon name="activity" [size]="14" />
        <strong>{{ working() }}</strong> working
      </button>
      <a class="seg" routerLink="/agents">
        <mc-icon name="agents" [size]="14" />
        <strong>{{ agents() }}</strong> {{ agents() === 1 ? 'agent' : 'agents' }} running
      </a>
      <button type="button" class="seg" [class.warn]="needs() > 0" (click)="filter.emit('needs')">
        <mc-icon name="hand" [size]="14" />
        <strong>{{ needs() }}</strong> {{ needs() === 1 ? 'needs' : 'need' }} you
      </button>
      <span class="seg static" [title]="costTitle()">
        <mc-icon name="clock" [size]="14" />
        <strong>{{ today() }}</strong> today
      </span>
    </div>
  `,
  styles: [
    `
      .fleet {
        display: flex;
        flex-wrap: wrap;
        gap: 0.35rem;
        margin-bottom: 0.75rem;
      }
      .seg {
        appearance: none;
        display: inline-flex;
        align-items: center;
        gap: 0.35rem;
        min-height: 36px;
        padding: 0.2rem 0.7rem;
        border-radius: 1.2rem;
        border: 1px solid var(--border);
        background: var(--panel);
        color: var(--muted);
        font: inherit;
        font-size: 0.8rem;
        text-decoration: none;
        cursor: pointer;
        white-space: nowrap;
      }
      .seg strong {
        color: var(--text);
        font-variant-numeric: tabular-nums;
      }
      .seg.static {
        cursor: default;
      }
      .seg.warn {
        border-color: var(--amber-border);
        background: var(--amber-soft);
      }
      .seg.warn strong,
      .seg.warn mc-icon {
        color: var(--amber);
      }
      button.seg:hover,
      a.seg:hover {
        border-color: var(--accent);
      }
    `,
  ],
})
export class FleetSummaryComponent {
  private realtime = inject(RealtimeService);
  private clock = inject(NowService);

  readonly filter = output<'live' | 'needs'>();

  private readonly counts = computed(() => {
    const midnight = new Date(this.clock.now());
    midnight.setHours(0, 0, 0, 0);
    return fleetCounts(this.realtime.sessions(), midnight.getTime());
  });
  protected readonly working = computed(() => this.counts().working);
  protected readonly agents = computed(() => this.counts().agentsRunning);
  protected readonly needs = computed(() => this.counts().needYou);
  protected readonly today = computed(() => {
    const c = this.counts();
    return c.priced ? formatUsd(c.costToday) : `${formatTokens(c.tokensToday)} tokens`;
  });
  protected readonly costTitle = computed(
    () => `Across ${this.counts().todaySessions} sessions active since midnight (estimated)`,
  );
}
