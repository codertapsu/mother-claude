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

import { AccountsService } from '../core/accounts.service';
import { ApiService } from '../core/api.service';
import { AuthService } from '../core/auth.service';
import { normalizeJob, plural, formatBytes } from '../core/data-jobs';
import { DataJobsService } from '../core/data-jobs.service';
import {
  DELETE_DANGEROUS_REASON,
  DeleteGroup,
  DeletePlan,
  DeletePlanSession,
  DeleteSkip,
  deleteBody,
  deleteButtonLabel,
  deleteErrorText,
  deletePlanLine,
  deleteSkipReason,
  mergeDeletePlans,
  normalizeDeletePlan,
} from '../core/delete-data';
import { JobView, SessionSummary } from '../core/models';
import { DataJobComponent } from './data-job.component';
import { IconComponent } from './icon.component';
import { SheetComponent } from './sheet.component';
import { sessionName, shortPath } from './util';

/** What the dialog deletes. */
export interface DeleteStart {
  /** One request per account (`from` left out: every directory that has
   * them). */
  groups: readonly DeleteGroup[];
  /** What it is, for the title: "this conversation", "3 projects". */
  what: string;
  /** Conversations the opener has at hand (titles for the lists). */
  known?: readonly Pick<SessionSummary, 'id' | 'name' | 'title' | 'projectName'>[];
}

/** Where the dialog is: checking, showing the dry run, running. */
export type DeleteStep = 'loading' | 'plan' | 'run' | 'error';

/** Rows listed before "and N more". */
const ROWS = 30;

/**
 * Delete conversations or projects for good (docs/ACCOUNTS.md §14.3): it
 * opens with the dry run — what goes, with titles and sizes, and what is
 * skipped and why (a running conversation, one open in the API) — and
 * *Keep a copy for 7 days so this can be undone*, on by default; turned off
 * the files are gone right away. The button names the count ("Delete 12
 * conversations"); then the jobs (one per directory) with their Undo.
 * Deleting is a dangerous action: a device without the right is told so
 * and nothing is asked of the Mac.
 */
