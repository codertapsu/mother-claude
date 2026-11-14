import { Component, DestroyRef, computed, inject, signal } from '@angular/core';

import { computerNoun } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { MachinesService } from '../../core/machines.service';
import { MovesService } from '../../core/moves.service';
import { RunsService } from '../../core/runs.service';
import { MoveCardComponent } from '../../shared/move-card.component';
import { ComputersHeadComponent } from './computers-head.component';
import { RunsCardComponent } from './runs-card.component';

/** What the Activity's chips show. */
type Show = 'all' | 'moves' | 'commands' | 'tasks' | 'roles';

/** The chips, in order (English labels: translated where shown). */
const CHIPS: { id: Show; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'moves', label: 'Moves' },
  { id: 'commands', label: 'Commands' },
  { id: 'tasks', label: 'Tasks' },
  { id: 'roles', label: 'Team roles' },
];

/**
 * *Computers › Activity* (docs/UI-REDESIGN.md §6.5): what went between your
 * computers — *Recent moves* of sessions to and from this one (cancel, try
 * again, undo: spikes/machines/UX.md §1.2, §7.2–§7.3), the commands run on
 * another computer and from one here (the ones allowed without asking are
 * on *Permissions*), the tasks handed over, and the team roles this
 * computer runs for another's lead — with chips to show one kind. A phone
 * may act on a move, as a device may.
 */
@Component({
  selector: 'mc-computers-activity',
  imports: [ComputersHeadComponent, MoveCardComponent, RunsCardComponent, TPipe],
  template: `
    <div class="page narrow">
      <mc-computers-head />

      <div class="chips" role="group" [attr.aria-label]="'Show' | t">
        @for (c of chips(); track c.id) {
          <button
            type="button"
            class="chip-btn"
            [attr.aria-pressed]="show() === c.id"
            (click)="show.set(c.id)"
          >
            {{ c.label | t }}
          </button>
        }
      </div>

      @if (show() === 'all' || show() === 'moves') {
        <section class="card moves" [attr.aria-label]="'Recent moves' | t">
          <div class="chead">
            <h2 class="first">{{ 'Recent moves' | t }}</h2>
          </div>
          @if (recentMoves().length) {
            <div class="move-list">
              @for (e of shownMoves(); track e.move.id) {
                <mc-move-card [entry]="e" />
              }
            </div>
            @if (recentMoves().length > shownMoves().length) {
              <button type="button" class="btn ghost small" (click)="allMoves.set(true)">
                {{ 'Show all {n}' | t: { n: recentMoves().length } }}
              </button>
            }
          } @else {
            <p class="muted small">
              {{ 'No session moved to or from this {computer} yet.' | t: { computer: computer() } }}
            </p>
          }
        </section>
      }

      @if (inUse() && show() !== 'moves') {
        <mc-machines-runs [part]="show() === 'all' ? 'all' : part()" />
      }
    </div>
  `,
  styles: [
    `
      .chips {
        display: flex;
        flex-wrap: wrap;
        gap: 0.35rem;
        margin: 0 0 0.75rem;
      }
      .chip-btn {
        font: inherit;
        font-size: 0.82rem;
        padding: 0.25rem 0.7rem;
        border-radius: 1rem;
        border: 1px solid var(--border);
        background: var(--panel);
        color: var(--muted);
        cursor: pointer;
        min-height: 32px;
      }
      .chip-btn[aria-pressed='true'] {
        border-color: var(--accent);
        background: var(--accent-soft);
        color: var(--accent-text);
      }
      .chead {
        display: flex;
        align-items: center;
        gap: 0.6rem;
        margin-bottom: 0.35rem;
      }
      .chead h2 {
        margin: 0;
      }
      .card + mc-machines-runs {
        display: block;
        margin-top: 0.5rem;
      }
      /* One card per move, a line between them. */
      .move-list {
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
        margin-bottom: 0.5rem;
      }
      .move-list mc-move-card + mc-move-card {
        padding-top: 0.75rem;
        border-top: 1px solid var(--border);
      }
    `,
  ],
})
export class ActivityPageComponent {
  private readonly machines = inject(MachinesService);
  private readonly moves = inject(MovesService);
  private readonly runs = inject(RunsService);
  protected readonly computer = computerNoun;

  protected readonly show = signal<Show>('all');
  /** The chips: Team roles only while this computer runs one. */
  protected readonly chips = computed(() =>
    CHIPS.filter(
      (c) => c.id !== 'roles' || this.runs.roles().length > 0 || this.show() === 'roles',
    ),
  );
  /** The runs card's part for a chip. */
  protected readonly part = computed(() => {
    const s = this.show();
    return s === 'tasks' || s === 'roles' ? s : 'commands';
  });

  /** This computer's moves of sessions, sent and received (UX §1.2). */
  protected readonly recentMoves = this.moves.here;
  protected readonly allMoves = signal(false);
  protected readonly shownMoves = computed(() => {
    const all = this.recentMoves();
    return this.allMoves() ? all : all.slice(0, 5);
  });
  /** In use: on, being set up, or something is paired. */
  protected readonly inUse = computed(() => {
    const v = this.machines.view();
    return !!v && (v.enabled || v.setupStep !== null || v.peers.length > 0);
  });

  constructor() {
    const release = this.machines.watch();
    inject(DestroyRef).onDestroy(release);
    void this.machines.load();
    void this.moves.load();
  }
}
