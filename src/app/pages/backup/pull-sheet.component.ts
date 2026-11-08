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

import { ApiService } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import {
  ForceCounts,
  PRUNE_WARNING,
  backupErrorText,
  forceCounts,
  forceKinds,
  normalizeBackupIndex,
  normalizePullSkips,
  ownSets,
  preselect,
  prunedOf,
  pullTakesSession,
} from '../../core/backup';
import { jobIdOf, normalizeJob, plural, sentence } from '../../core/data-jobs';
import { DataJobsService } from '../../core/data-jobs.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { BackupIndex, BackupSet, JobView } from '../../core/models';
import { DANGEROUS_REASON } from '../../core/move-data';
import { machineNoun } from '../../core/paths';
import { DataJobComponent } from '../../shared/data-job.component';
import { SheetComponent } from '../../shared/sheet.component';
import { BackupSessionsComponent } from './backup-sessions.component';
import { JobDetailsComponent } from './job-details.component';

/** Where the pull dialog is ('confirm': a force pull says what it replaces
 * before it starts). */
export type PullStep = 'loading' | 'list' | 'confirm' | 'run';

/** One account's set in the dialog. */
interface PullGroup {
  account: string;
  name: string;
  index: BackupIndex | null;
  /** Why its set could not be read ('' when it was). */
  error: string;
}

/** A pull this dialog started. */
interface PullRun {
  account: string;
  label: string;
  job: JobView | null;
}

/**
 * Bring this Mac up to date from the backup sets, from the desktop or a
 * paired phone (docs/BACKUP.md
 * "Carrying sessions to another laptop") — one account's set, or every set
 * (one per directory: a token account's sessions are in the Default's; a
 * set whose key is not here says so). Before anything is written it lists
 * what will happen to each
 * session: new here; newer in the backup (this Mac's copy is replaced and
 * kept for undo); changed on both (kept side by side, never merged);
 * running here (skipped); newer here. Afterwards each account's result,
 * with the conflicts and the file each one was written to. A folder's trust
 * needs the dangerous right.
 *
 * Force (docs/BACKUP.md "Force backup, force pull"): the backup's copy
 * replaces this Mac's wherever they differ — also one that changed here or
 * is newer here. The list then shows what each copy here is and that it is
 * replaced, and a confirmation says how many, that sessions only on this
 * Mac stay, running ones are skipped and the replaced copies are kept for
 * undo. It needs the dangerous right, as deleting does.
 */
