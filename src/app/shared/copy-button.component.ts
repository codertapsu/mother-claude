import { ChangeDetectionStrategy, Component, input, signal } from '@angular/core';

import { IconComponent } from './icon.component';

/** Copies `text` to the clipboard (with a fallback for non-secure contexts,
 * where the async Clipboard API is unavailable). */
@Component({
  selector: 'mc-copy-button',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent],
  template: `
    <button
      type="button"
      class="btn small"
      [attr.aria-label]="label() + ': ' + text()"
      (click)="copy()"
    >
      <mc-icon [name]="copied() ? 'check' : 'copy'" [size]="14" />
      {{ copied() ? 'Copied' : label() }}
    </button>
  `,
})
export class CopyButtonComponent {
  readonly text = input.required<string>();
  readonly label = input('Copy');
  protected readonly copied = signal(false);
  private timer?: ReturnType<typeof setTimeout>;

  async copy(): Promise<void> {
    const text = this.text();
    let ok: boolean;
    try {
      await navigator.clipboard.writeText(text);
      ok = true;
    } catch {
      ok = legacyCopy(text);
    }
    if (ok) {
      this.copied.set(true);
      clearTimeout(this.timer);
      this.timer = setTimeout(() => this.copied.set(false), 1500);
    }
  }
}

function legacyCopy(text: string): boolean {
  try {
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.setAttribute('readonly', '');
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand('copy');
    document.body.removeChild(ta);
    return ok;
  } catch {
    return false;
  }
}
