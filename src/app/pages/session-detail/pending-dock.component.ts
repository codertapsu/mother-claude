import { ChangeDetectionStrategy, Component, computed, inject, linkedSignal } from '@angular/core';

import { PendingRef } from '../../core/models';
import { ViewportService } from '../../core/viewport.service';
import { IconComponent } from '../../shared/icon.component';
import { lingerList } from '../../shared/linger';
import { PendingCardComponent } from '../../shared/pending-card.component';
import { SessionContext } from './session-context';

/**
 * The session's pending requests, pinned above the composer: one card at a
 * time with a "1 of N" pager (J/K on a focused card moves too), the card body
 * capped at 45vh with its own scroll. While the composer has focus on a
 * phone it folds into a one-line "Claude needs you · Review" bar so the
 * conversation stays visible above the keyboard.
 */
@Component({
  selector: 'mc-pending-dock',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, PendingCardComponent],
  template: `
    @if (list().length) {
      @if (collapsed()) {
        <button class="needs" type="button" (click)="expand()">
          <mc-icon name="hand" [size]="16" />
          <span>Claude needs you</span>
          <span class="spacer"></span>
          <span class="review"
            >Review{{ list().length > 1 ? ' (' + list().length + ')' : '' }}</span
          >
        </button>
      } @else {
        @if (list().length > 1) {
          <div class="pager" role="group" aria-label="Pending requests">
            <button
              class="btn ghost small icon-only"
              type="button"
              aria-label="Previous request"
              [disabled]="index() === 0"
              (click)="step(-1)"
            >
              <mc-icon name="back" [size]="16" />
            </button>
            <span class="count">{{ index() + 1 }} of {{ list().length }}</span>
            <button
              class="btn ghost small icon-only"
              type="button"
              aria-label="Next request"
              [disabled]="index() >= list().length - 1"
              (click)="step(1)"
            >
              <mc-icon name="chevronRight" [size]="16" />
            </button>
          </div>
        }
        @if (current(); as c) {
          <mc-pending-card
            [item]="c.item"
            [session]="ctx.session()"
            [gone]="c.gone"
            bodyMax="min(40vh, 20rem)"
            (move)="step($event)"
            (sendAsMessage)="toComposer($event)"
          />
        }
      }
    }
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.35rem;
        min-width: 0;
        /* The composer below must stay on screen: the requests scroll. */
        max-height: calc(var(--vvh, 100dvh) * 0.52);
        overflow-y: auto;
        overscroll-behavior: contain;
      }
      :host:empty {
        display: none;
      }
      .needs {
        appearance: none;
        display: flex;
        align-items: center;
        gap: 0.5rem;
        min-height: var(--tap);
        padding: 0 0.85rem;
        border-radius: var(--radius);
        border: 1px solid var(--amber-border);
        background: var(--amber-soft);
        color: var(--text);
        font: inherit;
        font-size: 0.875rem;
        font-weight: 600;
        cursor: pointer;
      }
      .needs mc-icon,
      .review {
        color: var(--amber);
      }
      .pager {
        display: flex;
        align-items: center;
        justify-content: center;
        gap: 0.5rem;
        font-size: 0.8rem;
        color: var(--muted);
      }
      .count {
        font-variant-numeric: tabular-nums;
      }
    `,
  ],
})
export class PendingDockComponent {
  protected readonly ctx = inject(SessionContext);
  private viewport = inject(ViewportService);

  private readonly lingering = lingerList<PendingRef>(() => this.ctx.pending());
  protected readonly list = computed(() =>
    [...this.lingering()].sort((a, b) => a.item.createdAt - b.item.createdAt),
  );
  protected readonly index = linkedSignal<number, number>({
    source: () => this.list().length,
    computation: (len, prev) => Math.max(0, Math.min(prev?.value ?? 0, len - 1)),
  });
  protected readonly current = computed(() => this.list()[this.index()]);
  protected readonly collapsed = computed(
    () => this.ctx.composerFocused() && this.viewport.phone(),
  );

  protected step(delta: number): void {
    const len = this.list().length;
    this.index.set(Math.max(0, Math.min(len - 1, this.index() + delta)));
  }

  protected expand(): void {
    const el = document.activeElement;
    if (el instanceof HTMLElement) el.blur();
    this.ctx.composerFocused.set(false);
  }

  protected toComposer(text: string): void {
    this.ctx.composerInsert.set({ text, at: Date.now() });
  }
}
