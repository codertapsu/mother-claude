import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  output,
  signal,
} from '@angular/core';
import { RouterLink } from '@angular/router';

import { FleetSummary, PlanUsage, SessionSummary } from '../core/models';
import { NowService } from '../core/now.service';
import { compactWindows, windowPct, windowSeverity, worstSeverity } from '../core/plan-usage';
import { PlanUsageService } from '../core/plan-usage.service';
import { RealtimeService } from '../core/realtime.service';
import { IconComponent } from './icon.component';
import { PlanUsageSheetComponent } from './plan-usage-sheet.component';
import { Tone, formatCountdown, formatTokens, formatUsd, resetsIn, severityTone } from './util';

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
 * - costToday: the sum of each session's `costToday` (what was priced since
 *   local midnight, not its lifetime `costUsd`).
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
    out.costToday += s.costToday ?? 0;
    if ((s.lastActivityAt ?? 0) >= midnight) {
      out.todaySessions += 1;
      if (typeof s.costUsd === 'number') out.priced = true;
      out.tokensToday += (s.usage?.input ?? 0) + (s.usage?.output ?? 0);
    }
  }
  out.costToday = Math.round(out.costToday * 100) / 100;
  return out;
}

/** One meter of the compact plan-usage segment. */
export interface CompactMeter {
  key: string;
  short: string;
  pct: number;
  bar: number;
  tone: Tone;
  /** Time left until the window starts over ('' when unknown or reset). */
  left: string;
}

/** The compact view of the plan usage: the session and weekly windows (and
 * any other window near its limit), the worst tone across every window,
 * and a sentence for assistive tech. */
export function compactPlan(
  u: PlanUsage | null,
  now: number,
): { meters: CompactMeter[]; tone: Tone; label: string } | null {
  if (!u?.available) return null;
  const meters = compactWindows(u.windows, now).map(({ window: w, short }) => {
    const pct = windowPct(w, now);
    const reset = !!w.resetsAt && w.resetsAt > now;
    return {
      key: w.key,
      short,
      pct,
      bar: Math.min(100, pct),
      tone: severityTone(windowSeverity(w, now)),
      left: reset ? formatCountdown((w.resetsAt ?? now) - now) : '',
    };
  });
  const parts = u.windows.map((w) => {
    const left = resetsIn(w.resetsAt, now);
    return `${w.label} ${windowPct(w, now)}%${left ? `, ${left}` : ''}`;
  });
  return {
    meters,
    tone: severityTone(worstSeverity(u.windows, now)),
    label: `Plan usage: ${parts.length ? parts.join('; ') : 'not read yet'}. Show details`,
  };
}

/**
 * "3 working · 7 agents running · 2 need you · $4.12 today" across every
 * session, then the Claude plan's usage ("Session 42% · Week 18%", with the
 * time until the session window resets). Each part is a shortcut (a list
 * filter, the Agents page, the plan-usage sheet). Cost is what was priced
 * since local midnight (each session's `costToday`, not its lifetime cost),
 * falling back to tokens when no session's price is known. Counted like the
 * server's fleet summary ({@link fleetCounts}), so the phone and the fleet
 * agree. The plan part is hidden when the account has no plan usage.
 */
@Component({
  selector: 'mc-fleet-summary',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, IconComponent, PlanUsageSheetComponent],
  template: `
    @if (hasSessions() || plan()) {
      <div class="fleet" role="group" aria-label="Right now">
        @if (hasSessions()) {
          <button type="button" class="seg" (click)="filter.emit('live')">
            <mc-icon name="activity" [size]="14" />
            <strong>{{ working() }}</strong> working
          </button>
          <a class="seg" routerLink="/agents">
            <mc-icon name="agents" [size]="14" />
            <strong>{{ agents() }}</strong> {{ agents() === 1 ? 'agent' : 'agents' }} running
          </a>
          <button
            type="button"
            class="seg"
            [class.warn]="needs() > 0"
            (click)="filter.emit('needs')"
          >
            <mc-icon name="hand" [size]="14" />
            <strong>{{ needs() }}</strong> {{ needs() === 1 ? 'needs' : 'need' }} you
          </button>
          <span class="seg static" [title]="costTitle()">
            <mc-icon name="clock" [size]="14" />
            <strong>{{ today() }}</strong> today
          </span>
        }
        @if (plan(); as p) {
          <button
            type="button"
            class="seg plan"
            [attr.data-tone]="p.tone"
            [attr.aria-label]="p.label"
            (click)="sheetOpen.set(true)"
          >
            @for (m of p.meters; track m.key; let first = $first) {
              <span class="pm">
                <span class="pbar" [attr.data-tone]="m.tone" aria-hidden="true"
                  ><span class="pfill" [style.width.%]="m.bar"></span
                ></span>
                {{ m.short }} <strong [attr.data-tone]="m.tone">{{ m.pct }}%</strong>
                @if (first && m.left) {
                  <span class="left"><mc-icon name="clock" [size]="11" />{{ m.left }}</span>
                }
              </span>
            } @empty {
              <mc-icon name="gauge" [size]="14" /> Plan usage
            }
          </button>
        }
      </div>
      <mc-plan-usage-sheet [open]="sheetOpen()" (closed)="sheetOpen.set(false)" />
    }
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
      .seg.plan {
        flex-wrap: wrap;
        gap: 0.2rem 0.7rem;
        max-width: 100%;
        white-space: normal;
      }
      .seg.plan[data-tone='amber'],
      .seg.plan[data-tone='red'] {
        border-color: var(--tone-border);
        background: var(--tone-soft);
      }
      .pm {
        display: inline-flex;
        align-items: center;
        gap: 0.35rem;
        white-space: nowrap;
      }
      .seg .pm strong[data-tone='amber'],
      .seg .pm strong[data-tone='red'] {
        color: var(--tone);
      }
      .pbar {
        width: 1.9rem;
        height: 0.35rem;
        border-radius: 0.2rem;
        background: var(--panel-3);
        overflow: hidden;
      }
      .pfill {
        display: block;
        height: 100%;
        border-radius: inherit;
        background: var(--tone);
      }
      .left {
        display: inline-flex;
        align-items: center;
        gap: 0.2rem;
        color: var(--faint);
        font-variant-numeric: tabular-nums;
      }
      @media (max-width: 767px) {
        .seg.plan {
          flex: 1 1 100%;
          justify-content: center;
          padding-block: 0.3rem;
        }
        .pbar {
          width: 1.4rem;
        }
      }
    `,
  ],
})
export class FleetSummaryComponent {
  private realtime = inject(RealtimeService);
  private clock = inject(NowService);
  private planUsage = inject(PlanUsageService);

  readonly filter = output<'live' | 'needs'>();
  protected readonly sheetOpen = signal(false);

  protected readonly hasSessions = computed(() => this.realtime.sessions().length > 0);
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
    () =>
      `Spent since midnight across ${this.counts().todaySessions} sessions active today (estimated)`,
  );
  protected readonly plan = computed(() => compactPlan(this.planUsage.usage(), this.clock.now()));
}
