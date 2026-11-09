import { ChangeDetectionStrategy, Component, inject } from '@angular/core';

import { tr } from '../core/i18n/i18n';
import { TPipe } from '../core/i18n/t.pipe';
import { NoticeService, Toast, ToastLevel } from '../core/notice.service';
import { IconComponent, IconName } from './icon.component';

const ICONS: Record<ToastLevel, IconName> = {
  info: 'info',
  success: 'check',
  warning: 'alert',
  error: 'xCircle',
};

/** A toast's headline when it brings none of its own (English, translated
 * where read). */
const TITLES: Record<ToastLevel, string> = {
  info: 'Note',
  success: 'Done',
  warning: 'Needs your attention',
  error: 'Something went wrong',
};

/**
 * The shell's toasts plus the polite screen-reader region. Each toast says
 * at a glance how much it matters: its level's colour along its edge and
 * behind its icon, a bold headline, then the message and what can be done
 * about it. They sit under the top edge on phones (the composer and tab bar
 * own the bottom) and in the top-right corner on desktop, where they are
 * seen. A bar shows the time left; pointing at a toast or focusing it stops
 * that time, so it can be read. Errors and warnings are announced at once
 * (`role="alert"`), the rest politely.
 */
@Component({
  selector: 'mc-toasts',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, TPipe],
  template: `
    <div class="stack">
      @for (t of notices.toasts(); track t.id) {
        <div
          class="toast"
          [attr.data-level]="t.level"
          [attr.role]="urgent(t) ? 'alert' : 'status'"
          [attr.aria-live]="urgent(t) ? 'assertive' : 'polite'"
          (mouseenter)="notices.hold(t.id)"
          (mouseleave)="notices.release(t.id)"
          (focusin)="notices.hold(t.id)"
          (focusout)="notices.release(t.id)"
        >
          <span class="badge"><mc-icon [name]="icon(t)" [size]="18" /></span>
          <div class="body">
            <strong class="title">{{ title(t) }}</strong>
            @for (line of lines(t); track $index; let first = $first) {
              @if (first) {
                <p class="text">{{ line }}</p>
              } @else {
                <p class="text more">{{ line }}</p>
              }
            }
            @if (t.action; as a) {
              <button class="btn small primary act" type="button" (click)="run(t)">
                {{ a.label | t }}
              </button>
            }
          </div>
          <button
            class="btn ghost icon-only small close"
            type="button"
            [attr.aria-label]="'Dismiss' | t"
            (click)="notices.dismiss(t.id)"
          >
            <mc-icon name="x" [size]="16" />
          </button>
          @if (t.timeout > 0) {
            <span class="time" [style.animation-duration.ms]="t.timeout"></span>
          }
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
        top: calc(env(safe-area-inset-top) + 0.6rem);
        width: min(30rem, calc(100vw - 1.5rem));
        display: flex;
        flex-direction: column;
        gap: 0.55rem;
        pointer-events: none;
      }
      @media (min-width: 768px) {
        .stack {
          left: auto;
          transform: none;
          right: 1.25rem;
          top: 1rem;
          width: min(26rem, calc(100vw - 2.5rem));
        }
      }
      .toast {
        --edge: var(--accent);
        --tint: var(--accent-soft);
        position: relative;
        overflow: hidden;
        pointer-events: auto;
        display: flex;
        align-items: flex-start;
        gap: 0.75rem;
        padding: 0.85rem 0.6rem 0.95rem 0.9rem;
        border-radius: var(--radius);
        border: 1px solid var(--border-strong);
        border-left: 5px solid var(--edge);
        background: linear-gradient(var(--tint), var(--tint)), var(--panel);
        box-shadow: var(--shadow);
        animation: mc-toast-in 0.22s ease-out;
      }
      .toast[data-level='success'] {
        --edge: var(--green);
        --tint: var(--green-soft);
      }
      .toast[data-level='warning'] {
        --edge: var(--amber);
        --tint: var(--amber-soft);
        border-color: var(--amber-border);
        border-left-color: var(--edge);
      }
      .toast[data-level='error'] {
        --edge: var(--red);
        --tint: var(--red-soft);
        border-color: var(--red-border);
        border-left-color: var(--edge);
      }
      .badge {
        flex: 0 0 auto;
        display: grid;
        place-items: center;
        width: 2rem;
        height: 2rem;
        border-radius: 50%;
        background: var(--edge);
        color: var(--on-accent, #fff);
      }
      .body {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        gap: 0.2rem;
      }
      .title {
        font-size: 0.95rem;
        font-weight: 650;
        line-height: 1.3;
      }
      .text {
        margin: 0;
        font-size: 0.9rem;
        line-height: 1.45;
        overflow-wrap: anywhere;
      }
      /* What follows the first line: one point each. */
      .text.more {
        position: relative;
        padding-left: 0.9rem;
        margin-top: 0.15rem;
      }
      .text.more::before {
        content: '';
        position: absolute;
        left: 0.15rem;
        top: 0.6em;
        width: 0.35rem;
        height: 0.35rem;
        border-radius: 50%;
        background: var(--edge);
      }
      .act {
        margin-top: 0.35rem;
      }
      .close {
        flex: 0 0 auto;
        margin-top: -0.2rem;
      }
      /* The time left: it runs out along the bottom edge. */
      .time {
        position: absolute;
        left: 0;
        bottom: 0;
        height: 3px;
        width: 100%;
        background: var(--edge);
        opacity: 0.55;
        transform-origin: left;
        animation-name: mc-toast-time;
        animation-timing-function: linear;
        animation-fill-mode: forwards;
      }
      .toast:hover .time,
      .toast:focus-within .time {
        animation-play-state: paused;
      }
      @keyframes mc-toast-in {
        from {
          opacity: 0;
          transform: translateY(-0.5rem);
        }
      }
      @keyframes mc-toast-time {
        to {
          transform: scaleX(0);
        }
      }
      @media (prefers-reduced-motion: reduce) {
        .toast {
          animation: none;
        }
        .time {
          display: none;
        }
      }
    `,
  ],
})
export class ToastComponent {
  protected readonly notices = inject(NoticeService);

  protected icon(t: Toast): IconName {
    return ICONS[t.level] ?? 'info';
  }

  protected title(t: Toast): string {
    return t.title ?? tr(TITLES[t.level] ?? TITLES.info);
  }

  /** The message's lines: the first says what happened, each next one a
   * point about it. */
  protected lines(t: Toast): string[] {
    return t.text.split('\n').filter((l) => l.trim());
  }

  /** Errors and warnings are said at once. */
  protected urgent(t: Toast): boolean {
    return t.level === 'error' || t.level === 'warning';
  }

  protected run(t: Toast): void {
    this.notices.dismiss(t.id);
    t.action?.run();
  }
}
