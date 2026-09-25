import { ChangeDetectionStrategy, Component, input } from '@angular/core';

import { IconComponent, IconName } from './icon.component';
import { Tone } from './util';

/** Small metadata chip: `<mc-chip icon="branch" tone="blue">main</mc-chip>`. */
@Component({
  selector: 'mc-chip',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent],
  host: { '[attr.data-tone]': 'tone()' },
  template: `
    @if (icon(); as i) {
      <mc-icon [name]="i" [size]="13" />
    }
    <span class="txt"><ng-content /></span>
  `,
  styles: [
    `
      :host {
        display: inline-flex;
        align-items: center;
        gap: 0.3rem;
        max-width: 100%;
        min-width: 0;
        font-size: 0.75rem;
        line-height: 1.4;
        color: var(--tone);
        background: var(--tone-soft);
        border: 1px solid var(--tone-border);
        border-radius: 0.4rem;
        padding: 0.08rem 0.45rem;
        white-space: nowrap;
      }
      .txt {
        overflow: hidden;
        text-overflow: ellipsis;
        min-width: 0;
      }
    `,
  ],
})
export class ChipComponent {
  readonly tone = input<Tone>('muted');
  readonly icon = input<IconName | undefined>(undefined);
}