@Component({
  selector: 'mc-delete-data-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [DataJobComponent, IconComponent, SheetComponent],
  template: `
    <mc-sheet [open]="open()" [title]="title()" (closed)="close()">
      @if (open()) {
        @if (!canDangerous()) {
          <p class="notice warn small" role="status">{{ dangerousReason }}</p>
          <div class="row actions">
            <button class="btn ghost" type="button" (click)="close()">Close</button>
          </div>
        } @else {
          @switch (step()) {
            @case ('loading') {
              <p class="muted small">Checking what it would delete…</p>
            }
            @case ('error') {
              <p class="notice error small" role="alert">{{ error() }}</p>
              <div class="row actions">
                <button class="btn" type="button" (click)="check()">Try again</button>
                <button class="btn ghost" type="button" (click)="close()">Close</button>
              </div>
            }
            @case ('plan') {
              @if (plan(); as p) {
                <p class="planline">{{ line() }}</p>
                @if (p.jobs > 1) {
                  <p class="muted small">
                    It runs as {{ p.jobs }} jobs, one for each account's folder.
                  </p>
                }
                @if (p.sessions.length) {
                  <h4>What goes</h4>
                  <ul class="plist" aria-label="What goes">
                    @for (s of shown(); track s.id + (s.account ?? '')) {
                      <li>
                        <span class="ptitle">{{ titleOf(s) }}</span>
                        <span class="muted small">{{ rowLine(s) }}</span>
                      </li>
                    }
                  </ul>
                  @if (more() > 0) {
                    <p class="muted small">and {{ more() }} more</p>
                  }
                }
                @if (p.skipped.length) {
                  <h4>Skipped ({{ p.skipped.length }})</h4>
                  <ul class="plist skips" aria-label="Skipped">
                    @for (k of p.skipped; track $index) {
                      <li>
                        <span class="ptitle">{{ skipName(k) }}</span>
                        <span class="small why">{{ reason(k) }}</span>
                      </li>
                    }
                  </ul>
                }
                @if (p.sessions.length) {
                  <label class="check">
                    <input
                      type="checkbox"
                      [checked]="keep()"
                      [disabled]="busy()"
                      (change)="keep.set($any($event.target).checked)"
                    />
                    <span>
                      Keep a copy for 7 days so this can be undone
                      <span class="muted small hint-line">
                        @if (keep()) {
                          The space comes back when the copy goes.
                        } @else {
                          Off: deleted for good, right away.
                        }
                      </span>
                    </span>
                  </label>
                  @if (!keep()) {
                    <p class="notice warn small" role="status">
                      Deleted for good, right away: there is no Undo.
                    </p>
                  }
                  <p class="muted small keeps">
                    Prompt history and each folder's settings and trust stay as they are.
                  </p>
                }
              }
              @if (error()) {
                <p class="notice error small" role="alert">{{ error() }}</p>
              }
              <div class="row actions">
                <button
                  class="btn danger"
                  type="button"
                  [disabled]="busy() || !plan()?.sessions?.length"
                  (click)="run()"
                >
                  <mc-icon name="trash" [size]="14" />
                  {{ busy() ? 'Deleting…' : buttonLabel() }}
                </button>
                <button class="btn ghost" type="button" [disabled]="busy()" (click)="close()">
                  Cancel
                </button>
              </div>
            }
            @case ('run') {
              <div class="jobs">
                @for (j of jobs(); track j.id) {
                  <mc-data-job class="jrow" [job]="j" (changed)="jobChanged($event)" />
                }
              </div>
              @if (error()) {
                <p class="notice error small" role="alert">{{ error() }}</p>
              }
              <div class="row actions">
                <button class="btn ghost" type="button" (click)="close()">
                  {{ anyRunning() ? 'Close (it goes on)' : 'Close' }}
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
      .planline {
        margin: 0.25rem 0 0.4rem;
        line-height: 1.5;
      }
      h4 {
        margin: 0.85rem 0 0.3rem;
        font-size: 0.85rem;
      }
      .plist {
        margin: 0;
        padding: 0;
        list-style: none;
      }
      .plist > li {
        display: flex;
        flex-direction: column;
        gap: 0.05rem;
        padding: 0.4rem 0;
        border-top: 1px solid var(--border);
        overflow-wrap: anywhere;
      }
      .ptitle {
        font-weight: 500;
      }
      .skips .why {
        color: var(--muted);
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
      .keeps {
        margin: 0.6rem 0 0;
      }
      .actions {
        margin-top: 0.9rem;
      }
      .jobs {
        display: flex;
        flex-direction: column;
      }
      .jrow {
        padding: 0.6rem 0;
      }
      .jrow + .jrow {
        border-top: 1px solid var(--border);
      }
    `,
  ],
})
export class DeleteDataSheetComponent {
  private api = inject(ApiService);
  private jobsSvc = inject(DataJobsService);
  private accounts = inject(AccountsService);
  protected readonly canDangerous = inject(AuthService).canDangerous;

  readonly open = input(false);
  readonly start = input<DeleteStart | null>(null);
  /** Closed, with the jobs it ran as they are now (none: nothing ran). */
  readonly closed = output<JobView[]>();

  protected readonly dangerousReason = DELETE_DANGEROUS_REASON;
  protected readonly step = signal<DeleteStep>('loading');
  protected readonly busy = signal(false);
  protected readonly error = signal('');
  protected readonly keep = signal(true);
  /** Each request's dry run, in the order of the groups. */
  private readonly plans = signal<(DeletePlan | null)[]>([]);
  protected readonly jobs = signal<JobView[]>([]);
  private readonly started = signal<DeleteStart | null>(null);
  private following?: AbortController;
  private checking = 0;

  protected readonly title = computed(() => `Delete ${this.started()?.what ?? 'conversations'}`);
  protected readonly plan = computed(() => {
    const plans = this.plans().filter((p): p is DeletePlan => !!p);
    return plans.length ? mergeDeletePlans(plans) : null;
  });
  protected readonly line = computed(() => {
    const p = this.plan();
    return p ? deletePlanLine(p) : '';
  });
  protected readonly buttonLabel = computed(() => deleteButtonLabel(this.plan()));
  protected readonly shown = computed(() => (this.plan()?.sessions ?? []).slice(0, ROWS));
  protected readonly more = computed(
    () => (this.plan()?.sessions.length ?? 0) - this.shown().length,
  );
  protected readonly anyRunning = computed(() => this.jobs().some((j) => j.state === 'running'));
  /** Titles the opener knows. */
  private readonly titles = computed(() => {
    const out = new Map<string, string>();
    for (const s of this.started()?.known ?? []) out.set(s.id, sessionName(s));
    for (const s of this.plan()?.sessions ?? []) if (s.title) out.set(s.id, s.title);
    return out;
  });

