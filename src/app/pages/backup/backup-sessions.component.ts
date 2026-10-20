import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, input, output } from '@angular/core';

import { actionText, pullTakes, restoreTakes } from '../../core/backup';
import { formatBytes } from '../../core/data-jobs';
import { BackupAction, BackupIndexSession } from '../../core/models';
import { TimeAgoComponent } from '../../shared/time-ago.component';
import { shortPath } from '../../shared/util';

/** Rows shown before "Show all". */
const FIRST_ROWS = 60;

const TONES: Record<BackupAction, string> = {
  new: 'green',
  update: 'blue',
  conflict: 'amber',
  same: 'muted',
  localNewer: 'muted',
  live: 'faint',
};

/**
 * The sessions of a backup set with what a restore or a pull would do with
 * each (docs/BACKUP.md): a checkbox per session (sessions it can't take are
 * shown, not offered), its title (what the session list called it when it
 * was backed up; else its id), its id and folder, where and when it was
 * backed up, its size and Claude Code's pruning warning. What it would bring is listed
 * open; the rest (the same, newer here, already there, running) folded
 * below it.
 */
@Component({
  selector: 'mc-backup-sessions',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [NgTemplateOutlet, TimeAgoComponent],
  template: `
    @if (takes().length) {
      <div class="bulk">
        <span class="muted small">{{ selectedCount() }} of {{ takes().length }} selected</span>
        <button class="btn ghost small" type="button" (click)="setAll(true)">All</button>
        <button class="btn ghost small" type="button" (click)="setAll(false)">None</button>
      </div>
    }
    <ul class="rows">
      @for (s of shown(); track s.id) {
        <ng-container [ngTemplateOutlet]="row" [ngTemplateOutletContext]="{ $implicit: s }" />
      }
    </ul>
    @if (more() > 0) {
      <button class="btn ghost small" type="button" (click)="expand.emit()">
        Show {{ more() }} more
      </button>
    }
    @if (rest().length) {
      <details class="rest">
        <summary class="muted small">{{ restLabel() }}</summary>
        <ul class="rows">
          @for (s of rest(); track s.id) {
            <ng-container [ngTemplateOutlet]="row" [ngTemplateOutletContext]="{ $implicit: s }" />
          }
        </ul>
      </details>
    }
    @if (!sessions().length) {
      <p class="muted small">This backup holds no conversations yet.</p>
    }

    <ng-template #row let-s>
      <li class="srow" [attr.data-tone]="tone(s.action)">
        <label class="spick">
          <input
            type="checkbox"
            [checked]="selected().has(s.id)"
            [disabled]="!takesAction(s.action) || disabled()"
            [attr.aria-label]="'Session ' + (s.title || s.id.slice(0, 8)) + ': ' + text(s.action)"
            (change)="flip.emit(s.id)"
          />
          <span class="sbody">
            @if (s.title) {
              <span class="stitle">{{ s.title }}</span>
              <span class="sline muted small">
                <span class="mono">{{ s.id.slice(0, 8) }}</span>
                @if (s.cwd) {
                  <span>{{ short(s.cwd) }}</span>
                }
              </span>
            } @else {
              <span class="sline">
                <span class="mono">{{ s.id.slice(0, 8) }}</span>
                @if (s.cwd) {
                  <span class="small">{{ short(s.cwd) }}</span>
                }
              </span>
            }
            <span class="act small">{{ text(s.action) }}</span>
            <span class="muted small">
              From {{ s.host || 'another Mac' }} <mc-time [ms]="s.pushedAt" /> ·
              {{ bytes(s.remoteBytes) }}
            </span>
            @if (s.pruneWarning) {
              <span class="prune small">{{ s.pruneWarning }}</span>
            }
          </span>
        </label>
      </li>
    </ng-template>
  `,
  styles: [
    `
      :host {
        display: block;
      }
      .bulk {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.3rem;
        margin-bottom: 0.3rem;
      }
      .bulk span {
        margin-right: auto;
      }
      .rows {
        margin: 0;
        padding: 0;
        list-style: none;
      }
      .srow {
        border-top: 1px solid var(--border);
      }
      .spick {
        display: flex;
        align-items: flex-start;
        gap: 0.6rem;
        padding: 0.5rem 0;
        cursor: pointer;
      }
      .spick input {
        margin-top: 0.2rem;
      }
      .sbody {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.1rem;
        overflow-wrap: anywhere;
      }
      .sline {
        display: flex;
        flex-wrap: wrap;
        align-items: baseline;
        gap: 0.2rem 0.6rem;
      }
      .stitle {
        font-weight: 500;
        line-height: 1.35;
      }
      .act {
        color: var(--muted);
      }
      [data-tone='green'] .act {
        color: var(--green);
      }
      [data-tone='blue'] .act {
        color: var(--blue);
      }
      [data-tone='amber'] .act {
        color: var(--amber);
      }
      .prune {
        color: var(--amber);
      }
      .rest {
        margin-top: 0.5rem;
      }
      .rest summary {
        cursor: pointer;
        min-height: 32px;
        display: flex;
        align-items: center;
      }
    `,
  ],
})
export class BackupSessionsComponent {
  readonly sessions = input.required<BackupIndexSession[]>();
  readonly selected = input.required<ReadonlySet<string>>();
  readonly mode = input<'pull' | 'restore'>('restore');
  /** Show every row (else the first ones and "Show N more"). */
  readonly all = input(false);
  readonly disabled = input(false);
  /** A session's checkbox was ticked or cleared. */
  readonly flip = output<string>();
  /** Select (true) or clear (false) every session it can take. */
  readonly bulk = output<{ ids: string[]; on: boolean }>();
  /** "Show N more" was tapped. */
  readonly expand = output<void>();

  /** Listed open, in the set's order (newest push first): what a pull
   * brings, or what is new to the account a restore goes into. */
  private readonly listed = computed(() =>
    this.sessions().filter((s) =>
      this.mode() === 'pull' ? pullTakes(s.action) : s.action === 'new',
    ),
  );
  /** "x of y selected", All and None speak for the listed ones. */
  protected readonly takes = this.listed;
  protected readonly shown = computed(() =>
    this.all() ? this.listed() : this.listed().slice(0, FIRST_ROWS),
  );
  protected readonly more = computed(() => this.listed().length - this.shown().length);
  /** The rest, folded: what needs nothing (a pull), or what the account
   * has already or runs (a restore skips those). */
  protected readonly rest = computed(() => {
    const open = new Set(this.listed());
    return this.sessions().filter((s) => !open.has(s));
  });
  protected readonly restLabel = computed(() => {
    const n = this.rest().length;
    return this.mode() === 'pull'
      ? `${n} ${n === 1 ? 'needs' : 'need'} nothing (the same, newer here, or running here)`
      : `${n} ${n === 1 ? 'is' : 'are'} already there, or running there`;
  });
  protected readonly selectedCount = computed(
    () => this.listed().filter((s) => this.selected().has(s.id)).length,
  );

  protected takesAction(action: BackupAction): boolean {
    return this.mode() === 'pull' ? pullTakes(action) : restoreTakes(action);
  }

  protected text(action: BackupAction): string {
    return actionText(action, this.mode());
  }

  protected tone(action: BackupAction): string {
    return TONES[action];
  }

  protected short(p: string): string {
    return shortPath(p);
  }

  protected bytes(n: number): string {
    return formatBytes(n);
  }

  protected setAll(on: boolean): void {
    this.bulk.emit({ ids: this.takes().map((s) => s.id), on });
  }
}
