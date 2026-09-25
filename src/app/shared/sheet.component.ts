import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  ElementRef,
  effect,
  inject,
  input,
  output,
  viewChild,
} from '@angular/core';

import { IconComponent } from './icon.component';

/**
 * A modal panel on the native `<dialog>` element (`showModal()`), so the
 * browser provides the focus trap, Escape, an inert background, the top
 * layer (no stacking-context surprises) and focus return. A bottom sheet on
 * phones and a centred dialog on wider screens; `size="full"` fills the
 * screen (long plans, the terminal). Escape, a click on the backdrop and the
 * close button all emit `closed`; the parent owns `open`.
 */
@Component({
  selector: 'mc-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent],
  template: `
    <dialog
      #dlg
      class="dlg"
      [class.full]="size() === 'full'"
      [attr.aria-label]="title()"
      (cancel)="onCancel($event)"
    >
      <div class="panel">
        <div class="head">
          <strong class="title">{{ title() }}</strong>
          <ng-content select="[sheetActions]" />
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
    </dialog>
  `,
  styles: [
    `
      .dlg {
        border: 0;
        padding: 0;
        margin: auto auto 0;
        width: 100%;
        max-width: 100%;
        max-height: 88dvh;
        background: transparent;
        color: var(--text);
        overflow: visible;
      }
      .dlg::backdrop {
        background: var(--scrim);
      }
      .panel {
        display: flex;
        flex-direction: column;
        max-height: 88dvh;
        background: var(--panel);
        border: 1px solid var(--border);
        border-radius: 1rem 1rem 0 0;
        padding: 0.6rem 1rem calc(1rem + env(safe-area-inset-bottom));
        box-shadow: var(--shadow);
      }
      .head {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        margin-bottom: 0.4rem;
        flex: 0 0 auto;
      }
      .title {
        flex: 1;
        min-width: 0;
        font-size: 1rem;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .body {
        overflow: auto;
        min-height: 0;
        overscroll-behavior: contain;
      }
      .dlg.full {
        margin: 0;
        max-height: none;
        height: var(--vvh, 100dvh);
      }
      .dlg.full .panel {
        height: 100%;
        max-height: none;
        border-radius: 0;
        padding-top: max(0.6rem, env(safe-area-inset-top));
      }
      .dlg.full .body {
        flex: 1;
      }
      @media (min-width: 768px) {
        .dlg {
          margin: auto;
          max-width: 32rem;
        }
        .panel {
          border-radius: var(--radius);
          padding-bottom: 1rem;
        }
        .dlg.full {
          max-width: min(64rem, 92vw);
          height: min(52rem, 88dvh);
          margin: auto;
        }
        .dlg.full .panel {
          border-radius: var(--radius);
          padding-top: 0.6rem;
        }
      }
    `,
  ],
})
export class SheetComponent {
  readonly open = input(false);
  readonly title = input('');
  /** `sheet` (default) or `full` screen. */
  readonly size = input<'sheet' | 'full'>('sheet');
  readonly closed = output<void>();

  private readonly dlg = viewChild.required<ElementRef<HTMLDialogElement>>('dlg');
  private returnFocus: HTMLElement | null = null;

  constructor() {
    effect(() => {
      const el = this.dlg().nativeElement;
      const open = this.open();
      if (open && !el.open) {
        this.returnFocus =
          document.activeElement instanceof HTMLElement ? document.activeElement : null;
        try {
          el.showModal();
        } catch {
          el.setAttribute('open', '');
        }
      } else if (!open && el.open) {
        el.close();
        this.returnFocus?.focus?.();
        this.returnFocus = null;
      }
    });
    // A click on the dialog box itself (not its panel) is a backdrop click.
    effect((onCleanup) => {
      const el = this.dlg().nativeElement;
      const onClick = (e: MouseEvent) => {
        if (e.target === el) this.closed.emit();
      };
      el.addEventListener('click', onClick);
      onCleanup(() => el.removeEventListener('click', onClick));
    });
    inject(DestroyRef).onDestroy(() => {
      const el = this.dlg().nativeElement;
      if (el.open) el.close();
    });
  }

  protected onCancel(e: Event): void {
    // Escape: let the parent decide (it owns `open`).
    e.preventDefault();
    this.closed.emit();
  }
}