@Component({
  selector: 'mc-pull-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [BackupSessionsComponent, DataJobComponent, JobDetailsComponent, SheetComponent, TPipe],
  template: `
    <mc-sheet [open]="open()" [title]="title()" (closed)="closed.emit()">
      @if (open()) {
        @switch (step()) {
          @case ('loading') {
            <p class="muted small">{{ 'Opening the backups…' | t }}</p>
          }
          @case ('list') {
            @for (g of groups(); track g.account) {
              <section class="grp">
                @if (groups().length > 1) {
                  <h3>{{ g.name }}</h3>
                }
                @if (g.error) {
                  <p class="notice error small" role="alert">{{ g.error }}</p>
                } @else if (g.index; as ix) {
                  <mc-backup-sessions
                    mode="pull"
                    [force]="forcing()"
                    [sessions]="ix.sessions"
                    [selected]="selected()"
                    [all]="expanded().has(g.account)"
                    [disabled]="busy()"
                    (flip)="toggle($event)"
                    (bulk)="bulk($event)"
                    (expand)="expand(g.account)"
                  />
                }
              </section>
            } @empty {
              <p class="muted small">{{ 'No account has a backup in the folder yet.' | t }}</p>
            }
            @if (pruneCount()) {
              <p class="notice warn small" role="status">
                {{ plural(pruneCount(), 'selected session') }}: {{ pruneWarning | t }}
              </p>
            }
            <label class="check">
              <input
                type="checkbox"
                [checked]="trust() && canDangerous()"
                [disabled]="busy() || !canDangerous()"
                (change)="trust.set($any($event.target).checked)"
              />
              <span>
                {{
                  'Also trust the folders of new sessions on this {computer}'
                    | t: { computer: computer() }
                }}
                <span class="muted small hint-line">{{
                  'A trusted folder runs its project hooks and MCP servers without asking.' | t
                }}</span>
                @if (!canDangerous()) {
                  <span class="small hint-line blocked">{{ dangerousReason | t }}</span>
                }
              </span>
            </label>
            <label class="check force">
              <input
                type="checkbox"
                [checked]="forcing()"
                [disabled]="busy() || !canDangerous()"
                (change)="setForce($any($event.target).checked)"
              />
              <span>
                {{
                  "Force: the backup's copy replaces this {computer}'s"
                    | t: { computer: computer() }
                }}
                <span class="muted small hint-line">{{
                  "Also where it changed here or is newer here. This {computer}'s copies are kept for undo, for seven days; sessions only on this {computer} stay, and running ones are skipped."
                    | t: { computer: computer() }
                }}</span>
                @if (!canDangerous()) {
                  <span class="small hint-line blocked">{{ dangerousReason | t }}</span>
                }
              </span>
            </label>
            @if (error()) {
              <p class="notice error small" role="alert">{{ error() }}</p>
            }
            <div class="row actions">
              <button
                class="btn"
                type="button"
                [class.primary]="!forcing()"
                [class.danger]="forcing()"
                [disabled]="!selected().size || busy()"
                (click)="forcing() ? step.set('confirm') : pull()"
              >
                {{ pullLabel() }}
              </button>
            </div>
          }
          @case ('confirm') {
            <div class="notice warn small confirm" role="alert">
              <p>
                <strong>{{ 'Force pull.' | t }}</strong>
                @if (counts().replaced) {
                  {{
                    "The backup's copy replaces this {computer}'s copy of"
                      | t: { computer: computer() }
                  }}
                  <strong>{{ plural(counts().replaced, 'session') }}</strong>
                  @if (kinds()) {
                    ({{ kinds() }})
                  }
                  {{ '— also where it changed here or is newer here.' | t }}
                } @else {
                  {{
                    "None of the selected sessions has a copy here that differs from the backup's."
                      | t
                  }}
                }
                @if (counts().added) {
                  {{ addedText() }}
                }
              </p>
              <p>
                {{
                  "Sessions only on this {computer} stay as they are, and sessions running here are skipped. This {computer}'s copies are kept for seven days: Undo puts them back."
                    | t: { computer: computer() }
                }}
              </p>
            </div>
            @if (error()) {
              <p class="notice error small" role="alert">{{ error() }}</p>
            }
            <div class="row actions">
              <button class="btn danger" type="button" [disabled]="busy()" (click)="pull()">
                {{ busy() ? ('Starting…' | t) : ('Force pull' | t) }}
              </button>
              <button
                class="btn ghost"
                type="button"
                [disabled]="busy()"
                (click)="step.set('list')"
              >
                {{ 'Back' | t }}
              </button>
            </div>
          }
          @case ('run') {
            <div class="runs">
              @for (r of runs(); track r.account) {
                @if (r.job; as j) {
                  <mc-data-job [job]="j" (changed)="setJob(r.account, $event)">
                    <mc-job-details [job]="j" />
                  </mc-data-job>
                } @else {
                  <p class="muted small">{{ '{label}: starting…' | t: { label: r.label } }}</p>
                }
              }
            </div>
            @for (m of skipped(); track m) {
              <p class="muted small">{{ m }}</p>
            }
            @if (error()) {
              <p class="notice error small" role="alert">{{ error() }}</p>
            }
            <div class="row actions">
              <button class="btn ghost" type="button" (click)="closed.emit()">
                {{ running() ? ('Close (it goes on)' | t) : ('Close' | t) }}
              </button>
            </div>
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
      .grp:first-child h3 {
        margin-top: 0;
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
      .confirm p {
        margin: 0 0 0.4rem;
        line-height: 1.5;
      }
      .confirm p:last-child {
        margin-bottom: 0;
      }
      .actions {
        margin-top: 0.9rem;
      }
      .runs {
        display: flex;
        flex-direction: column;
        gap: 0.9rem;
      }
    `,
  ],
})
export class PullSheetComponent {
  private api = inject(ApiService);
  private jobs = inject(DataJobsService);
  /** A folder's trust needs the dangerous right (docs/ACCOUNTS.md §13.9). */
  protected readonly canDangerous = inject(AuthService).canDangerous;
  private readonly hostOs = inject(AuthService).hostOs;
  /** The desktop's computer in a sentence ("this {computer}"): "Mac", "PC"
   * (Vietnamese "máy Mac", "máy PC"). */
  protected readonly computer = computed(() => tr(machineNoun(this.hostOs())));
  protected readonly dangerousReason = DANGEROUS_REASON;

