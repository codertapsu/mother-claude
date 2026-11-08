import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, input, output } from '@angular/core';

import {
  forceReplaces,
  forceText,
  pruneWarningText,
  pullTakesSession,
  restoreLists,
  restoreTakes,
} from '../../core/backup';
import { formatBytes } from '../../core/data-jobs';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
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
 * backed up, its size, whether it is in the storage only (archived) and
 * Claude Code's pruning warning. What it would bring is listed
 * open; the rest (the same, newer here, already there, running) folded
 * below it. With `force` (a force pull or restore) the copies here that
 * are not the backup's are listed too, each saying it will be replaced.
 */
@Component({
  selector: 'mc-backup-sessions',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [NgTemplateOutlet, TimeAgoComponent, TPipe],
  template: `
    @if (takes().length) {
      <div class="bulk">
        <span class="muted small">{{
          '{n} of {total} selected' | t: { n: selectedCount(), total: takes().length }
        }}</span>
        <button class="btn ghost small" type="button" (click)="setAll(true)">
          {{ 'All' | t }}
        </button>
        <button class="btn ghost small" type="button" (click)="setAll(false)">
          {{ 'None||select' | t }}
        </button>
      </div>
    }
    <ul class="rows">
      @for (s of shown(); track s.id) {
        <ng-container [ngTemplateOutlet]="row" [ngTemplateOutletContext]="{ $implicit: s }" />
      }
    </ul>
    @if (more() > 0) {
      <button class="btn ghost small" type="button" (click)="expand.emit()">
        {{ 'Show {n} more' | t: { n: more() } }}
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
      <p class="muted small">{{ 'This backup holds no conversations yet.' | t }}</p>
    }

    <ng-template #row let-s>
      <li class="srow" [attr.data-tone]="tone(s.action)">
        <label class="spick">
          <input
            type="checkbox"
            [checked]="selected().has(s.id)"
            [disabled]="!takesSession(s) || disabled()"
            [attr.aria-label]="
              'Session {name}: {state}' | t: { name: s.title || s.id.slice(0, 8), state: text(s) }
            "
            (change)="flip.emit(s.id)"
          />
          <span class="sbody">
            @if (s.title) {
              <span class="stitle">{{ s.title }}</span>
              <span class="sline muted small">
                <span class="mono">{{ s.id.slice(0, 8) }}</span>
                @if (s.mapped || s.cwd) {
                  <span [title]="mappedTitle(s)">{{ short(s.mapped || s.cwd) }}</span>
                }
              </span>
            } @else {
              <span class="sline">
                <span class="mono">{{ s.id.slice(0, 8) }}</span>
                @if (s.mapped || s.cwd) {
                  <span class="small" [title]="mappedTitle(s)">{{ short(s.mapped || s.cwd) }}</span>
                }
              </span>
            }
            <span class="act small" [class.replaces]="force() && replaces(s)"
              >{{ text(s) }}
              @if (s.archived) {
                <span class="tag">{{ 'In storage only' | t }}</span>
              }
            </span>
            <span class="muted small">
              {{ 'From {host}' | t: { host: s.host || ('another machine' | t) } }}
              <mc-time [ms]="s.pushedAt" /> ·
              {{ bytes(s.remoteBytes) }}
            </span>
            @if (s.pruneWarning) {
              <span class="prune small">{{ prune(s.pruneWarning) }}</span>
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
      .act.replaces {
        color: var(--amber);
      }
      .prune {
        color: var(--amber);
      }
      .tag {
        margin-left: 0.35rem;
        font-size: 0.72rem;
        padding: 0.02rem 0.4rem;
        border-radius: 0.4rem;
        background: var(--panel-3);
        color: var(--muted);
        white-space: nowrap;
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
  /** A force pull or restore: the copies here that are not the backup's are
   * taken too (replaced, kept for undo). */
  readonly force = input(false);
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
   * brings, or what is new to the account a restore goes into (with
   * `force`, also the copies there it replaces). */
  private readonly listed = computed(() =>
    this.sessions().filter((s) =>
      this.mode() === 'pull' ? pullTakesSession(s, this.force()) : restoreLists(s, this.force()),
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
    const one = n === 1;
    if (this.force()) {
      return this.mode() === 'pull'
        ? one
          ? tr('{n} needs nothing (the same, running here, or in storage only)', { n })
          : tr('{n} need nothing (the same, running here, or in storage only)', { n })
        : one
          ? tr('{n} is there as the backup has it, or running there', { n })
          : tr('{n} are there as the backup has them, or running there', { n });
    }
    return this.mode() === 'pull'
      ? one
        ? tr('{n} needs nothing (the same, newer here, or running here)', { n })
        : tr('{n} need nothing (the same, newer here, or running here)', { n })
      : one
        ? tr('{n} is already there, or running there', { n })
        : tr('{n} are already there, or running there', { n });
  });
  protected readonly selectedCount = computed(
    () => this.listed().filter((s) => this.selected().has(s.id)).length,
  );

  protected takesSession(s: BackupIndexSession): boolean {
    if (this.mode() === 'pull') return pullTakesSession(s, this.force());
    return this.force() ? restoreLists(s, true) : restoreTakes(s.action);
  }

  protected replaces(s: BackupIndexSession): boolean {
    return forceReplaces(s);
  }

  protected text(s: BackupIndexSession): string {
    return forceText(s, this.mode(), this.force());
  }

  protected tone(action: BackupAction): string {
    return TONES[action];
  }

  protected short(p: string | undefined): string {
    return shortPath(p ?? '');
  }

  /** A session a folder pair puts in the folder here: where it lived. */
  protected mappedTitle(s: BackupIndexSession): string {
    const there = s.folder || s.cwd;
    return s.mapped && there
      ? tr('{folder} on {host}', { folder: there, host: s.host || tr('the other machine') })
      : '';
  }

  protected readonly prune = pruneWarningText;

  protected bytes(n: number): string {
    return formatBytes(n);
  }

  protected setAll(on: boolean): void {
    this.bulk.emit({ ids: this.takes().map((s) => s.id), on });
  }
}
