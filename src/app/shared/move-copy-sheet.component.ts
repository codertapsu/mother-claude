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

import { DEFAULT_ACCOUNT, isAccountColor, sharersOf, transferTargets } from '../core/accounts';
import { AccountsService } from '../core/accounts.service';
import { ApiService } from '../core/api.service';
import { AuthService } from '../core/auth.service';
import {
  formatBytes,
  normalizeJob,
  normalizePlan,
  plural,
  transferErrorText,
} from '../core/data-jobs';
import { DataJobsService } from '../core/data-jobs.service';
import {
  AccountRef,
  AccountView,
  JobView,
  TransferMode,
  TransferPlan,
  TransferWhat,
} from '../core/models';
import { AccountChipComponent } from './account-chip.component';
import { DataJobComponent } from './data-job.component';
import { IconComponent } from './icon.component';
import { SheetComponent } from './sheet.component';
import { shortPath } from './util';

/** Where the dialog is: choosing, looking at the dry run, running. */
export type MoveStep = 'pick' | 'plan' | 'run';

/** Sessions of a plan listed before "and N more". */
const PLAN_ROWS = 30;

interface TargetOption {
  account: AccountView;
  color: string;
  /** Its directory, shortened. */
  dir: string;
  /** "Shared with Marcus (Pro)" for the Default ('' otherwise). */
  shared: string;
}

/**
 * Desktop: move or copy a conversation, or every conversation of a project,
 * to another Claude account's directory (docs/ACCOUNTS.md §6.5, P3.4). Pick
 * the account (only those with another directory: a token account shares
 * the Default's), Move or Copy, and whether the folder's trust comes along;
 * a dry run then lists what would be written; the run shows its progress,
 * and Undo stays offered for as long as the server keeps the undo copy.
 */