  constructor() {
    // Every opening starts over, with the dry run.
    let wasOpen = false;
    effect(() => {
      const open = this.open();
      if (open && !wasOpen) {
        untracked(() => {
          this.following?.abort();
          this.started.set(this.start());
          this.keep.set(true);
          this.jobs.set([]);
          this.plans.set([]);
          this.error.set('');
          this.busy.set(false);
          void this.accounts.ensureLoaded();
          if (this.canDangerous()) void this.check();
        });
      }
      wasOpen = open;
    });
    inject(DestroyRef).onDestroy(() => this.following?.abort());
  }

  protected titleOf(s: DeletePlanSession): string {
    return this.titles().get(s.id) || s.id.slice(0, 8);
  }

  protected rowLine(s: DeletePlanSession): string {
    const account = this.accounts.multi() && s.account ? this.accounts.byId(s.account)?.name : '';
    return [
      s.id.slice(0, 8),
      shortPath(s.cwd),
      s.files || s.bytes ? `${plural(s.files, 'file')} · ${formatBytes(s.bytes)}` : '',
      account ? `in ${account}` : '',
    ]
      .filter(Boolean)
      .join(' · ');
  }

  protected skipName(k: DeleteSkip): string {
    if (k.id) return this.titles().get(k.id) || k.id.slice(0, 8);
    if (k.cwd) return shortPath(k.cwd);
    return 'Something';
  }

  protected reason(k: DeleteSkip): string {
    return deleteSkipReason(k);
  }

  protected jobChanged(j: JobView): void {
    this.jobs.update((list) => list.map((x) => (x.id === j.id ? j : x)));
  }

  protected close(): void {
    this.closed.emit(this.jobs());
  }

  /** The dry run of every request. */
  protected async check(): Promise<void> {
    const groups = this.started()?.groups ?? [];
    const run = ++this.checking;
    this.step.set('loading');
    this.error.set('');
    try {
      const plans = await Promise.all(
        groups.map(async (g) => {
          const res = await this.api.planDelete(deleteBody(g, this.keep()));
          const plan = normalizeDeletePlan(res?.plan);
          if (!plan) throw new Error('The Mac sent a plan this app could not read.');
          return plan;
        }),
      );
      if (run !== this.checking) return;
      this.plans.set(plans);
      this.step.set('plan');
    } catch (e) {
      if (run !== this.checking) return;
      this.error.set(deleteErrorText(e));
      this.step.set('error');
    }
  }

  protected async run(): Promise<void> {
    const groups = this.started()?.groups ?? [];
    const plans = this.plans();
    if (this.busy() || !this.plan()?.sessions.length) return;
    this.busy.set(true);
    this.error.set('');
    const list: JobView[] = [];
    try {
      // Only the requests with something to delete.
      for (const [i, g] of groups.entries()) {
        if (!plans[i]?.sessions.length) continue;
        const res = await this.api.startDelete(deleteBody(g, this.keep()));
        const raw = Array.isArray(res?.jobs) && res.jobs.length ? res.jobs : [res?.job];
        for (const j of raw.map(normalizeJob)) {
          if (j && !list.some((x) => x.id === j.id)) list.push(j);
        }
      }
      if (!list.length) throw new Error('The Mac sent no job this app could read.');
    } catch (e) {
      this.busy.set(false);
      this.error.set(deleteErrorText(e));
      if (list.length) this.follow(list);
      return;
    }
    this.busy.set(false);
    this.follow(list);
  }

  private follow(list: JobView[]): void {
    this.jobs.set(list);
    this.step.set('run');
    this.following?.abort();
    const ctrl = new AbortController();
    this.following = ctrl;
    for (const j of list) {
      void this.jobsSvc
        .follow(j, (next) => this.jobChanged(next), ctrl.signal)
        .catch((e) => {
          if (!ctrl.signal.aborted)
            this.error.set(`Can't follow it any more: ${deleteErrorText(e)}`);
        });
    }
  }
}

/** The conversations a closed dialog deleted for good or for now (not
 * undone, not failed). */
export function deletedIds(jobs: readonly JobView[]): Set<string> {
  const out = new Set<string>();
  for (const j of jobs) {
    if (j.kind !== 'delete' || j.state !== 'done') continue;
    const r = j.result as { deleted?: unknown } | undefined;
    const ids = Array.isArray(r?.deleted) ? r.deleted : j.sessions;
    for (const id of ids) if (typeof id === 'string') out.add(id);
  }
  return out;
}

/** "3 conversations", "this conversation". */
export function countedWhat(n: number, noun: 'conversation' | 'project'): string {
  return n === 1 ? `this ${noun}` : plural(n, noun);
}
