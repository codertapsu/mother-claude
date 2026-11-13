import { ChangeDetectionStrategy, Component, input, signal } from '@angular/core';

import { TPipe } from '../core/i18n/t.pipe';
import { copyText } from './clipboard';
import { IconComponent } from './icon.component';

/** Copies `text` to the clipboard (with a fallback for non-secure contexts,
 * where the async Clipboard API is unavailable). `ariaLabel` replaces the
 * spoken "Copy: <text>" (for a secret that must not end up in it). `label`
 * is shown in the language shown (callers may pass it translated). */
@Component({
  selector: 'mc-copy-button',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, TPipe],
  template: `
    <button
      type="button"
      class="btn small"
      [attr.aria-label]="ariaLabel() || (label() | t) + ': ' + text()"
      (click)="copy()"
    >
      <mc-icon [name]="copied() ? 'check' : 'copy'" [size]="14" />
      {{ copied() ? ('Copied' | t) : (label() | t) }}
    </button>
  `,
})
export class CopyButtonComponent {
  readonly text = input.required<string>();
  readonly label = input('Copy');
  readonly ariaLabel = input('');
  protected readonly copied = signal(false);
  private timer?: ReturnType<typeof setTimeout>;

  async copy(): Promise<void> {
    if (await copyText(this.text())) {
      this.copied.set(true);
      clearTimeout(this.timer);
      this.timer = setTimeout(() => this.copied.set(false), 1500);
    }
  }
}
