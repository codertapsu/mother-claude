import { ChangeDetectionStrategy, Component, input, output } from '@angular/core';

import { ACCOUNT_COLORS } from '../../core/accounts';
import { IconComponent } from '../../shared/icon.component';

const NAMES: Record<string, string> = {
  '#8b5cf6': 'Violet',
  '#0ea5e9': 'Sky blue',
  '#f59e0b': 'Amber',
  '#10b981': 'Green',
  '#ef4444': 'Red',
  '#ec4899': 'Pink',
  '#14b8a6': 'Teal',
  '#6366f1': 'Indigo',
  '#84cc16': 'Lime',
};

/** The account colours as swatches (a radio group). */
@Component({
  selector: 'mc-account-colors',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent],
  template: `
    <div class="swatches" role="radiogroup" [attr.aria-label]="label()">
      @for (c of colors; track c) {
        <button
          type="button"
          role="radio"
          class="sw"
          [style.background]="c"
          [attr.aria-checked]="c === value()"
          [attr.aria-label]="name(c)"
          [title]="name(c)"
          [disabled]="disabled()"
          (click)="picked.emit(c)"
        >
          @if (c === value()) {
            <mc-icon name="check" [size]="15" [stroke]="3" />
          }
        </button>
      }
    </div>
  `,
  styles: [
    `
      .swatches {
        display: flex;
        flex-wrap: wrap;
        gap: 0.4rem;
      }
      .sw {
        appearance: none;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        width: 2rem;
        height: 2rem;
        padding: 0;
        border: 2px solid transparent;
        border-radius: 50%;
        color: #fff;
        cursor: pointer;
        box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.18);
      }
      .sw[aria-checked='true'] {
        border-color: var(--text);
      }
      .sw:disabled {
        cursor: not-allowed;
        opacity: 0.5;
      }
      @media (max-width: 767px), (pointer: coarse) {
        .sw {
          width: 2.5rem;
          height: 2.5rem;
        }
      }
    `,
  ],
})
export class AccountColorsComponent {
  readonly value = input<string | null>(null);
  readonly label = input('Colour');
  readonly disabled = input(false);
  readonly picked = output<string>();

  protected readonly colors = ACCOUNT_COLORS;

  protected name(c: string): string {
    return NAMES[c] ?? c;
  }
}
