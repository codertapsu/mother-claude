import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { tr } from '../core/i18n/i18n';
import { formatUsd } from './util';

/** "$1.84" — estimated from token usage, or final when Claude reported it. */
@Component({
  selector: 'mc-cost',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.title]': 'title()' },
  template: `{{ text() }}`,
  styles: [
    `
      :host {
        font-variant-numeric: tabular-nums;
        white-space: nowrap;
      }
    `,
  ],
})
export class CostComponent {
  readonly usd = input<number | undefined>(undefined);
  readonly final = input(false);

  protected readonly text = computed(() => formatUsd(this.usd()));
  protected readonly title = computed(() =>
    this.usd() === undefined
      ? tr('Cost unknown for this model')
      : this.final()
        ? tr('Cost reported by Claude Code')
        : tr('Estimated from token usage (API list prices)'),
  );
}
