import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { SessionStatus } from '../core/models';
import { statusLabel, statusTone } from './util';

/** Session status with a colour dot; the dot pulses while the session is live
 * and working (static under prefers-reduced-motion). */
@Component({
  selector: 'mc-status-pill',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.data-tone]': 'tone()', role: 'status' },
  template: `<span class="dot" [class.pulse]="pulse()"></span
    ><span class="txt">{{ text() }}</span>`,
  styles: [
    `
      :host {
        display: inline-flex;
        align-items: center;
        gap: 0.4rem;
        font-size: 0.75rem;
        font-weight: 600;
        color: var(--tone);
        background: var(--tone-soft);
        border: 1px solid var(--tone-border);
        border-radius: 1rem;
        padding: 0.1rem 0.55rem 0.1rem 0.45rem;
        white-space: nowrap;
        line-height: 1.4;
      }
      .dot {
        width: 0.5rem;
        height: 0.5rem;
        border-radius: 50%;
        background: currentColor;
        flex: 0 0 auto;
      }
      .dot.pulse {
        animation: mc-pulse 1.6s ease-out infinite;
      }
    `,
  ],
})
export class StatusPillComponent {
  readonly status = input.required<SessionStatus | string>();
  /** The process is alive (drives the pulsing dot). */
  readonly live = input(false);
  /** Override the label. */
  readonly label = input<string | undefined>(undefined);

  protected readonly tone = computed(() => statusTone(this.status()));
  protected readonly text = computed(() => this.label() ?? statusLabel(this.status()));
  protected readonly pulse = computed(
    () => this.live() && ['working', 'starting', 'waiting'].includes(this.status()),
  );
}
