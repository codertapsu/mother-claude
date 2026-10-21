import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
} from '@angular/core';

import { AccountsService } from '../../core/accounts.service';
import { ApiService } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import {
  PRUNE_WARNING,
  backupErrorText,
  normalizeBackupIndex,
  ownSets,
  preselect,
  prunedOf,
} from '../../core/backup';
import { jobIdOf, normalizeJob, plural } from '../../core/data-jobs';
import { DataJobsService } from '../../core/data-jobs.service';
import { BackupIndex, BackupSet, JobView } from '../../core/models';
import { DANGEROUS_REASON } from '../../core/move-data';
import { DataJobComponent } from '../../shared/data-job.component';
import { SheetComponent } from '../../shared/sheet.component';
import { BackupSessionsComponent } from './backup-sessions.component';
import { JobDetailsComponent } from './job-details.component';

/** Where the restore dialog is. */
export type RestoreStep = 'loading' | 'list' | 'confirm' | 'run' | 'error';

/**
 * Bring sessions back from a backup set (docs/BACKUP.md), from the desktop
 * or a paired phone (docs/ACCOUNTS.md §13.9). Choose
 * the set (one per directory: a token account's sessions are in the
 * Default's) and the account to restore into (the same by default); the
 * set's sessions are listed by title with how each compares (new ones
 * start selected) and
 * Claude Code's pruning warning next to those it would delete again. If any
 * selected one has it, the dialog says so before it starts. A folder's
 * trust comes back only when ticked (it needs the dangerous right).
 * Restoring writes only new files and can be undone.
 */