@Component({
  selector: 'mc-move-copy-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AccountChipComponent, DataJobComponent, IconComponent, SheetComponent],
  template: `
    <mc-sheet [open]="open()" [title]="title()" (closed)="closed.emit()">
      @if (open() && what(); as w) {
        @if (!isDesktop()) {
          <p class="muted small">Conversations are moved and copied on the Mac.</p>
        } @else {
          @if (step() !== 'run') {
            <p class="lede small">
              {{
                w.what === 'session' ? 'The conversation' : 'Every conversation of ' + short(w.cwd)
              }}
              is in the directory of <mc-account-chip [account]="from()" />. Copying leaves it there
              as it is; moving takes it out once every copy is verified.
            </p>
          }

          @switch (step()) {
            @case ('pick') {
              <h3>To</h3>
              <div class="opts" role="radiogroup" aria-label="To">
                @for (t of targets(); track t.account.id) {
                  <button
                    type="button"
                    role="radio"
                    class="opt"
                    [class.on]="t.account.id === targetId()"
                    [attr.aria-checked]="t.account.id === targetId()"
                    [disabled]="busy()"
                    (click)="pickTarget(t.account.id)"
                  >
                    <span class="adot" aria-hidden="true" [style.background]="t.color"></span>
                    <span class="ot">
                      <strong class="oname">{{ t.account.name }}</strong>
                      @if (t.dir) {
                        <span class="mono muted small">{{ t.dir }}</span>
                      }
                      @if (t.shared) {
                        <span class="muted small">{{ t.shared }}</span>
                      }
                    </span>
                    @if (t.account.id === targetId()) {
                      <mc-icon name="check" [size]="16" />
                    }
                  </button>
                } @empty {
                  <p class="muted small">
                    No other account keeps its own directory (a token account shares the Default's),
                    so there is nowhere to move it.
                  </p>
                }
              </div>

              <h3>How</h3>
              <div class="segmented" role="group" aria-label="Move or copy">
                <button
                  type="button"
                  [attr.aria-pressed]="mode() === 'copy'"
                  [disabled]="busy()"
                  (click)="setMode('copy')"
                >
                  <mc-icon name="copy" [size]="14" /> Copy
                </button>
                <button
                  type="button"
                  [attr.aria-pressed]="mode() === 'move'"
                  [disabled]="busy()"
                  (click)="setMode('move')"
                >
                  <mc-icon name="move" [size]="14" /> Move
                </button>
              </div>
              <p class="muted small hint">
                @if (mode() === 'copy') {
                  Both accounts have it afterwards; each copy goes on by itself.
                } @else {
                  Only {{ target()?.name ?? 'the other account' }} has it afterwards.
                }
              </p>

              <label class="check">
                <input
                  type="checkbox"
                  [checked]="trust()"
                  [disabled]="busy()"
                  (change)="setTrust($any($event.target).checked)"
                />
                <span>
                  Also trust this folder for {{ target()?.name ?? 'the other account' }}
                  <span class="muted small hint-line"
                    >A trusted folder runs its project hooks and MCP servers without asking.</span
                  >
                </span>
              </label>

              @if (w.what === 'session' && live()) {
                <p class="notice warn small" role="status">
                  It is running right now: it can be moved or copied once it has stopped.
                </p>
              }
              @if (error()) {
                <p class="notice error small" role="alert">{{ error() }}</p>
              }
              <div class="row actions">
                <button
                  class="btn primary"
                  type="button"
                  [disabled]="!target() || busy() || (w.what === 'session' && live())"
                  (click)="dryRun()"
                >
                  {{ busy() ? 'Checking…' : 'Check what it does' }}
                </button>
              </div>
            }

            @case ('plan') {
              @if (plan(); as p) {
                <p class="planline">{{ planLine() }}</p>
                @if (p.sessions.length) {
                  <ul class="psess">
                    @for (s of shownSessions(); track s.id) {
                      <li>
                        <details>
                          <summary>
                            <span class="mono">{{ s.id.slice(0, 8) }}</span>
                            @if (s.cwd) {
                              <span class="muted small">{{ short(s.cwd) }}</span>
                            }
                            <span class="muted small"
                              >{{ fileCount(s.files.length) }} · {{ bytes(sizeOf(s.files)) }}</span
                            >
                          </summary>
                          <ul class="pfiles">
                            @for (f of s.files; track f.to) {
                              <li class="mono small">{{ short(f.to) }}</li>
                            }
                          </ul>
                        </details>
                      </li>
                    }
                  </ul>
                  @if (p.sessions.length > shownSessions().length) {
                    <p class="muted small">
                      and {{ p.sessions.length - shownSessions().length }} more
                    </p>
                  }
                }
                <ul class="effects small">
                  @if (p.addsProjectEntry) {
                    <li>
                      Adds the folder's entry, with its trust, to {{ target()?.name }}'s settings.
                    </li>
                  }
                  @if (p.removesProjectEntry) {
                    <li>Removes the folder's entry from {{ from().name }}'s settings.</li>
                  }
                  @if (p.historyLines) {
                    <li>
                      Adds {{ plural(p.historyLines, 'line') }} of prompt history to
                      {{ target()?.name }}.
                    </li>
                  }
                  @if (p.writesShared) {
                    <li>
                      Writes {{ target()?.name }}'s shared files, so it waits until no Claude Code
                      session uses that directory.
                    </li>
                  }
                  <li>Everything it changes can be undone for seven days.</li>
                </ul>
              }
              @if (error()) {
                <p class="notice error small" role="alert">{{ error() }}</p>
              }
              <div class="row actions">
                <button class="btn primary" type="button" [disabled]="busy()" (click)="run()">
                  <mc-icon [name]="mode() === 'move' ? 'move' : 'copy'" [size]="14" />
                  {{ busy() ? 'Starting…' : runLabel() }}
                </button>
                <button class="btn ghost" type="button" [disabled]="busy()" (click)="back()">
                  Back
                </button>
              </div>
            }

            @case ('run') {
              @if (job(); as j) {
                <mc-data-job [job]="j" [label]="jobLabel()" (changed)="job.set($event)" />
              }
              @if (error()) {
                <p class="notice error small" role="alert">{{ error() }}</p>
              }
              <div class="row actions">
                @if (job()?.state === 'failed') {
                  <button class="btn" type="button" (click)="back()">Back</button>
                }
                <button class="btn ghost" type="button" (click)="closed.emit()">
                  {{ job()?.state === 'running' ? 'Close (it goes on)' : 'Close' }}
                </button>
              </div>
            }
          }
        }
      }
    </mc-sheet>
  `,
  styles: [
    `
      h3 {
        margin: 0.9rem 0 0.4rem;
        font-size: 0.9rem;
      }
      .lede {
        margin: 0 0 0.25rem;
        color: var(--muted);
        line-height: 1.6;
      }
      .opts {
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
      }
      .opt {
        appearance: none;
        display: flex;
        align-items: center;
        gap: 0.65rem;
        width: 100%;
        min-height: var(--tap);
        padding: 0.5rem 0.7rem;
        text-align: left;
        font: inherit;
        color: var(--text);
        background: var(--panel-2);
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        cursor: pointer;
      }
      .opt.on {
        border-color: var(--accent);
        background: var(--accent-soft);
      }
      .opt mc-icon {
        color: var(--accent-text);
      }
      .adot {
        flex: 0 0 auto;
        width: 0.7rem;
        height: 0.7rem;
        border-radius: 50%;
      }
      .ot {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.05rem;
        overflow-wrap: anywhere;
      }
      .hint {
        margin: 0.35rem 0 0;
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
      .notice {
        margin: 0.75rem 0 0;
      }
      .actions {
        margin-top: 0.9rem;
      }
      .planline {
        margin: 0.5rem 0;
        line-height: 1.5;
      }
      .psess,
      .pfiles,
      .effects {
        margin: 0;
        padding: 0;
        list-style: none;
      }
      .psess > li {
        padding: 0.35rem 0;
        border-top: 1px solid var(--border);
      }
      .psess summary {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0.2rem 0.6rem;
        cursor: pointer;
        min-height: 32px;
      }
      .pfiles {
        padding: 0.3rem 0 0.2rem 1rem;
      }
      .pfiles li {
        overflow-wrap: anywhere;
        color: var(--muted);
      }
      .effects {
        margin-top: 0.6rem;
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
        color: var(--muted);
      }
      .effects li::before {
        content: '• ';
      }
    `,
  ],
})
export class MoveCopySheetComponent {
  private api = inject(ApiService);
  private jobs = inject(DataJobsService);
  private accounts = inject(AccountsService);
  protected readonly isDesktop = inject(AuthService).isDesktop;

