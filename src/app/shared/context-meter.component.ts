import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { tr } from '../core/i18n/i18n';
import { ContextFill } from '../core/models';
import { Tone, formatTokens } from './util';

/** Context-window fill: a thin bar + percentage. Amber from 75 %, red from 90 %. */
@Component({
  selector: 'mc-context-meter',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '[attr.data-tone]': 'tone()',
    '[attr.title]': 'title()',
    role: 'meter',
    '[attr.aria-label]': 'ariaLabel()',
    'aria-valuemin': '0',
    'aria-valuemax': '100',
    '[attr.aria-valuenow]': 'pct()',
  },
  template: `
    <span class="bar"><span class="fill" [style.width.%]="pct()"></span></span>
    <span class="txt"
      >{{ pct() }}%
      @if (!compact()) {
        <span class="muted"> · {{ used() }} / {{ window() }}</span>
      }
    </span>
  `,
  styles: [
    `
      :host {
        display: inline-flex;
        align-items: center;
        gap: 0.4rem;
        font-size: 0.75rem;
        white-space: nowrap;
      }
      .bar {
        width: 3rem;
        height: 0.35rem;
        border-radius: 0.2rem;
        background: var(--panel-3);
        overflow: hidden;
        flex: 0 0 auto;
      }
      :host(.wide) .bar {
        width: 100%;
        flex: 1;
      }
      .fill {
        display: block;
        height: 100%;
        background: var(--tone);
        border-radius: inherit;
      }
      .txt {
        color: var(--tone);
      }
    `,
  ],
})
export class ContextMeterComponent {
  readonly context = input.required<ContextFill>();
  readonly compact = input(true);

  protected readonly pct = computed(() =>
    Math.max(0, Math.min(100, Math.round(this.context().pct ?? 0))),
  );
  protected readonly tone = computed<Tone>(() =>
    this.pct() >= 90 ? 'red' : this.pct() >= 75 ? 'amber' : 'accent',
  );
  protected readonly used = computed(() => formatTokens(this.context().used));
  protected readonly window = computed(() => formatTokens(this.context().window));
  protected readonly title = computed(() =>
    tr('Context: {used} of {window} tokens ({pct}%)', {
      used: this.used(),
      window: this.window(),
      pct: this.pct(),
    }),
  );
  protected readonly ariaLabel = computed(() => tr('Context window used'));
}
