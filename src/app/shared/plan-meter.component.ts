import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';

import { PlanWindow } from '../core/models';
import { NowService } from '../core/now.service';
import { hasReset, windowPct, windowSeverity } from '../core/plan-usage';
import { capitalize, resetClock, resetsIn, severityTone } from './util';

/**
 * One window of the Claude plan's usage: its name, a bar, the percentage,
 * when it starts over (counting down on the shared clock, no polling) and,
 * near the limit, a word as well as a colour. Once the reset time passes it
 * shows 0 % until the server says otherwise.
 */
@Component({
  selector: 'mc-plan-meter',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    role: 'meter',
    'aria-valuemin': '0',
    'aria-valuemax': '100',
    '[attr.aria-label]': 'window().label',
    '[attr.aria-valuenow]': 'bar()',
    '[attr.aria-valuetext]': 'valueText()',
    '[attr.data-tone]': 'tone()',
  },
  template: `
    <div class="head">
      <span class="label">{{ window().label }}</span>
      <span class="pct">{{ pct() }}%</span>
    </div>
    <span class="bar"><span class="fill" [style.width.%]="bar()"></span></span>
    <div class="foot">
      <span>{{ resetText() }}</span>
      @if (severityText()) {
        <span class="sev">{{ severityText() }}</span>
      }
    </div>
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.3rem;
        min-width: 0;
      }
      .head,
      .foot {
        display: flex;
        align-items: baseline;
        justify-content: space-between;
        gap: 0.75rem;
        min-width: 0;
      }
      .label {
        font-size: 0.875rem;
        min-width: 0;
        overflow-wrap: anywhere;
      }
      .pct {
        font-weight: 600;
        font-variant-numeric: tabular-nums;
        color: var(--tone);
      }
      .bar {
        display: block;
        height: 0.45rem;
        border-radius: 0.25rem;
        background: var(--panel-3);
        overflow: hidden;
      }
      .fill {
        display: block;
        height: 100%;
        border-radius: inherit;
        background: var(--tone);
      }
      .foot {
        font-size: 0.75rem;
        color: var(--muted);
        font-variant-numeric: tabular-nums;
      }
      .sev {
        color: var(--tone);
        font-weight: 600;
        white-space: nowrap;
      }
    `,
  ],
})
export class PlanMeterComponent {
  private clock = inject(NowService);
  readonly window = input.required<PlanWindow>();

  protected readonly pct = computed(() => windowPct(this.window(), this.clock.now()));
  protected readonly bar = computed(() => Math.min(100, this.pct()));
  private readonly severity = computed(() => windowSeverity(this.window(), this.clock.now()));
  protected readonly tone = computed(() => severityTone(this.severity()));
  protected readonly severityText = computed(() => {
    const s = this.severity();
    if (s === 'critical') return this.pct() >= 100 ? 'Limit reached' : 'Almost at the limit';
    return s === 'warning' ? 'Near the limit' : '';
  });
  protected readonly resetText = computed(() => {
    const w = this.window();
    const now = this.clock.now();
    if (hasReset(w, now)) return 'Reset: a new window starts with your next message';
    const left = resetsIn(w.resetsAt, now);
    return left ? `${capitalize(left)} · ${resetClock(w.resetsAt, now)}` : '';
  });
  protected readonly valueText = computed(() =>
    [`${this.pct()}% used`, this.severityText(), resetsIn(this.window().resetsAt, this.clock.now())]
      .filter(Boolean)
      .join(', '),
  );
}
