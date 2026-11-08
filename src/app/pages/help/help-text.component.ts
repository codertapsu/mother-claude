import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { inlineRuns } from './help-text';

/**
 * A help string with its `**bold**` and `` `code` `` marks as elements. Each
 * run is its own element, so no template whitespace lands between them.
 */
@Component({
  selector: 'mc-help-text',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @for (r of runs(); track $index) {
      @switch (r.kind) {
        @case ('strong') {
          <strong>{{ r.text }}</strong>
        }
        @case ('code') {
          <code>{{ r.text }}</code>
        }
        @default {
          <span>{{ r.text }}</span>
        }
      }
    }
  `,
  styles: [
    `
      code {
        overflow-wrap: anywhere;
      }
    `,
  ],
})
export class HelpTextComponent {
  readonly text = input.required<string>();
  protected readonly runs = computed(() => inlineRuns(this.text()));
}
