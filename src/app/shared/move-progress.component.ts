import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';

import { tr } from '../core/i18n/i18n';
import { TPipe } from '../core/i18n/t.pipe';
import { MoveRecord } from '../core/moves';
import { MoveWords, RECORD_WORDS, progressLines, stepClock } from '../core/moves-text';
import { NowService } from '../core/now.service';
import { IconComponent } from './icon.component';

/**
 * A move's steps (spikes/machines/UX.md §7.2): checked, stopped here,
 * packed, sent (with a bar, the bytes and about how long is left), placed
 * there, started there — each done, under way, still to come, or where it
 * stopped — with the time it began.
 */
@Component({
  selector: 'mc-move-progress',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, TPipe],
  template: `
    <ol class="steps" [attr.aria-label]="'Progress' | t">
      @for (l of lines(); track l.key) {
        <li [attr.data-state]="l.state">
          <span class="mark" aria-hidden="true">
            @switch (l.state) {
              @case ('done') {
                <mc-icon name="check" [size]="14" />
              }
              @case ('failed') {
                <mc-icon name="x" [size]="14" />
              }
              @case ('active') {
                <span class="spin"></span>
              }
              @default {
                <span class="dot"></span>
              }
            }
          </span>
          <span class="label">
            <span class="visually-hidden">{{ stateWord(l.state) }}: </span>{{ l.label }}
          </span>
          @if (l.state !== 'todo' && l.at) {
            <time class="at" [attr.datetime]="iso(l.at)">{{ clock(l.at) }}</time>
          }
          @if (l.bar !== undefined) {
            <span
              class="bar"
              role="progressbar"
              aria-valuemin="0"
              aria-valuemax="100"
              [attr.aria-valuenow]="pct(l.bar)"
              [attr.aria-label]="l.label"
            >
              <span class="fill" [style.width.%]="pct(l.bar)"></span>
            </span>
            @if (l.eta) {
              <span class="eta">{{ l.eta }}</span>
            }
          }
        </li>
      }
    </ol>
  `,
  styles: [
    `
      .steps {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 0.35rem;
      }
      li {
        display: grid;
        grid-template-columns: 1.4rem 1fr auto;
        align-items: center;
        column-gap: 0.4rem;
        row-gap: 0.25rem;
        min-height: 1.6rem;
        font-size: 0.9rem;
      }
      li[data-state='todo'] {
        color: var(--faint);
      }
      li[data-state='failed'] .label {
        color: var(--red);
      }
      .mark {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        width: 1.2rem;
        height: 1.2rem;
      }
      li[data-state='done'] .mark {
        color: var(--green);
      }
      li[data-state='failed'] .mark {
        color: var(--red);
      }
      .dot {
        width: 0.45rem;
        height: 0.45rem;
        border-radius: 50%;
        border: 1.5px solid var(--faint);
      }
      .spin {
        width: 0.85rem;
        height: 0.85rem;
        border: 2px solid var(--border-strong);
        border-top-color: var(--accent);
        border-radius: 50%;
        animation: mc-spin 0.8s linear infinite;
      }
      .label {
        min-width: 0;
        overflow-wrap: anywhere;
      }
      .at {
        font-size: 0.75rem;
        color: var(--faint);
        font-variant-numeric: tabular-nums;
        white-space: nowrap;
      }
      .bar {
        grid-column: 2 / 3;
        height: 0.4rem;
        border-radius: 0.2rem;
        background: var(--panel-3);
        overflow: hidden;
      }
      .fill {
        display: block;
        height: 100%;
        border-radius: inherit;
        background: var(--accent);
        transition: width 0.3s ease;
      }
      .eta {
        font-size: 0.75rem;
        color: var(--muted);
        white-space: nowrap;
      }
    `,
  ],
})
export class MoveProgressComponent {
  private readonly clockSvc = inject(NowService);
  readonly move = input.required<MoveRecord>();
  /** Its computers in this computer's words (`MovesService.words`). */
  readonly words = input<MoveWords>(RECORD_WORDS);

  protected readonly lines = computed(() =>
    progressLines(this.move(), this.clockSvc.now(), this.words()),
  );

  protected clock(ms: number): string {
    return stepClock(ms);
  }

  protected iso(ms: number): string {
    return new Date(ms).toISOString();
  }

  protected pct(f: number): number {
    return Math.round(f * 100);
  }

  /** What a step's mark says to a screen reader. */
  protected stateWord(state: string): string {
    switch (state) {
      case 'done':
        return tr('Done');
      case 'active':
        return tr('Under way');
      case 'failed':
        return tr('Stopped');
      default:
        return tr('Still to come');
    }
  }
}
