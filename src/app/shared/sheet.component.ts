import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  ElementRef,
  afterNextRender,
  effect,
  inject,
  input,
  output,
  viewChild,
} from '@angular/core';

import { IconComponent } from './icon.component';

/**
 * A dismissible panel: a bottom sheet on phones, a centred dialog on wider
 * screens. Escape, the backdrop and the close button all emit `closed`.
 * Focus moves into the panel when it opens. The element is moved to <body>
 * so no ancestor's stacking context (cards, sticky bars) can cover it.
 */
@Component({
  selector: 'mc-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent],
  host: { '(document:keydown.escape)': 'onEsc()' },
  template: `
    @if (open()) {
      <div class="backdrop">
        <button class="scrim" type="button" aria-label="Close" (click)="closed.emit()"></button>
        <div
          class="panel"
          role="dialog"
          aria-modal="true"
          [attr.aria-label]="title()"
          #panel
          tabindex="-1"
        >
          <div class="head">
            <strong>{{ title() }}</strong>
            <button
              class="btn ghost icon-only"
              type="button"
              aria-label="Close"
              (click)="closed.emit()"
            >
              <mc-icon name="x" />
            </button>
          </div>
          <div class="body"><ng-content /></div>
        </div>
      </div>
    }
  `,
  styles: [
    `
      .backdrop {
        position: fixed;
        inset: 0;
        z-index: 60;
        display: flex;
        align-items: flex-end;
        justify-content: center;
      }
      .scrim {
        position: absolute;
        inset: 0;
        border: 0;
        background: var(--scrim);
        cursor: default;
      }
      .panel {
        position: relative;
        width: 100%;
        max-height: 85dvh;
        overflow: auto;
        background: var(--panel);
        border: 1px solid var(--border);
        border-radius: 1rem 1rem 0 0;
        padding: 0.75rem 1rem calc(1rem + env(safe-area-inset-bottom));
        box-shadow: var(--shadow);
        outline: none;
      }
      .head {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        margin-bottom: 0.5rem;
      }
      .head strong {
        flex: 1;
        font-size: 1rem;
      }
      @media (min-width: 768px) {
        .backdrop {
          align-items: center;
          padding: 1rem;
        }
        .panel {
          max-width: 30rem;
          border-radius: var(--radius);
          padding-bottom: 1rem;
        }
      }
    `,
  ],
})
export class SheetComponent {
  readonly open = input(false);
  readonly title = input('');
  readonly closed = output<void>();
  private readonly panel = viewChild<ElementRef<HTMLElement>>('panel');

  constructor() {
    const host = inject(ElementRef<HTMLElement>).nativeElement as HTMLElement;
    afterNextRender(() => document.body.appendChild(host));
    inject(DestroyRef).onDestroy(() => host.remove());
    effect(() => {
      const el = this.panel()?.nativeElement;
      if (this.open() && el) queueMicrotask(() => el.focus());
    });
  }

  onEsc(): void {
    if (this.open()) this.closed.emit();
  }
}
