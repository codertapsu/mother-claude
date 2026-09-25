import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { UnifiedRow, toSplitRows } from './diff';

/**
 * Renders diff rows (see diff.ts) unified — one column with old/new line
 * gutters — or split side by side. Used by the Changes tab, tool results in
 * the transcript and the permission previews.
 */
@Component({
  selector: 'mc-diff-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[class.split]': "mode() === 'split'", '[class.bounded]': 'bounded()' },
  template: `
    @if (mode() === 'unified') {
      @for (l of rows(); track $index) {
        @if (l.kind === 'hunk' || l.kind === 'meta') {
          <div class="pline {{ l.kind }} full">{{ l.text || ' ' }}</div>
        } @else {
          <div class="pline {{ l.kind }}">
            <span class="gut">{{ l.oldNo ?? '' }}</span>
            <span class="gut">{{ l.newNo ?? '' }}</span>
            <span class="sign" aria-hidden="true">{{
              l.kind === 'add' ? '+' : l.kind === 'del' ? '−' : ''
            }}</span>
            <span class="ptext">{{ l.text || ' ' }}</span>
          </div>
        }
      }
    } @else {
      @for (r of splitRows(); track $index) {
        @if (r.header) {
          <div class="pline {{ r.headerKind }} full">{{ r.header }}</div>
        } @else {
          <div class="srow">
            <span class="gut">{{ r.left?.no ?? '' }}</span>
            <span class="scell {{ r.left?.kind }}">{{ r.left?.text || ' ' }}</span>
            <span class="gut gr">{{ r.right?.no ?? '' }}</span>
            <span class="scell {{ r.right?.kind }}">{{ r.right?.text || ' ' }}</span>
          </div>
        }
      }
    }
  `,
  styles: [
    `
      :host {
        display: block;
        overflow-x: auto;
        font-family: var(--mono);
        font-size: 0.76rem;
        padding: 0.35rem 0;
        background: var(--code-bg);
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
      }
      :host(.bounded) {
        max-height: 28rem;
        overflow-y: auto;
      }
      .pline {
        display: flex;
        line-height: 1.45;
        width: max-content;
        min-width: 100%;
      }
      .pline.full {
        white-space: pre;
        padding: 0 0.6rem;
      }
      .gut {
        flex: 0 0 3.2rem;
        text-align: right;
        padding-right: 0.45rem;
        color: var(--faint);
        font-size: 0.7rem;
        line-height: 1.6;
        user-select: none;
        overflow: hidden;
      }
      .sign {
        flex: 0 0 1rem;
        text-align: center;
        user-select: none;
      }
      .ptext {
        white-space: pre;
        padding-right: 0.6rem;
      }
      .pline.add {
        color: var(--green);
        background: var(--green-soft);
      }
      .pline.del {
        color: var(--red);
        background: var(--red-soft);
      }
      .pline.hunk {
        color: var(--accent-text);
        margin-top: 0.3rem;
      }
      .pline.meta {
        color: var(--muted);
      }
      .srow {
        display: grid;
        grid-template-columns: 3.2rem minmax(0, 1fr) 3.2rem minmax(0, 1fr);
      }
      :host(.split) .pline.full {
        white-space: pre-wrap;
        word-break: break-all;
        width: auto;
        min-width: 0;
      }
      .gut.gr {
        border-left: 1px solid var(--border);
      }
      .scell {
        white-space: pre-wrap;
        word-break: break-all;
        padding: 0 0.5rem;
      }
      .scell.add {
        color: var(--green);
        background: var(--green-soft);
      }
      .scell.del {
        color: var(--red);
        background: var(--red-soft);
      }
      .scell.empty {
        background: var(--panel-2);
      }
    `,
  ],
})
export class DiffViewComponent {
  readonly rows = input.required<UnifiedRow[]>();
  readonly mode = input<'unified' | 'split'>('unified');
  /** Cap the height and scroll inside. */
  readonly bounded = input(true);
  protected readonly splitRows = computed(() => toSplitRows(this.rows()));
}
