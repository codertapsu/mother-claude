import { ChangeDetectionStrategy, Component, inject, input } from '@angular/core';

import { AuthService } from '../core/auth.service';
import { TPipe } from '../core/i18n/t.pipe';

/**
 * Something only the desktop app changes or answers (docs/UI-REDESIGN.md
 * §5): the desktop gets the control (the content); any other device sees
 * its value and one line saying where it is done — never a disabled
 * control.
 *
 * ```html
 * <mc-desktop-only [value]="interval()"><select …></select></mc-desktop-only>
 * <mc-desktop-only [line]="'Answer on the desktop app.' | t"><button …/></mc-desktop-only>
 * ```
 */
@Component({
  selector: 'mc-desktop-only',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [TPipe],
  template: `
    @if (isDesktop()) {
      <ng-content />
    } @else {
      <span class="elsewhere">
        @if (value()) {
          <span class="value">{{ value() }}</span>
        }
        <span class="line muted small">{{ line() || ('Changed on the desktop app.' | t) }}</span>
      </span>
    }
  `,
  styles: [
    `
      .elsewhere {
        display: inline-flex;
        flex-direction: column;
        gap: 0.1rem;
        min-width: 0;
      }
      .value {
        font-size: 0.875rem;
        overflow-wrap: anywhere;
      }
    `,
  ],
})
export class DesktopOnlyComponent {
  protected readonly isDesktop = inject(AuthService).isDesktop;

  /** What it is set to, in the language shown (none: only the line). */
  readonly value = input('');
  /** Where it is done instead of "Changed on the desktop app." (in the
   * language shown). */
  readonly line = input('');
}