  readonly open = input(false);
  /** One conversation, or every conversation of a project. */
  readonly what = input<TransferWhat | null>(null);
  /** The account it is in now (the Default when null). */
  readonly source = input<AccountRef | null | undefined>(null);
  /** A session that runs now can't be moved or copied. */
  readonly live = input(false);
  readonly closed = output<void>();

  protected readonly step = signal<MoveStep>('pick');
  protected readonly busy = signal(false);
  protected readonly error = signal('');
  protected readonly mode = signal<TransferMode>('copy');
  protected readonly trust = signal(false);
  protected readonly plan = signal<TransferPlan | null>(null);
  protected readonly job = signal<JobView | null>(null);
  private readonly pickedTarget = signal<string | null>(null);
  private following?: AbortController;

  protected readonly from = computed<AccountRef>(() => {
    const s = this.source();
    if (s) return s;
    const d = this.accounts.byId(DEFAULT_ACCOUNT);
    return {
      id: DEFAULT_ACCOUNT,
      name: d?.name ?? 'Default',
      color: d?.color ?? '#8b5cf6',
      kind: 'directory',
    };
  });
  protected readonly targets = computed<TargetOption[]>(() => {
    const list = this.accounts.list();
    return transferTargets(list, this.from()).map((a) => {
      const sharers = a.isDefault ? sharersOf(list, a.id) : [];
      return {
        account: a,
        color: isAccountColor(a.color) ? a.color : 'var(--accent)',
        dir: shortPath(a.dir),
        shared: sharers.length ? `Shared with ${sharers.map((x) => x.name).join(', ')}` : '',
      };
    });
  });
  protected readonly targetId = computed(() => {
    const ids = this.targets().map((t) => t.account.id);
    const picked = this.pickedTarget();
    return picked && ids.includes(picked) ? picked : (ids[0] ?? '');
  });
  protected readonly target = computed(() => this.accounts.byId(this.targetId()));
  protected readonly title = computed(() =>
    this.what()?.what === 'project'
      ? 'Move or copy this project'
      : 'Move or copy to another account',
  );
  protected readonly shownSessions = computed(() =>
    (this.plan()?.sessions ?? []).slice(0, PLAN_ROWS),
  );
  protected readonly planLine = computed(() => {
    const p = this.plan();
    const to = this.target()?.name ?? 'the other account';
    if (!p) return '';
    if (!p.sessions.length) return `There is no conversation to ${p.mode} here.`;
    const what = `${plural(p.sessions.length, 'conversation')} (${plural(p.files, 'file')}, ${formatBytes(p.bytes)})`;
    return p.mode === 'move'
      ? `Moves ${what} into ${to}'s directory, out of ${this.from().name}'s.`
      : `Copies ${what} into ${to}'s directory.`;
  });
  protected readonly runLabel = computed(() => {
    const n = this.plan()?.sessions.length ?? 0;
    return `${this.mode() === 'move' ? 'Move' : 'Copy'} ${plural(n, 'conversation')}`;
  });
  /** What the running job is called, fixed when it starts (a move changes
   * the session's account, and with it the targets). */
  protected readonly jobLabel = signal('');

