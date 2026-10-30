import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';

import { Activity } from '../core/models';
import { NowService } from '../core/now.service';
import { RealtimeService } from '../core/realtime.service';
import { ElapsedPipe } from './time.pipes';
import { activityIsBusy, activityLabel, resumesIn } from './util';

/** "Running Bash · npm test · 12s" — what a session or agent is doing now;
 * "Scheduled to resume · in 12m · Check the deploy" when a wakeup or job
 * will wake it (the time first: a long reason is cut, the time never). */
@Component({
  selector: 'mc-activity-line',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ElapsedPipe],
  host: { '[attr.data-kind]': 'activity()?.kind' },
  template: `
    @if (activity(); as a) {
      <span class="dot" [class.busy]="busy()" [class.stale]="stale()"></span>
      <span class="line">
        <span class="label">{{ label() }}</span>
        @if (resumes(); as r) {
          <span class="sep">·</span><span class="elapsed">{{ r }}</span>
        }
        @if (a.detail) {
          <span class="sep">·</span><span class="detail">{{ a.detail }}</span>
        }
        @if (!resumes() && (busy() || a.kind === 'waiting') && a.since) {
          <span class="sep">·</span><span class="elapsed">{{ a.since | elapsed }}</span>
        }
      </span>
    }
  `,
  styles: [
    `
      :host {
        display: flex;
        align-items: center;
        gap: 0.45rem;
        min-width: 0;
        font-size: 0.85rem;
        color: var(--muted);
      }
      .dot {
        flex: 0 0 auto;
        width: 0.45rem;
        height: 0.45rem;
        border-radius: 50%;
        background: var(--faint);
      }
      .dot.busy {
        background: var(--green);
        color: var(--green);
        animation: mc-pulse 1.6s ease-out infinite;
      }
      .dot.stale {
        background: var(--faint) !important;
        animation: none;
      }
      :host([data-kind='waiting']) .dot {
        background: var(--amber);
      }
      :host([data-kind='retrying']) .dot {
        background: var(--red);
        color: var(--red);
      }
      :host([data-kind='scheduled']) .dot {
        background: var(--accent);
      }
      .line {
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .label {
        color: var(--text);
        font-weight: 500;
      }
      .sep {
        margin: 0 0.3rem;
        color: var(--faint);
      }
      .detail {
        font-family: var(--mono);
        font-size: 0.8rem;
      }
      .elapsed {
        font-variant-numeric: tabular-nums;
      }
    `,
  ],
})
export class ActivityLineComponent {
  private realtime = inject(RealtimeService);
  private clock = inject(NowService);
  /** Not Live: the dot greys out (the activity may be out of date). */
  protected readonly stale = computed(() => this.realtime.connection() !== 'open');
  readonly activity = input<Activity | undefined>(undefined);
  protected readonly label = computed(() => activityLabel(this.activity()));
  protected readonly busy = computed(() => activityIsBusy(this.activity()));
  /** "in 12m" / "due now", ticking (only a scheduled activity reads the
   * clock). */
  protected readonly resumes = computed(() => {
    const a = this.activity();
    return a?.kind === 'scheduled' && a.resumesAt ? resumesIn(a.resumesAt, this.clock.now()) : '';
  });
}