  readonly open = input(false);
  /** One account's set; null: every account's. */
  readonly account = input<string | null>(null);
  /** Every account's set (those of their own are pulled from). */
  readonly sets = input<BackupSet[]>([]);
  /** The sessions to start selected (those dropped on their account), when
   * a pull takes them; else everything a pull would bring. */
  readonly ids = input<readonly string[] | null>(null);
  readonly closed = output<void>();
  /** A pull started: its job id, for the page's list. */
  readonly started = output<string>();

  protected readonly step = signal<PullStep>('loading');
  protected readonly busy = signal(false);
  protected readonly error = signal('');
  protected readonly groups = signal<PullGroup[]>([]);
  protected readonly selected = signal<ReadonlySet<string>>(new Set());
  protected readonly trust = signal(false);
  /** A force pull (needs the dangerous right). */
  protected readonly force = signal(false);
  protected readonly forcing = computed(() => this.force() && this.canDangerous());
  protected readonly expanded = signal<ReadonlySet<string>>(new Set());
  protected readonly runs = signal<PullRun[]>([]);
  /** Sets a pull passed over, in the server's words. */
  protected readonly skipped = signal<string[]>([]);
  private following?: AbortController;
  private loadSeq = 0;

  protected readonly pruneWarning = PRUNE_WARNING;
  protected readonly plural = plural;

  protected readonly title = computed(() => {
    const id = this.account();
    const name = id ? this.sets().find((s) => s.account === id)?.name : '';
    return name ? tr('Pull {name} from the backup', { name }) : tr('Pull from the backups');
  });
  /** The button that starts it: "Pull 3 sessions", "Force pull 3 sessions…". */
  protected readonly pullLabel = computed(() => {
    if (this.busy()) return tr('Starting…');
    const what = plural(this.selected().size, 'session');
    return this.forcing() ? tr('Force pull {what}…', { what }) : tr('Pull {what}', { what });
  });
  protected readonly pruneCount = computed(() =>
    this.groups().reduce(
      (n, g) => n + prunedOf(g.index?.sessions ?? [], this.selected()).length,
      0,
    ),
  );
  protected readonly running = computed(() => this.runs().some((r) => r.job?.state === 'running'));
  /** What the selected sessions of a force pull are, counted. */
  protected readonly counts = computed<ForceCounts>(() =>
    forceCounts(
      this.groups().flatMap((g) => g.index?.sessions ?? []),
      this.selected(),
    ),
  );
  protected readonly kinds = computed(() => forceKinds(this.counts()));
  /** "1 session new to this Mac comes too." */
  protected readonly addedText = computed(() => {
    const n = this.counts().added;
    const computer = this.computer();
    return n === 1
      ? tr('{n} session new to this {computer} comes too.', { n, computer })
      : tr('{n} sessions new to this {computer} come too.', { n, computer });
  });

