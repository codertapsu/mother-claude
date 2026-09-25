import { ChangeDetectionStrategy, Component, computed, inject, output } from '@angular/core';
import { RouterLink } from '@angular/router';

import { NowService } from '../core/now.service';
import { RealtimeService } from '../core/realtime.service';
import { IconComponent } from './icon.component';
import { formatTokens, formatUsd } from './util';

/**
 * "3 working · 7 agents running · 2 need you · $4.12 today" across every
 * session. Each part is a shortcut (a list filter, the Agents page). Cost is
 * what was priced since local midnight (each session's `costToday`, not its
 * lifetime cost), falling back to tokens when no session's price is known.
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

  protected readonly working = computed(
    () =>
      this.realtime.sessions().filter((s) => s.status === 'working' || s.status === 'starting')
        .length,
  );
  protected readonly agents = computed(() =>
    this.realtime
      .sessions()
      .reduce((n, s) => n + (s.agents?.running ?? 0) + (s.agents?.backgroundRunning ?? 0), 0),
  );
  protected readonly needs = computed(
    () =>
      this.realtime.sessions().filter((s) => s.status === 'waiting' || (s.pending?.length ?? 0) > 0)
        .length,
  );
  private readonly todays = computed(() => {
    const midnight = new Date(this.clock.now());
    midnight.setHours(0, 0, 0, 0);
    return this.realtime
      .sessions()
      .filter((s) => (s.lastActivityAt ?? s.startedAt ?? 0) >= midnight.getTime());
  });
  protected readonly today = computed(() => {
    const list = this.todays();
    const priced = list.filter((s) => typeof s.costUsd === 'number');
    if (priced.length) return formatUsd(priced.reduce((n, s) => n + (s.costToday ?? 0), 0));
    const tokens = list.reduce((n, s) => n + (s.usage?.input ?? 0) + (s.usage?.output ?? 0), 0);
    return `${formatTokens(tokens)} tokens`;
  });
  protected readonly costTitle = computed(
    () => `Spent since midnight across ${this.todays().length} sessions active today (estimated)`,
  );
}
