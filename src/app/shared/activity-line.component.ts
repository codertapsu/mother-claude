import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { Activity } from '../core/models';
import { ElapsedPipe } from './time.pipes';
import { activityIsBusy, activityLabel } from './util';

/** "Running Bash · npm test · 12s" — what a session or agent is doing now. */
@Component({
  selector: 'mc-activity-line',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ElapsedPipe],
  host: { '[attr.data-kind]': 'activity()?.kind' },
  template: `
    @if (activity(); as a) {
      <span class="dot" [class.busy]="busy()"></span>
      <span class="line">
        <span class="label">{{ label() }}</span>
        @if (a.detail) {
          <span class="sep">·</span><span class="detail">{{ a.detail }}</span>
        }
        @if (busy() && a.since) {
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
      :host([data-kind='waiting']) .dot {
        background: var(--amber);
      }
      :host([data-kind='retrying']) .dot {
        background: var(--red);
        color: var(--red);
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
  readonly activity = input<Activity | undefined>(undefined);
  protected readonly label = computed(() => activityLabel(this.activity()));
  protected readonly busy = computed(() => activityIsBusy(this.activity()));
}