  constructor() {
    // Every opening starts over.
    effect(() => {
      if (!this.open()) return;
      untracked(() => {
        this.following?.abort();
        this.trust.set(false);
        this.force.set(false);
        this.runs.set([]);
        this.skipped.set([]);
        this.error.set('');
        void this.load();
      });
    });
    inject(DestroyRef).onDestroy(() => this.following?.abort());
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

  protected expand(account: string): void {
    this.expanded.update((prev) => new Set(prev).add(account));
  }

  /** Force on or off: the selection starts over with what that pull takes. */
  protected setForce(on: boolean): void {
    this.force.set(on);
    this.selected.set(this.preselected(on && this.canDangerous()));
  }

  private preselected(force: boolean): Set<string> {
    const picked = new Set<string>();
    for (const g of this.groups()) {
      for (const sid of preselect(g.index?.sessions ?? [], 'pull', force)) picked.add(sid);
    }
    return picked;
  }

  protected setJob(account: string, job: JobView): void {
    this.runs.update((list) => list.map((r) => (r.account === account ? { ...r, job } : r)));
  }

  /** Each set's index, compared with this Mac's copy of its account. */
  async load(): Promise<void> {
    const seq = ++this.loadSeq;
    this.step.set('loading');
    this.expanded.set(new Set());
    const id = this.account();
    // Every set of its own that is there; one whose key is missing too, so
    // its group says why it can't be pulled.
    const sets = ownSets(this.sets()).filter((s) =>
      id ? s.account === id : s.exists || s.keyMissing,
    );
    const groups = await Promise.all(
      sets.map(async (s): Promise<PullGroup> => {
        try {
          const ix = normalizeBackupIndex(await this.api.getBackupIndex(s.account));
          if (!ix) {
            throw new Error(
              tr('The {computer} sent a backup list this app could not read.', {
                computer: this.computer(),
              }),
            );
          }
          return { account: s.account, name: s.name, index: ix, error: '' };
        } catch (e) {
          return { account: s.account, name: s.name, index: null, error: backupErrorText(e) };
        }
      }),
    );
    if (seq !== this.loadSeq) return;
    this.groups.set(groups);
    const wanted = new Set(this.ids() ?? []);
    const dropped = groups
      .flatMap((g) => g.index?.sessions ?? [])
      .filter((s) => wanted.has(s.id) && pullTakesSession(s, false))
      .map((s) => s.id);
    this.selected.set(dropped.length ? new Set(dropped) : this.preselected(this.forcing()));
    this.step.set('list');
  }

  /** One pull per account with something selected (each its own job). */
  protected async pull(): Promise<void> {
    if (!this.selected().size || this.busy()) return;
    const force = this.forcing();
    const wanted = this.groups()
      .map((g) => ({
        g,
        ids: (g.index?.sessions ?? [])
          .filter((s) => pullTakesSession(s, force) && this.selected().has(s.id))
          .map((s) => s.id),
      }))
      .filter((w) => w.ids.length > 0);
    if (!wanted.length) return;
    this.busy.set(true);
    this.error.set('');
    const runs: PullRun[] = [];
    const firsts: (JobView | string)[] = [];
    const skipped: string[] = [];
    const failures: string[] = [];
    for (const { g, ids } of wanted) {
      const label = force
        ? tr('Force pull of {name}', { name: g.name })
        : tr('Pull of {name}', { name: g.name });
      try {
        const res = await this.api.pullBackup({
          account: g.account,
          ids,
          ...(this.trust() && this.canDangerous() ? { trust: true } : {}),
          ...(force ? { force: true } : {}),
        });
        for (const m of normalizePullSkips(res?.skipped)) {
          const name = this.sets().find((s) => s.account === m.account)?.name;
          skipped.push(name ? `${name}: ${sentence(m.message)}` : sentence(m.message));
        }
        const raw = (res?.jobs ?? [])[0];
        const first = normalizeJob(raw) ?? jobIdOf(raw);
        if (!first) continue;
        runs.push({ account: g.account, label, job: typeof first === 'string' ? null : first });
        firsts.push(first);
        this.started.emit(typeof first === 'string' ? first : first.id);
      } catch (e) {
        failures.push(`${g.name}: ${backupErrorText(e)}`);
      }
    }
    this.busy.set(false);
    if (!runs.length) {
      this.error.set(failures.join(' ') || tr('Nothing was pulled.'));
      if (this.step() === 'confirm') this.step.set('list');
      return;
    }
    this.runs.set(runs);
    this.skipped.set(skipped);
    this.error.set(failures.join(' '));
    this.step.set('run');
    this.following?.abort();
    const ctrl = new AbortController();
    this.following = ctrl;
    await Promise.all(
      firsts.map((first, i) =>
        this.jobs
          .follow(first, (j) => this.setJob(runs[i].account, j), ctrl.signal)
          .catch((e) => {
            if (!ctrl.signal.aborted) {
              this.error.set(
                tr("Can't follow {label} any more: {error}", {
                  label: runs[i].label,
                  error: backupErrorText(e),
                }),
              );
            }
          }),
      ),
    );
  }
}
