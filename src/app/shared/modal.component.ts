import { Component, input, output } from '@angular/core';

/** Lightweight, dependency-free modal: backdrop + centered card, Esc/✕/backdrop
 * to close. Projects arbitrary content. */
@Component({
  selector: 'mc-modal',
  imports: [],
  host: { '(document:keydown.escape)': 'onEsc()' },
  template: `
    @if (open()) {
      <div class="backdrop">
        <button class="backdrop-btn" aria-label="Close" (click)="closed.emit()"></button>
        <div class="modal card" role="dialog" aria-modal="true" [attr.aria-label]="title()">
          <div class="modal-head">
            <strong>{{ title() }}</strong>
            <button class="x" aria-label="Close" (click)="closed.emit()">✕</button>
          </div>
          <div class="modal-body"><ng-content /></div>
          <div class="modal-foot">
            <button class="btn primary" (click)="closed.emit()">{{ confirmLabel() }}</button>
          </div>
        </div>
      </div>
    }
  `,
  styles: [
    `
      .backdrop {
        position: fixed;
        inset: 0;
        z-index: 50;
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 1rem;
        padding-top: max(1rem, env(safe-area-inset-top));
        background: rgba(0, 0, 0, 0.55);
      }
      .backdrop-btn {
        position: absolute;
        inset: 0;
        background: transparent;
        border: none;
        cursor: default;
      }
      .modal {
        position: relative;
        z-index: 1;
        width: 100%;
        max-width: 34rem;
        max-height: 85vh;
        overflow: auto;
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
      }
      .modal-head {
        display: flex;
        align-items: center;
        gap: 0.5rem;
      }
      .modal-head strong {
        font-size: 1.05rem;
      }
      .x {
        margin-left: auto;
        appearance: none;
        background: none;
        border: none;
        color: var(--muted);
        font-size: 1rem;
        cursor: pointer;
      }
      .modal-foot {
        display: flex;
        justify-content: flex-end;
        margin-top: 0.25rem;
      }
    `,
  ],
})
export class ModalComponent {
  readonly open = input(false);
  readonly title = input('');
  readonly confirmLabel = input('Got it');
  readonly closed = output<void>();

  onEsc(): void {
    if (this.open()) this.closed.emit();
  }
}
