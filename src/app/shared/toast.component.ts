import { ChangeDetectionStrategy, Component, inject } from '@angular/core';

import { NoticeService, Toast } from '../core/notice.service';
import { IconComponent, IconName } from './icon.component';

const ICONS: Record<Toast['level'], IconName> = {
  info: 'info',
  success: 'check',
  warning: 'alert',
  error: 'alert',
};

/**
 * The shell's toasts plus the polite screen-reader region. Toasts sit under
 * the top edge on phones (the composer and tab bar own the bottom) and in the
 * bottom-right corner on desktop.
 */
@Component({
  selector: 'mc-toasts',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent],
  template: `
    <div class="stack" role="status" aria-live="polite">
      @for (t of notices.toasts(); track t.id) {
        <div class="toast" [attr.data-level]="t.level">
          <mc-icon [name]="icon(t)" [size]="16" />
          <span class="text">{{ t.text }}</span>
          @if (t.action; as a) {
            <button class="btn small" type="button" (click)="run(t)">{{ a.label }}</button>
          }
          <button
            class="btn ghost icon-only small"
            type="button"
            aria-label="Dismiss"
            (click)="notices.dismiss(t.id)"
          >
            <mc-icon name="x" [size]="14" />
          </button>
        </div>
      }
    </div>
    <div class="visually-hidden" aria-live="polite" aria-atomic="true">
      {{ notices.announcement() }}
    </div>
  `,
  styles: [
    `
      .stack {
        position: fixed;
        z-index: 70;
        left: 50%;
        transform: translateX(-50%);
        top: calc(env(safe-area-inset-top) + 0.5rem);
        width: min(30rem, calc(100vw - 1.5rem));
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
        pointer-events: none;
      }
      @media (min-width: 768px) {
        .stack {
          top: auto;
          left: auto;
          transform: none;
          right: 1.25rem;
          bottom: 1.25rem;
        }
      }
      .toast {
        pointer-events: auto;
        display: flex;
        align-items: center;
        gap: 0.55rem;
        padding: 0.5rem 0.4rem 0.5rem 0.8rem;
        border-radius: var(--radius);
        border: 1px solid var(--border-strong);
        background: var(--panel);
        box-shadow: var(--shadow);
        font-size: 0.875rem;
      }
      .toast[data-level='success'] mc-icon:first-child {
        color: var(--green);
      }
      .toast[data-level='warning'] {
        border-color: var(--amber-border);
      }
      .toast[data-level='warning'] mc-icon:first-child {
        color: var(--amber);
      }
      .toast[data-level='error'] {
        border-color: var(--red-border);
      }
      .toast[data-level='error'] mc-icon:first-child {
        color: var(--red);
      }
      .text {
        flex: 1;
        min-width: 0;
        overflow-wrap: anywhere;
        line-height: 1.4;
      }
    `,
  ],
})
export class ToastComponent {
  protected readonly notices = inject(NoticeService);

  protected icon(t: Toast): IconName {
    return ICONS[t.level] ?? 'info';
  }

  protected run(t: Toast): void {
    this.notices.dismiss(t.id);
    t.action?.run();
  }
}