  protected readonly plural = plural;

  constructor() {
    // Every opening starts over.
    effect(() => {
      if (!this.open()) return;
      untracked(() => {
        this.following?.abort();
        this.step.set('pick');
        this.busy.set(false);
        this.error.set('');
        this.mode.set('copy');
        this.trust.set(false);
        this.plan.set(null);
        this.job.set(null);
        this.pickedTarget.set(null);
        void this.accounts.ensureLoaded();
      });
    });
    inject(DestroyRef).onDestroy(() => this.following?.abort());
  }

  protected short(p: string | undefined): string {
    return shortPath(p);
  }

  protected bytes(n: number): string {
    return formatBytes(n);
  }

  protected fileCount(n: number): string {
    return plural(n, 'file');
  }

  protected sizeOf(files: { size: number }[]): number {
    return files.reduce((n, f) => n + f.size, 0);
  }

  protected pickTarget(id: string): void {
    this.pickedTarget.set(id);
    this.error.set('');
  }

  protected setMode(m: TransferMode): void {
    this.mode.set(m);
    this.error.set('');
  }

  protected setTrust(on: boolean): void {
    this.trust.set(on);
    this.error.set('');
  }

  protected back(): void {
    this.following?.abort();
    this.step.set('pick');
    this.error.set('');
    this.plan.set(null);
    this.job.set(null);
  }

  /** The dry run: what it would write, nothing written yet. */
  protected async dryRun(): Promise<void> {
    const body = this.body();
    if (!body || this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      const res = await this.api.planTransfer(body);
      const plan = normalizePlan(res?.plan);
      if (!plan) throw new Error('The server sent a plan this app could not read.');
      this.plan.set(plan);
      this.step.set('plan');
    } catch (e) {
      this.error.set(transferErrorText(e));
    } finally {
      this.busy.set(false);
    }
  }

  protected async run(): Promise<void> {
    const body = this.body();
    if (!body || this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    let first: JobView | null;
    try {
      const res = await this.api.startTransfer(body);
      first = normalizeJob(res?.job);
      if (!first) throw new Error('The server sent a job this app could not read.');
    } catch (e) {
      this.busy.set(false);
      this.error.set(transferErrorText(e));
      return;
    }
    this.busy.set(false);
    this.jobLabel.set(
      `${this.mode() === 'move' ? 'Move' : 'Copy'} to ${this.target()?.name ?? 'another account'}`,
    );
    this.job.set(first);
    this.step.set('run');
    this.following?.abort();
    const ctrl = new AbortController();
    this.following = ctrl;
    try {
      await this.jobs.follow(first, (j) => this.job.set(j), ctrl.signal);
    } catch (e) {
      if (!ctrl.signal.aborted) this.error.set(`Can't follow it any more: ${transferErrorText(e)}`);
    }
  }

  private body() {
    const w = this.what();
    const t = this.target();
    if (!w || !t) return null;
    return { ...w, from: this.from().id, to: t.id, mode: this.mode(), trust: this.trust() };
  }
}
