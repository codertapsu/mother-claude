import { Component, DestroyRef, computed, effect, inject, untracked } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { ActivatedRoute, Router } from '@angular/router';
import { map } from 'rxjs';

import { DataJobsService } from '../../core/data-jobs.service';
import { TPipe } from '../../core/i18n/t.pipe';
import { JobView } from '../../core/models';
import { DataJobComponent } from '../../shared/data-job.component';
import { EmptyStateComponent } from '../../shared/empty-state.component';
import { JobDetailsComponent } from '../backup/job-details.component';
import { AccountsHeadComponent } from './accounts-head.component';

/** What the chips show (`?show=`). */
type Show = 'all' | 'moves' | 'backups' | 'deletes';

/** The chips, in order (English labels: translated where shown). */
const CHIPS: { id: Show; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'moves', label: 'Moves and copies' },
  { id: 'backups', label: 'Backups' },
  { id: 'deletes', label: 'Deleted' },
];

/** The kinds of job a chip shows. */
const KINDS: Record<Exclude<Show, 'all'>, readonly JobView['kind'][]> = {
  moves: ['move', 'copy'],
  backups: ['backup', 'restore', 'pull'],
  deletes: ['delete'],
};

/** The kinds that carry a backup's details (conflicts, skips). */
const DETAILED = new Set<JobView['kind']>(['backup', 'restore', 'pull']);

/**
 * *Accounts › Recent changes* (docs/UI-REDESIGN.md §6.6): every data job
 * the desktop keeps (`GET /data/jobs`) — moves and copies between accounts,
 * backups, restores, pulls, deletes — newest first, with chips to show one
 * kind (`?show=moves|backups|deletes`: the pages that start them link here
 * with theirs), how each ended, and *Undo* while it is possible. Live: a
 * job that starts or ends anywhere comes in by its frame, and a running one
 * shows how far it got. Computer moves are Computers › Activity's.
 */
@Component({
  selector: 'mc-accounts-changes',
  imports: [
    AccountsHeadComponent,
    DataJobComponent,
    EmptyStateComponent,
    JobDetailsComponent,
    TPipe,
  ],
  template: `
    <div class="page narrow">
      <mc-accounts-head />

      <div class="section-head">
        <h2>{{ 'Recent changes' | t }}</h2>
        <span class="spacer"></span>
        <span class="muted small">{{ 'Undo is possible for 7 days' | t }}</span>
      </div>
      <div class="chips" role="group" [attr.aria-label]="'Show' | t">
        @for (c of chips; track c.id) {
          <button
            type="button"
            class="chip-btn"
            [attr.aria-pressed]="show() === c.id"
            (click)="pick(c.id)"
          >
            {{ c.label | t }}
          </button>
        }
      </div>

      @if (shown().length) {
        <section class="card list jobs" [attr.aria-label]="'Recent changes' | t">
          @for (j of shown(); track j.id) {
            <mc-data-job class="jrow" [job]="j" (changed)="jobs.note($event)">
              @if (detailed(j)) {
                <mc-job-details [job]="j" />
              }
            </mc-data-job>
          }
        </section>
      } @else {
        <mc-empty-state
          icon="clock"
          [heading]="'No change yet' | t"
          [text]="
            'Moves and copies between accounts, backups, restores, pulls and deletes show here, with Undo while it is possible.'
              | t
          "
          help="move-data"
        />
      }
    </div>
  `,
  styles: [
    `
      .section-head {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        margin: 1.4rem 0 0.55rem;
      }
      .section-head h2 {
        margin: 0;
      }
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
      .jobs {
        padding-top: 0;
        padding-bottom: 0;
      }
      .jrow + .jrow {
        border-top: 1px solid var(--border);
      }
    `,
  ],
})
export class ChangesPageComponent {
  protected readonly jobs = inject(DataJobsService);
  private readonly router = inject(Router);
  private readonly route = inject(ActivatedRoute);

  protected readonly chips = CHIPS;
  /** The chip shown, from `?show=` (anything else: every kind). */
  protected readonly show = toSignal(
    this.route.queryParamMap.pipe(
      map((q) => {
        const s = q.get('show');
        return s === 'moves' || s === 'backups' || s === 'deletes' ? (s as Show) : 'all';
      }),
    ),
    { initialValue: 'all' as Show },
  );
  /** The jobs of the chip, newest first. */
  protected readonly shown = computed(() => {
    const s = this.show();
    const kinds = s === 'all' ? null : KINDS[s];
    return [...this.jobs.recent()]
      .filter((j) => !kinds || kinds.includes(j.kind))
      .sort((a, b) => b.startedAt - a.startedAt);
  });
  private readonly stop = new AbortController();
  /** Running jobs followed here (their progress). */
  private readonly following = new Set<string>();

  constructor() {
    inject(DestroyRef).onDestroy(() => this.stop.abort());
    void this.jobs.loadRecent();
    // A running job shows how far it got: follow each one that comes.
    effect(() => {
      const list = this.jobs.recent();
      untracked(() => this.followRunning(list));
    });
  }

  protected pick(id: Show): void {
    void this.router.navigate([], {
      relativeTo: this.route,
      queryParams: { show: id === 'all' ? null : id },
      queryParamsHandling: 'merge',
      replaceUrl: true,
    });
  }

  protected detailed(j: JobView): boolean {
    return DETAILED.has(j.kind);
  }

  private followRunning(list: readonly JobView[]): void {
    for (const j of list) {
      if (j.state !== 'running' || this.following.has(j.id)) continue;
      this.following.add(j.id);
      void this.jobs
        .follow(j, () => undefined, this.stop.signal)
        .catch(() => undefined)
        .finally(() => this.following.delete(j.id));
    }
  }
}