@Component({
  selector: 'mc-restore-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [BackupSessionsComponent, DataJobComponent, JobDetailsComponent, SheetComponent],
  template: `
    <mc-sheet [open]="!!account()" title="Restore from a backup" (closed)="closed.emit()">
      @if (account()) {
        @if (step() !== 'run') {
          <div class="pick">
            <label class="field">
              <span class="field-label">Backup of</span>
              <select [disabled]="busy()" (change)="pickSet($event)">
                @for (s of choosable(); track s.account) {
                  <option [value]="s.account" [selected]="s.account === setId()">
                    {{ s.name }}
                  </option>
                }
              </select>
            </label>
            <label class="field">
              <span class="field-label">Restore into</span>
              <select [disabled]="busy()" (change)="pickInto($event)">
                @for (a of intoOptions(); track a.id) {
                  <option [value]="a.id" [selected]="a.id === intoId()">{{ a.name }}</option>
                }
              </select>
            </label>
          </div>
        }

        @switch (step()) {
          @case ('loading') {
            <p class="muted small">Opening the backup…</p>
          }
          @case ('error') {
            <p class="notice error small" role="alert">{{ error() }}</p>
            <div class="row actions">
              <button class="btn" type="button" (click)="load()">Try again</button>
            </div>
          }
          @case ('list') {
            @if (index(); as ix) {
              <mc-backup-sessions
                mode="restore"
                [sessions]="ix.sessions"
                [selected]="selected()"
                [all]="showAll()"
                [disabled]="busy()"
                (flip)="toggle($event)"
                (bulk)="bulk($event)"
                (expand)="showAll.set(true)"
              />
            }
            <label class="check">
              <input
                type="checkbox"
                [checked]="trust() && canDangerous()"
                [disabled]="busy() || !canDangerous()"
                (change)="trust.set($any($event.target).checked)"
              />
              <span>
                Also trust their folders for {{ intoName() }}
                <span class="muted small hint-line"
                  >A trusted folder runs its project hooks and MCP servers without asking.</span
                >
                @if (!canDangerous()) {
                  <span class="small hint-line blocked">{{ dangerousReason }}</span>
                }
              </span>
            </label>
            @if (error()) {
              <p class="notice error small" role="alert">{{ error() }}</p>
            }
            <div class="row actions">
              <button
                class="btn primary"
                type="button"
                [disabled]="!selected().size || busy()"
                (click)="restore()"
              >
                {{ busy() ? 'Starting…' : 'Restore ' + plural(selected().size, 'session') }}
              </button>
            </div>
          }
          @case ('confirm') {
            <div class="notice warn small" role="alert">
              <p>
                <strong>{{ pruneCount() }}</strong> of the selected sessions
                {{ pruneCount() === 1 ? 'is' : 'are' }} older than {{ intoName() }} keeps
                conversations. {{ pruneWarning }}
              </p>
              <p>
                Raise it in {{ intoName() }}'s settings.json first to keep
                {{ pruneCount() === 1 ? 'it' : 'them' }}.
              </p>
            </div>
            <div class="row actions">
              <button class="btn primary" type="button" [disabled]="busy()" (click)="start()">
                {{ busy() ? 'Starting…' : 'Restore anyway' }}
              </button>
              <button
                class="btn ghost"
                type="button"
                [disabled]="busy()"
                (click)="step.set('list')"
              >
                Back
              </button>
            </div>
          }
          @case ('run') {
            @if (job(); as j) {
              <mc-data-job [job]="j" (changed)="job.set($event)">
                <mc-job-details [job]="j" />
              </mc-data-job>
            }
            @if (error()) {
              <p class="notice error small" role="alert">{{ error() }}</p>
            }
            <div class="row actions">
              <button class="btn ghost" type="button" (click)="closed.emit()">
                {{ job()?.state === 'running' ? 'Close (it goes on)' : 'Close' }}
              </button>
            </div>
          }
        }
      }
    </mc-sheet>
  `,
  styles: [
    `
      .pick {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
        gap: 0.6rem;
        margin-bottom: 0.75rem;
      }
      .check {
        display: flex;
        align-items: flex-start;
        gap: 0.55rem;
        margin-top: 0.9rem;
        cursor: pointer;
      }
      .check input {
        margin-top: 0.2rem;
      }
      .hint-line {
        display: block;
      }
      .blocked {
        color: var(--amber);
      }
      .notice {
        margin: 0.75rem 0 0;
      }
      .notice p {
        margin: 0 0 0.4rem;
        line-height: 1.5;
      }
      .notice p:last-child {
        margin-bottom: 0;
      }
      .actions {
        margin-top: 0.9rem;
      }
    `,
  ],
})
export class RestoreSheetComponent {
  private api = inject(ApiService);
  private jobs = inject(DataJobsService);
  /** A folder's trust needs the dangerous right (docs/ACCOUNTS.md §13.9). */
  protected readonly canDangerous = inject(AuthService).canDangerous;
  protected readonly dangerousReason = DANGEROUS_REASON;
  private accounts = inject(AccountsService);

  /** The account whose set to restore from (null: closed). */
  readonly account = input<string | null>(null);
  /** Every account's set (those of their own that exist can be chosen). */
  readonly sets = input<BackupSet[]>([]);
  readonly closed = output<void>();
  /** A restore started: its job id, for the page's list. */
  readonly started = output<string>();

  protected readonly step = signal<RestoreStep>('loading');
  protected readonly busy = signal(false);
  protected readonly error = signal('');
  protected readonly index = signal<BackupIndex | null>(null);
  protected readonly selected = signal<ReadonlySet<string>>(new Set());
  protected readonly trust = signal(false);
  protected readonly showAll = signal(false);
  protected readonly job = signal<JobView | null>(null);
  protected readonly setId = signal('');
  protected readonly intoId = signal('');
  private following?: AbortController;
  private loadSeq = 0;

  protected readonly pruneWarning = PRUNE_WARNING;
  protected readonly plural = plural;

