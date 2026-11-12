import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { tr } from '../core/i18n/i18n';
import { TimeAgoComponent } from './time-ago.component';

/** The mark of each tone, in the Machines words. */
const MARKS = { danger: '✕', warn: '!', info: '●' } as const;

/**
 * One thing that waits for you in the Inbox, beside the requests
 * (docs/UI-REDESIGN.md §6.4): its title, one line of why, and its action
 * (the content) — a device that wants to pair, a computer's failing check,
 * a project changed on both computers, a backup that failed, an update.
 * Its mark says how it stands: ✕ it blocks, ! it works but worse, ● news.
 * Another computer's item names that computer.
 */
@Component({
  selector: 'mc-attention-card',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [TimeAgoComponent],
  host: { '[attr.data-tone]': 'tone()', '[attr.data-kind]': 'kind() || null' },
  template: `
    <article class="att" [attr.aria-label]="heading()">
      <span class="mark" aria-hidden="true">{{ mark() }}</span>
      <div class="body">
        <div class="top">
          <span class="visually-hidden">{{ toneLabel() }}</span>
          <strong class="title">{{ heading() }}</strong>
          @if (meta()) {
            <span class="meta muted small">{{ meta() }}</span>
          }
          @if (machine()) {
            <span class="mchip">{{ machine() }}</span>
          }
          @if (when(); as ms) {
            <mc-time class="when muted small" [ms]="ms" />
          }
        </div>
        @if (why()) {
          <p class="why small">{{ why() }}</p>
        }
        <div class="acts"><ng-content /></div>
      </div>
    </article>
  `,
  styles: [
    `
      :host {
        display: block;
        --tone: var(--muted);
        --tone-soft: var(--panel-2);
      }
      :host([data-tone='danger']) {
        --tone: var(--red);
        --tone-soft: var(--red-soft);
      }
      :host([data-tone='warn']) {
        --tone: var(--amber);
        --tone-soft: var(--amber-soft);
      }
      :host([data-tone='info']) {
        --tone: var(--accent-text);
        --tone-soft: var(--accent-soft);
      }
      .att {
        display: flex;
        gap: 0.65rem;
        padding: 0.75rem 0.85rem;
        background: var(--panel);
        border: 1px solid var(--border);
        border-left: 3px solid var(--tone);
        border-radius: var(--radius);
      }
      .mark {
        flex: 0 0 auto;
        width: 1.6rem;
        height: 1.6rem;
        border-radius: 50%;
        display: grid;
        place-items: center;
        background: var(--tone-soft);
        color: var(--tone);
        font-weight: 700;
        font-size: 0.85rem;
      }
      .body {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.2rem;
      }
      .top {
        display: flex;
        flex-wrap: wrap;
        align-items: baseline;
        gap: 0.3rem 0.5rem;
      }
      .title {
        overflow-wrap: anywhere;
      }
      .mchip {
        font-size: 0.7rem;
        font-weight: 600;
        padding: 0.05rem 0.45rem;
        border-radius: 1rem;
        border: 1px solid var(--border-strong);
        color: var(--muted);
        white-space: nowrap;
      }
      .when {
        margin-left: auto;
        white-space: nowrap;
      }
      .why {
        margin: 0;
        color: var(--muted);
        overflow-wrap: anywhere;
      }
      .acts {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0.4rem;
        margin-top: 0.35rem;
      }
      .acts:empty {
        display: none;
      }
    `,
  ],
})
export class AttentionCardComponent {
  /** What it is, in the language shown. */
  readonly heading = input.required<string>();
  /** One line of why (in the language shown). */
  readonly why = input('');
  /** A few words beside the title: where it comes from, how. */
  readonly meta = input('');
  readonly tone = input<'danger' | 'warn' | 'info'>('warn');
  /** The other computer it is about (its name). */
  readonly machine = input('');
  /** When it happened (ms). */
  readonly when = input<number | null | undefined>(null);
  /** What kind of item it is (for styles and specs). */
  readonly kind = input('');

  protected readonly mark = computed(() => MARKS[this.tone()]);
  /** The mark in words, for screen readers. */
  protected readonly toneLabel = computed(() => {
    switch (this.tone()) {
      case 'danger':
        return tr('Blocks:');
      case 'warn':
        return tr('Works, but worse:');
      default:
        return '';
    }
  });
}