  protected readonly choosable = computed(() => {
    const own = ownSets(this.sets());
    const list = own.filter((s) => s.exists);
    const cur = own.find((s) => s.account === this.setId());
    return cur && !list.includes(cur) ? [cur, ...list] : list;
  });
  /** Restoring needs a directory: a token account's is the Default's. */
  protected readonly intoOptions = computed(() =>
    this.accounts.list().filter((a) => a.kind === 'directory'),
  );
  protected readonly intoName = computed(
    () => this.accounts.byId(this.intoId())?.name ?? 'the account',
  );
  protected readonly pruneCount = computed(
    () => prunedOf(this.index()?.sessions ?? [], this.selected()).length,
  );

  constructor() {
    // Every opening starts over, with the set it was opened for.
    effect(() => {
      const id = this.account();
      untracked(() => {
        this.following?.abort();
        if (!id) return;
        this.setId.set(id);
        this.intoId.set(this.accounts.byId(id)?.kind === 'directory' ? id : 'default');
        this.trust.set(false);
        this.job.set(null);
        void this.load();
      });
    });
    inject(DestroyRef).onDestroy(() => this.following?.abort());
  }

  protected pickSet(e: Event): void {
    const id = (e.target as HTMLSelectElement).value;
    if (!id || id === this.setId()) return;
    this.setId.set(id);
    void this.load();
  }

  protected pickInto(e: Event): void {
    const id = (e.target as HTMLSelectElement).value;
    if (!id || id === this.intoId()) return;
    this.intoId.set(id);
    void this.load();
  }

  protected toggle(id: string): void {
    this.selected.update((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  protected bulk(e: { ids: string[]; on: boolean }): void {
    this.selected.update((prev) => {
      const next = new Set(prev);
      for (const id of e.ids) {
        if (e.on) next.add(id);
        else next.delete(id);
      }
      return next;
    });
  }

  /** The set's index, compared with the account to restore into. */
  async load(): Promise<void> {
    const seq = ++this.loadSeq;
    this.step.set('loading');
    this.error.set('');
    this.index.set(null);
    this.showAll.set(false);
    try {
      const raw = await this.api.getBackupIndex(this.setId(), this.intoId());
      if (seq !== this.loadSeq) return;
      const ix = normalizeBackupIndex(raw);
      if (!ix) throw new Error('The Mac sent a backup list this app could not read.');
      this.index.set(ix);
      this.selected.set(preselect(ix.sessions, 'restore'));
      this.step.set('list');
    } catch (e) {
      if (seq !== this.loadSeq) return;
      this.error.set(backupErrorText(e));
      this.step.set('error');
    }
  }

  /** Restore, saying first when Claude Code would prune some again. */
  protected restore(): void {
    if (!this.selected().size || this.busy()) return;
    if (this.pruneCount() > 0) {
      this.step.set('confirm');
      return;
    }
    void this.start();
  }

  protected async start(): Promise<void> {
    const ids = [...this.selected()];
    if (!ids.length || this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    let first: JobView | string;
    try {
      const res = await this.api.restoreBackup({
        account: this.setId(),
        ...(this.intoId() !== this.setId() ? { into: this.intoId() } : {}),
        ids,
        ...(this.trust() && this.canDangerous() ? { trust: true } : {}),
      });
      first = normalizeJob(res?.job) ?? jobIdOf(res?.job);
      if (!first) throw new Error('The Mac sent no job for the restore.');
    } catch (e) {
      this.busy.set(false);
      this.error.set(backupErrorText(e));
      if (this.step() === 'confirm') this.step.set('list');
      return;
    }
    this.busy.set(false);
    this.job.set(typeof first === 'string' ? null : first);
    this.step.set('run');
    this.started.emit(typeof first === 'string' ? first : first.id);
    this.following?.abort();
    const ctrl = new AbortController();
    this.following = ctrl;
    try {
      await this.jobs.follow(first, (j) => this.job.set(j), ctrl.signal);
    } catch (e) {
      if (!ctrl.signal.aborted) this.error.set(`Can't follow it any more: ${backupErrorText(e)}`);
    }
  }
}
