import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  WritableSignal,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
} from '@angular/core';
import { FormsModule } from '@angular/forms';

import { DEFAULT_ACCOUNT, sharersOf } from '../core/accounts';
import { AccountsService } from '../core/accounts.service';
import { ApiService } from '../core/api.service';
import { AuthService } from '../core/auth.service';
import { normalizeBackupIndex } from '../core/backup';
import { normalizeJob, plural } from '../core/data-jobs';
import { DataJobsService } from '../core/data-jobs.service';
import { BackupIndex, JobView, SessionSummary, TransferMode } from '../core/models';
import {
  CUSTOMIZATIONS,
  ChecklistRow,
  CustomizationItem,
  DANGEROUS_REASON,
  DataLocation,
  MovePlan,
  MoveWhat,
  accountHome,
  accountLocation,
  anyTarget,
  defaultTarget,
  indexRow,
  locationFromKey,
  locationGroups,
  locationHint,
  locationKey,
  locationName,
  modeHint,
  modeProblem,
  moveBody,
  moveDirection,
  moveErrorText,
  newSetKey,
  newSetStorage,
  normalizeMovePlan,
  projectRows,
  sessionRow,
  setLocation,
  setNameProblem,
  trustApplies,
  whatProblem,
} from '../core/move-data';
import { RealtimeService } from '../core/realtime.service';
import { StorageService } from '../core/storage.service';
import { DataJobComponent } from './data-job.component';
import { IconComponent } from './icon.component';
import { MoveChecklistComponent } from './move-checklist.component';
import { MovePlanComponent } from './move-plan.component';
import { SheetComponent } from './sheet.component';

/** What the sheet opens with. */
export interface MoveStart {
  /** Where things come from: an account or a set of a storage; null for
   * conversations that each go from where they are (Sessions › Select).
   * Absent: the Default account. */
  from?: DataLocation | null;
  /** Where they go (absent: the first place they can go). */
  to?: DataLocation | null;
  what?: MoveWhat;
  /** Conversations and projects to start ticked. */
  sessions?: readonly string[];
  projects?: readonly string[];
  /** Conversations the opener has at hand (titles for the checklist). */
  known?: readonly SessionSummary[];
}

/** Where the sheet is: choosing, looking at the dry run, running. */
export type MoveDataStep = 'pick' | 'plan' | 'run';

/** Conversations read per page for the checklists. */
const PAGE = 200;

const WHATS: readonly { value: MoveWhat; label: string }[] = [
  { value: 'all', label: 'Everything' },
  { value: 'projects', label: 'Projects' },
  { value: 'sessions', label: 'Conversations' },
  { value: 'customizations', label: 'Customizations' },
];

interface IndexState {
  key: string;
  ix: BackupIndex | null;
  loading: boolean;
  error: string;
}

/**
 * Move data (docs/ACCOUNTS.md §13.6): take conversations, projects,
 * everything, or an account's customizations from one place to another —
 * between accounts (an *assign* when both use the shared folder), into a
 * storage's set (a backup, or an archive when moved), out of one (a
 * restore), or between two sets. *From* and *To* list the accounts and the
 * storages with their sets (*New set…* as a target); conversations and
 * projects are searchable checklists (a set's come from its index). Move
 * or Copy, as far as the way allows; a folder's trust and the replace
 * option; then a dry run (what goes, what is skipped and why, warnings),
 * the jobs with their progress, and Undo. The desktop and paired phones
 * alike: a folder's trust and the settings and MCP customizations need the
 * dangerous right, and say so on a device without it.
 */
@Component({
  selector: 'mc-move-data-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    FormsModule,
    DataJobComponent,
    IconComponent,
    MoveChecklistComponent,
    MovePlanComponent,
    SheetComponent,
  ],
  template: `
    <mc-sheet [open]="open()" title="Move or copy data" (closed)="closed.emit()">
      @if (open()) {
        @if (!ready()) {
          @if (loadError()) {
            <p class="notice error small" role="alert">{{ loadError() }}</p>
            <div class="row actions">
              <button class="btn" type="button" (click)="reload()">Try again</button>
            </div>
          } @else {
            <p class="muted small">Loading…</p>
          }
        } @else if (!somewhere()) {
          <p class="muted small nowhere">
            There is nowhere to move it yet: add an account with a folder of its own on the Accounts
            page, or a storage on Backup &amp; sync.
          </p>
        } @else {
          @switch (step()) {
            @case ('pick') {
              <div class="places">
                <label class="field">
                  <span class="field-label">From</span>
                  <select
                    name="from"
                    [disabled]="busy()"
                    aria-describedby="md-from-hint"
                    (change)="pickFrom($any($event.target).value)"
                  >
                    @if (offerEverywhere()) {
                      <option value="" [selected]="fromKey() === ''">Where each one is</option>
                    }
                    @for (g of fromGroups(); track g.label) {
                      <optgroup [label]="g.label">
                        @for (o of g.options; track o.key) {
                          <option
                            [value]="o.key"
                            [disabled]="o.disabled"
                            [selected]="o.key === fromKey()"
                          >
                            {{ o.label }}
                          </option>
                        }
                      </optgroup>
                    }
                  </select>
                  <span class="field-hint" id="md-from-hint">{{ fromHint() }}</span>
                </label>
                <label class="field">
                  <span class="field-label">To</span>
                  <select
                    name="to"
                    [disabled]="busy()"
                    aria-describedby="md-to-hint"
                    (change)="pickTo($any($event.target).value)"
                  >
                    @if (!toKeyInEffect()) {
                      <option value="" [selected]="true" disabled>Choose where it goes</option>
                    }
                    @for (g of toGroups(); track g.label) {
                      <optgroup [label]="g.label">
                        @for (o of g.options; track o.key) {
                          <option
                            [value]="o.key"
                            [disabled]="o.disabled"
                            [selected]="o.key === toKeyInEffect()"
                          >
                            {{ o.label }}
                          </option>
                        }
                      </optgroup>
                    }
                  </select>
                  <span class="field-hint" id="md-to-hint">{{ toHint() }}</span>
                </label>
                @if (newSetFor()) {
                  <label class="field">
                    <span class="field-label">Name of the new set</span>
                    <input
                      name="newSet"
                      maxlength="40"
                      autocomplete="off"
                      autocapitalize="off"
                      spellcheck="false"
                      placeholder="Archive 2026"
                      [disabled]="busy()"
                      [ngModel]="newSetName()"
                      (ngModelChange)="newSetName.set($event); error.set('')"
                    />
                    @if (newSetName().trim() && newSetProblem()) {
                      <span class="error-text small">{{ newSetProblem() }}</span>
                    }
                  </label>
                }
              </div>
              @if (storageError()) {
                <p class="muted small">Storages could not be read: {{ storageError() }}</p>
              }

              <h3 id="md-what">What</h3>
              <div class="segmented whatseg" role="group" aria-labelledby="md-what">
                @for (w of whatOptions(); track w.value) {
                  <button
                    type="button"
                    [attr.aria-pressed]="what() === w.value"
                    [disabled]="busy() || (!!w.problem && what() !== w.value)"
                    [attr.title]="w.problem || null"
                    (click)="setWhat(w.value)"
                  >
                    {{ w.label }}
                  </button>
                }
              </div>
              @if (whatError()) {
                <p class="notice warn small" role="status">{{ whatError() }}</p>
              }

              @switch (what()) {
                @case ('all') {
                  <p class="small allline">{{ allLine() }}</p>
                }
                @case ('sessions') {
                  <mc-move-checklist
                    noun="conversations"
                    [label]="'Conversations of ' + fromName()"
                    [rows]="sessionRows()"
                    [selected]="pickedSessions()"
                    [loading]="sourceLoading()"
                    [error]="sourceError()"
                    [more]="sourceMore()"
                    moreText="Load earlier conversations"
                    [emptyText]="emptyText()"
                    [disabled]="busy()"
                    (flip)="flip(pickedSessions, $event)"
                    (bulk)="bulkSet(pickedSessions, $event)"
                    (selectAll)="bulkSet(pickedSessions, { ids: $event, on: true })"
                    (loadMore)="loadMore()"
                  />
                }
                @case ('projects') {
                  <mc-move-checklist
                    noun="projects"
                    [label]="'Projects of ' + fromName()"
                    [rows]="projectRowsShown()"
                    [selected]="pickedProjects()"
                    [loading]="sourceLoading()"
                    [error]="sourceError()"
                    [more]="sourceMore()"
                    moreText="Look through earlier conversations"
                    emptyText="No projects here."
                    [disabled]="busy()"
                    (flip)="flip(pickedProjects, $event)"
                    (bulk)="bulkSet(pickedProjects, $event)"
                    (selectAll)="bulkSet(pickedProjects, { ids: $event, on: true })"
                    (loadMore)="loadMore()"
                  />
                  <p class="muted small">
                    A project takes every conversation in its folder and its prompt history.
                  </p>
                }
                @case ('customizations') {
                  <ul class="custom" aria-label="Customizations">
                    @for (c of customRows(); track c.item) {
                      <li>
                        <label class="pick">
                          <input
                            type="checkbox"
                            [checked]="pickedCustom().has(c.item) && !c.blocked"
                            [disabled]="busy() || c.blocked"
                            (change)="flipCustom(c.item)"
                          />
                          <span class="body">
                            <span class="title">{{ c.label }}</span>
                            <span class="muted small">{{ c.hint }}</span>
                            @if (c.blocked) {
                              <span class="small blocked">{{ dangerousReason }}</span>
                            }
                          </span>
                        </label>
                      </li>
                    }
                  </ul>
                  <p class="muted small">Plugins are never copied: install them in the account.</p>
                }
              }

              <h3 id="md-how">How</h3>
              <div class="segmented" role="group" aria-labelledby="md-how">
                <button
                  type="button"
                  [attr.aria-pressed]="mode() === 'copy'"
                  [disabled]="busy() || !!copyProblem()"
                  (click)="setMode('copy')"
                >
                  <mc-icon name="copy" [size]="14" /> Copy
                </button>
                <button
                  type="button"
                  [attr.aria-pressed]="mode() === 'move'"
                  [disabled]="busy() || !!moveProblem()"
                  (click)="setMode('move')"
                >
                  <mc-icon name="move" [size]="14" /> Move
                </button>
              </div>
              @if (modeLine()) {
                <p class="muted small hint">{{ modeLine() }}</p>
              }
              @if (otherModeProblem()) {
                <p class="muted small hint">{{ otherModeProblem() }}</p>
              }

              @if (trustShown()) {
                <label class="check">
                  <input
                    type="checkbox"
                    [checked]="trust() && canDangerous()"
                    [disabled]="busy() || !canDangerous()"
                    (change)="trust.set($any($event.target).checked)"
                  />
                  <span>
                    Also trust their folders for {{ toName() }}
                    <span class="muted small hint-line"
                      >A trusted folder runs its project hooks and MCP servers without asking.</span
                    >
                    @if (!canDangerous()) {
                      <span class="small hint-line blocked">{{ dangerousReason }}</span>
                    }
                  </span>
                </label>
              }
              @if (what() === 'customizations') {
                <label class="check">
                  <input
                    type="checkbox"
                    [checked]="replace()"
                    [disabled]="busy()"
                    (change)="replace.set($any($event.target).checked)"
                  />
                  <span>
                    Replace what {{ toName() }} has
                    <span class="muted small hint-line"
                      >Its own copy goes into the undo folder first. Without this, only what it
                      lacks is added.</span
                    >
                  </span>
                </label>
              }

              @if (error()) {
                <p class="notice error small" role="alert">{{ error() }}</p>
              }
              <div class="row actions">
                <button
                  class="btn primary"
                  type="button"
                  [disabled]="!!blocker() || busy()"
                  (click)="dryRun()"
                >
                  {{ busy() ? 'Checking…' : 'Check what it does' }}
                </button>
                @if (blocker() && !busy()) {
                  <span class="muted small">{{ blocker() }}</span>
                }
              </div>
            }

            @case ('plan') {
              @if (plan(); as p) {
                <p class="route small muted">
                  {{ modeWord() }} · from {{ fromName() }} to {{ toName() }}
                </p>
                <mc-move-plan
                  [plan]="p"
                  [direction]="direction()"
                  [toName]="toName()"
                  [fromName]="fromName()"
                  [titles]="titles()"
                />
              }
              @if (error()) {
                <p class="notice error small" role="alert">{{ error() }}</p>
              }
              <div class="row actions">
                <button
                  class="btn primary"
                  type="button"
                  [disabled]="busy() || !runnable()"
                  (click)="run()"
                >
                  <mc-icon [name]="mode() === 'move' ? 'move' : 'copy'" [size]="14" />
                  {{ busy() ? 'Starting…' : runLabel() }}
                </button>
                <button class="btn ghost" type="button" [disabled]="busy()" (click)="back()">
                  Back
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
                @if (anyFailed()) {
                  <button class="btn" type="button" (click)="back()">Back</button>
                }
                <button class="btn ghost" type="button" (click)="closed.emit()">
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
      h3 {
        margin: 1rem 0 0.4rem;
        font-size: 0.9rem;
      }
      .places {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(13rem, 1fr));
        gap: 0.6rem 0.75rem;
      }
      .places .field-hint {
        overflow-wrap: anywhere;
        line-height: 1.4;
      }
      .whatseg {
        display: flex;
        flex-wrap: wrap;
      }
      @media (max-width: 479px) {
        .whatseg {
          display: grid;
          grid-template-columns: 1fr 1fr;
          width: 100%;
        }
        .whatseg button {
          justify-content: center;
        }
      }
      .allline {
        margin: 0.6rem 0 0;
        line-height: 1.5;
      }
      mc-move-checklist {
        margin-top: 0.6rem;
      }
      .custom {
        margin: 0.6rem 0 0.3rem;
        padding: 0;
        list-style: none;
      }
      .custom > li {
        border-top: 1px solid var(--border);
      }
      .pick {
        display: flex;
        align-items: flex-start;
        gap: 0.6rem;
        padding: 0.45rem 0;
        min-height: var(--tap);
        cursor: pointer;
      }
      .pick input {
        margin-top: 0.2rem;
      }
      .body {
        display: flex;
        flex-direction: column;
        gap: 0.05rem;
        min-width: 0;
      }
      .title {
        font-weight: 500;
      }
      .blocked {
        color: var(--amber);
      }
      .hint {
        margin: 0.35rem 0 0;
        line-height: 1.5;
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
      .route {
        margin: 0;
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
export class MoveDataSheetComponent {
  private api = inject(ApiService);
  private jobsSvc = inject(DataJobsService);
  private realtime = inject(RealtimeService);
  protected readonly accounts = inject(AccountsService);
  protected readonly storage = inject(StorageService);
  protected readonly canDangerous = inject(AuthService).canDangerous;

  readonly open = input(false);
  readonly start = input<MoveStart | null>(null);
  readonly closed = output<void>();

  protected readonly dangerousReason = DANGEROUS_REASON;
  protected readonly step = signal<MoveDataStep>('pick');
  protected readonly busy = signal(false);
  protected readonly error = signal('');
  /** The From pick ('' for "where each one is"). */
  protected readonly fromKey = signal('');
  /** The To pick ('' until one is picked: then the first place it can go). */
  protected readonly toKey = signal('');
  protected readonly newSetName = signal('');
  protected readonly what = signal<MoveWhat>('sessions');
  protected readonly pickedSessions = signal<ReadonlySet<string>>(new Set());
  protected readonly pickedProjects = signal<ReadonlySet<string>>(new Set());
  protected readonly pickedCustom = signal<ReadonlySet<CustomizationItem>>(new Set());
  private readonly modePick = signal<TransferMode>('copy');
  protected readonly trust = signal(false);
  protected readonly replace = signal(false);
  protected readonly plan = signal<MovePlan | null>(null);
  protected readonly jobs = signal<JobView[]>([]);
  private readonly started = signal<MoveStart | null>(null);

  /** Conversations read for the checklists (every account's, newest first). */
  private readonly pages = signal<SessionSummary[]>([]);
  private readonly pagesCursor = signal<string | null | undefined>(undefined);
  private readonly pagesLoading = signal(false);
  private readonly pagesError = signal('');
  /** The index of the set things come from. */
  private readonly index = signal<IndexState | null>(null);
  private following?: AbortController;

  protected readonly list = this.accounts.list;
  protected readonly storages = computed(() => this.storage.list() ?? []);
  protected readonly ready = computed(
    () => !!this.accounts.view() && (this.storage.list() !== null || !!this.storage.loadError()),
  );
  protected readonly loadError = computed(() =>
    this.accounts.view() ? '' : this.accounts.loadError(),
  );
  /** Another account, or a storage, to move things to. */
  protected readonly somewhere = computed(() => anyTarget(this.list(), this.storages()));
  protected readonly storageError = computed(() =>
    this.storage.list() === null ? this.storage.loadError() : '',
  );

  protected readonly offerEverywhere = computed(() => this.started()?.from === null);
  protected readonly everywhere = computed(() => this.fromKey() === '');
  protected readonly fromLoc = computed(() => locationFromKey(this.fromKey()));
  protected readonly fromGroups = computed(() =>
    locationGroups(this.list(), this.storages(), 'from'),
  );
  protected readonly toGroups = computed(() =>
    locationGroups(this.list(), this.storages(), 'to').map((g) => ({
      ...g,
      options: g.options.map((o) => (o.key === this.fromKey() ? { ...o, disabled: true } : o)),
    })),
  );
  /** The To in effect: the pick, else the first place it can go. */
  protected readonly toKeyInEffect = computed(() => {
    const picked = this.toKey();
    if (picked) return picked;
    const from = this.fromLoc() ?? this.singleSource();
    return locationKey(defaultTarget(from, this.list(), this.storages()));
  });
  /** "New set…" is picked: the storage it goes in. */
  protected readonly newSetFor = computed(() => newSetStorage(this.toKeyInEffect()));
  protected readonly newSetProblem = computed(() =>
    setNameProblem(this.newSetName(), this.storage.byId(this.newSetFor())),
  );
  protected readonly toLoc = computed<DataLocation | null>(() => {
    const store = this.newSetFor();
    if (store) return setLocation(store, this.newSetName().trim());
    return locationFromKey(this.toKeyInEffect());
  });
  protected readonly direction = computed(() => {
    const from = this.everywhere() ? null : this.fromLoc();
    const to = this.toLoc();
    // Each from where it is, and every one in the target's folder already:
    // an assign (nothing moves; a copy there would be the same conversation).
    if (!from && to?.kind === 'account') {
      const target = accountHome(this.list(), to.account);
      const sources = [...this.pickedAccounts()];
      if (sources.length && sources.every((a) => a && accountHome(this.list(), a) === target)) {
        return 'assign';
      }
    }
    return moveDirection(from, to, this.list());
  });

  protected readonly fromName = computed(() =>
    this.everywhere()
      ? 'their accounts'
      : locationName(this.fromLoc(), this.list(), this.storages()),
  );
  protected readonly toName = computed(() => {
    const store = this.newSetFor();
    if (store) {
      const label = this.storage.byId(store)?.label ?? 'the storage';
      return `${label} · ${this.newSetName().trim() || 'the new set'}`;
    }
    return locationName(this.toLoc(), this.list(), this.storages(), 'where it goes');
  });
  protected readonly fromHint = computed(() =>
    locationHint(this.everywhere() ? null : this.fromLoc(), this.list(), this.storages()),
  );
  protected readonly toHint = computed(() => {
    const store = this.newSetFor();
    if (store && !this.newSetName().trim()) {
      return `A new set in ${this.storage.byId(store)?.label ?? 'the storage'}: name it below.`;
    }
    return this.toLoc() ? locationHint(this.toLoc(), this.list(), this.storages()) : '';
  });

  protected readonly whatOptions = computed(() =>
    WHATS.map((w) => ({
      ...w,
      problem: whatProblem(w.value, this.direction(), this.everywhere()),
    })),
  );
  protected readonly whatError = computed(
    () => this.whatOptions().find((w) => w.value === this.what())?.problem ?? '',
  );
  protected readonly copyProblem = computed(() =>
    modeProblem('copy', this.direction(), this.what()),
  );
  protected readonly moveProblem = computed(() =>
    modeProblem('move', this.direction(), this.what()),
  );
  /** The pick, unless this way can't do it and can do the other. */
  protected readonly mode = computed<TransferMode>(() => {
    const pick = this.modePick();
    const other: TransferMode = pick === 'copy' ? 'move' : 'copy';
    const problem = pick === 'copy' ? this.copyProblem() : this.moveProblem();
    const otherProblem = pick === 'copy' ? this.moveProblem() : this.copyProblem();
    return problem && !otherProblem ? other : pick;
  });
  protected readonly modeWord = computed(() => (this.mode() === 'move' ? 'Move' : 'Copy'));
  protected readonly modeLine = computed(() =>
    this.direction() === 'same'
      ? ''
      : modeHint(
          this.mode(),
          this.direction(),
          this.what(),
          this.fromName(),
          this.toName(),
          this.replace(),
        ),
  );
  /** Why the mode not in effect can't be used ('' when it can). */
  protected readonly otherModeProblem = computed(() => {
    if (this.direction() === 'same') return '';
    const other = this.mode() === 'copy' ? this.moveProblem() : this.copyProblem();
    return other;
  });
  protected readonly trustShown = computed(
    () => this.toLoc()?.kind === 'account' && trustApplies(this.direction(), this.what()),
  );
  protected readonly customRows = computed(() =>
    CUSTOMIZATIONS.map((c) => ({ ...c, blocked: c.dangerous && !this.canDangerous() })),
  );

  // ─────────────────────────────────────── What the checklists list

  /** Conversations at hand: the opener's, the live store's and the pages
   * read, each once (the live store's is the freshest). */
  private readonly knownSessions = computed(() => {
    const out = new Map<string, SessionSummary>();
    for (const s of this.started()?.known ?? []) out.set(s.id, s);
    for (const s of this.pages()) out.set(s.id, s);
    for (const s of this.realtime.sessions()) out.set(s.id, s);
    return out;
  });
  /** The accounts the ticked conversations are recorded as ('' for one
   * not known here). */
  private readonly pickedAccounts = computed(() => {
    const known = this.knownSessions();
    return new Set([...this.pickedSessions()].map((id) => this.accountIdOf(known.get(id))));
  });
  /** The account all ticked conversations are in, when they share one. */
  private readonly singleSource = computed<DataLocation | null>(() => {
    const accounts = this.pickedAccounts();
    const only = accounts.size === 1 ? [...accounts][0] : null;
    return only ? accountLocation(only) : null;
  });
  private readonly indexKey = computed(() => {
    const f = this.fromLoc();
    return f?.kind === 'storage' ? locationKey(f) : '';
  });
  /** The conversations of From (an account's, a set's, or the ticked ones). */
  private readonly sourceSessions = computed(() => {
    if (this.everywhere()) {
      const known = this.knownSessions();
      const ids = this.started()?.sessions ?? [];
      return ids.map((id) => known.get(id)).filter((s): s is SessionSummary => !!s);
    }
    const f = this.fromLoc();
    if (f?.kind !== 'account') return [];
    // The ones the sheet opened with first, then the newest.
    const first = new Set(this.started()?.sessions ?? []);
    return [...this.knownSessions().values()]
      .filter((s) => this.accountIdOf(s) === f.account)
      .sort(
        (a, b) =>
          Number(first.has(b.id)) - Number(first.has(a.id)) ||
          (b.lastActivityAt ?? 0) - (a.lastActivityAt ?? 0),
      );
  });
  protected readonly sessionRows = computed<ChecklistRow[]>(() => {
    const f = this.fromLoc();
    if (f?.kind === 'storage') {
      const st = this.index();
      return st?.key === this.indexKey() && st.ix ? st.ix.sessions.map(indexRow) : [];
    }
    const rows = this.sourceSessions().map(sessionRow);
    // Ticked ones not read yet still show (by id).
    const shown = new Set(rows.map((r) => r.id));
    const extra = [...this.pickedSessions()]
      .filter((id) => !shown.has(id))
      .map((id) => ({ id, title: id.slice(0, 8), sub: '', tag: '', haystack: id }));
    return [...rows, ...extra];
  });
  protected readonly projectRowsShown = computed<ChecklistRow[]>(() => {
    const f = this.fromLoc();
    const keep = [...(this.started()?.projects ?? [])];
    if (f?.kind === 'storage') {
      const st = this.index();
      const ix = st?.key === this.indexKey() ? st.ix : null;
      return projectRows(ix?.sessions ?? [], keep);
    }
    return projectRows(this.sourceSessions(), keep);
  });
  protected readonly sourceLoading = computed(() =>
    this.fromLoc()?.kind === 'storage' ? !!this.index()?.loading : this.pagesLoading(),
  );
  protected readonly sourceError = computed(() =>
    this.fromLoc()?.kind === 'storage' ? (this.index()?.error ?? '') : this.pagesError(),
  );
  protected readonly sourceMore = computed(
    () => this.fromLoc()?.kind === 'account' && !!this.pagesCursor(),
  );
  protected readonly emptyText = computed(() =>
    this.fromLoc()?.kind === 'storage'
      ? 'This set holds no conversations.'
      : `No conversations of ${this.fromName()} here yet.`,
  );
  protected readonly allLine = computed(() => {
    const f = this.fromLoc();
    if (f?.kind === 'storage') {
      const n = this.storage.byId(f.storage)?.sets.find((s) => s.name === f.set)?.sessions;
      return `Everything in ${this.fromName()}${n !== undefined ? ` (${plural(n, 'conversation')})` : ''}.`;
    }
    const a = this.accounts.byId(f?.account);
    if (!a) return '';
    if (a.kind === 'token' || sharersOf(this.list(), a.id).length) {
      return `Every conversation recorded as ${a.name}'s (the shared folder holds others' too).`;
    }
    return `Every conversation of ${a.name}, and its prompt history.`;
  });
  /** The conversations' titles, for the dry run's skipped ones. */
  protected readonly titles = computed(() => {
    const out = new Map<string, string>();
    for (const r of this.sessionRows()) out.set(r.id, r.title);
    return out;
  });

  /** Why it can't be checked yet ('' when it can). */
  protected readonly blocker = computed(() => {
    const to = this.toLoc();
    if (!to) return 'Choose where it goes.';
    if (this.newSetFor() && this.newSetProblem()) return this.newSetProblem();
    if (this.direction() === 'same') return 'Pick another place for it to go.';
    if (this.whatError()) return this.whatError();
    const modeProblemNow = this.mode() === 'copy' ? this.copyProblem() : this.moveProblem();
    if (modeProblemNow) return modeProblemNow;
    switch (this.what()) {
      case 'sessions':
        return this.pickedSessions().size ? '' : 'Tick at least one conversation.';
      case 'projects':
        return this.pickedProjects().size ? '' : 'Tick at least one project.';
      case 'customizations':
        return this.allowedCustom().length ? '' : 'Tick at least one customization.';
      default:
        return '';
    }
  });
  private readonly allowedCustom = computed(() =>
    CUSTOMIZATIONS.filter(
      (c) => this.pickedCustom().has(c.item) && (!c.dangerous || this.canDangerous()),
    ).map((c) => c.item),
  );

  protected readonly runnable = computed(() => {
    const p = this.plan();
    return !!p && (p.sessions.length > 0 || p.customizations.some((c) => c.action !== 'skip'));
  });
  protected readonly runLabel = computed(() => {
    const p = this.plan();
    if (!p) return this.modeWord();
    if (!p.sessions.length) {
      const n = p.customizations.filter((c) => c.action !== 'skip').length;
      return n ? `Copy ${plural(n, 'customization')}` : `Nothing to ${this.mode()}`;
    }
    const actions = new Set(p.sessions.map((s) => s.action));
    const one = actions.size === 1 ? [...actions][0] : null;
    const verb =
      one === 'archive'
        ? 'Archive'
        : one === 'assign'
          ? 'Assign'
          : one === 'restore'
            ? 'Restore'
            : this.modeWord();
    return `${verb} ${plural(p.sessions.length, 'conversation')}`;
  });
  protected readonly anyRunning = computed(() => this.jobs().some((j) => j.state === 'running'));
  protected readonly anyFailed = computed(() => this.jobs().some((j) => j.state === 'failed'));

  constructor() {
    // Every opening starts over, with what the opener chose then: a change
    // of it while open (the session moved, its account changed) keeps the
    // jobs on screen.
    let wasOpen = false;
    effect(() => {
      const open = this.open();
      if (open && !wasOpen) untracked(() => this.reset(this.start()));
      wasOpen = open;
    });
    // A set's index, when its conversations are listed (or counted).
    effect(() => {
      const key = this.indexKey();
      const listing = this.what() === 'sessions' || this.what() === 'projects';
      if (!this.open() || !key || !listing) return;
      untracked(() => {
        if (this.index()?.key !== key) void this.loadIndex(key);
      });
    });
    // An account's conversations, when they are listed.
    effect(() => {
      const listing =
        this.open() &&
        this.fromLoc()?.kind === 'account' &&
        (this.what() === 'sessions' || this.what() === 'projects');
      if (!listing) return;
      untracked(() => {
        if (this.pagesCursor() === undefined && !this.pagesLoading()) void this.loadPage();
      });
    });
    inject(DestroyRef).onDestroy(() => this.following?.abort());
  }

  protected reload(): void {
    void this.accounts.refresh();
    void this.storage.refresh();
  }

  protected pickFrom(key: string): void {
    if (key === this.fromKey()) return;
    this.fromKey.set(key);
    this.error.set('');
    // What was ticked belongs to the old place; back at the opener's
    // place, its own picks come back.
    const start = this.started();
    const home = locationKey(
      start?.from === undefined ? accountLocation(DEFAULT_ACCOUNT) : start.from,
    );
    this.pickedSessions.set(new Set(key === home ? (start?.sessions ?? []) : []));
    this.pickedProjects.set(new Set(key === home ? (start?.projects ?? []) : []));
    if (key === '' && this.what() !== 'sessions') this.what.set('sessions');
  }

  protected pickTo(key: string): void {
    this.toKey.set(key);
    this.error.set('');
  }

  protected setWhat(w: MoveWhat): void {
    this.what.set(w);
    this.error.set('');
  }

  protected setMode(m: TransferMode): void {
    this.modePick.set(m);
    this.error.set('');
  }

  protected flip(target: WritableSignal<ReadonlySet<string>>, id: string): void {
    target.update((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
    this.error.set('');
  }

  protected bulkSet(
    target: WritableSignal<ReadonlySet<string>>,
    b: { ids: string[]; on: boolean },
  ): void {
    target.update((prev) => {
      const next = new Set(prev);
      for (const id of b.ids) {
        if (b.on) next.add(id);
        else next.delete(id);
      }
      return next;
    });
    this.error.set('');
  }

  protected flipCustom(item: CustomizationItem): void {
    this.pickedCustom.update((prev) => {
      const next = new Set(prev);
      if (next.has(item)) next.delete(item);
      else next.add(item);
      return next;
    });
    this.error.set('');
  }

  protected loadMore(): void {
    if (this.fromLoc()?.kind === 'account') void this.loadPage();
  }

  protected back(): void {
    this.following?.abort();
    this.step.set('pick');
    this.error.set('');
    this.plan.set(null);
    this.jobs.set([]);
  }

  protected jobChanged(j: JobView): void {
    this.jobs.update((list) => list.map((x) => (x.id === j.id ? j : x)));
  }

  /** The dry run: what it would do, nothing done yet. */
  protected async dryRun(): Promise<void> {
    const body = this.body();
    if (!body || this.busy() || this.blocker()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      const res = await this.api.planMove(body);
      const plan = normalizeMovePlan(res?.plan);
      if (!plan) throw new Error('The Mac sent a plan this app could not read.');
      this.plan.set(plan);
      this.step.set('plan');
    } catch (e) {
      this.error.set(moveErrorText(e));
    } finally {
      this.busy.set(false);
    }
  }

  protected async run(): Promise<void> {
    const body = this.body();
    if (!body || this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    let list: JobView[];
    try {
      const res = await this.api.startMove(body);
      const raw = Array.isArray(res?.jobs) && res.jobs.length ? res.jobs : [res?.job];
      const seen = new Set<string>();
      list = raw
        .map(normalizeJob)
        .filter((j): j is JobView => !!j && !seen.has(j.id) && !!seen.add(j.id));
      if (!list.length) throw new Error('The Mac sent no job this app could read.');
    } catch (e) {
      this.busy.set(false);
      this.error.set(moveErrorText(e));
      return;
    }
    this.busy.set(false);
    this.jobs.set(list);
    this.step.set('run');
    this.following?.abort();
    const ctrl = new AbortController();
    this.following = ctrl;
    const touchesStorage = this.fromLoc()?.kind === 'storage' || this.toLoc()?.kind === 'storage';
    await Promise.all(
      list.map((j) =>
        this.jobsSvc
          .follow(j, (next) => this.jobChanged(next), ctrl.signal)
          .catch((e) => {
            if (!ctrl.signal.aborted)
              this.error.set(`Can't follow it any more: ${moveErrorText(e)}`);
          }),
      ),
    );
    // A set's counts changed.
    if (touchesStorage && !ctrl.signal.aborted) void this.storage.refresh();
  }

  private body() {
    const to = this.toLoc();
    if (!to) return null;
    return moveBody({
      from: this.everywhere() ? null : this.fromLoc(),
      to,
      what: this.what(),
      sessions: [...this.pickedSessions()],
      projects: [...this.pickedProjects()],
      customizations: this.allowedCustom(),
      mode: this.mode(),
      trust: this.trustShown() && this.trust() && this.canDangerous(),
      replaceExisting: this.replace(),
    });
  }

  private reset(start: MoveStart | null): void {
    this.following?.abort();
    this.started.set(start);
    this.step.set('pick');
    this.busy.set(false);
    this.error.set('');
    this.plan.set(null);
    this.jobs.set([]);
    const from = start?.from === undefined ? accountLocation(DEFAULT_ACCOUNT) : start.from;
    this.fromKey.set(locationKey(from));
    // A storage without a set: "New set…" in it.
    const to = start?.to;
    this.toKey.set(
      !to ? '' : to.kind === 'storage' && !to.set ? newSetKey(to.storage) : locationKey(to),
    );
    this.newSetName.set('');
    this.what.set(
      start?.what ??
        (start?.sessions?.length ? 'sessions' : start?.projects?.length ? 'projects' : 'all'),
    );
    this.pickedSessions.set(new Set(start?.sessions ?? []));
    this.pickedProjects.set(new Set(start?.projects ?? []));
    this.pickedCustom.set(new Set());
    this.modePick.set('copy');
    this.trust.set(false);
    this.replace.set(false);
    // Read afresh: a move since may have changed whose they are.
    this.index.set(null);
    this.pages.set([]);
    this.pagesCursor.set(undefined);
    this.pagesError.set('');
    void this.accounts.ensureLoaded();
    void this.storage.ensureLoaded();
  }

  /** The account a conversation is recorded as (the Default when it names
   * none and only the Default exists; '' for an unregistered directory). */
  private accountIdOf(s: SessionSummary | undefined): string {
    if (!s) return '';
    if (s.account) return s.account.id;
    return this.accounts.multi() ? '' : DEFAULT_ACCOUNT;
  }

  private async loadPage(): Promise<void> {
    if (this.pagesLoading() || this.pagesCursor() === null) return;
    this.pagesLoading.set(true);
    this.pagesError.set('');
    try {
      const page = await this.api.listSessions({
        scope: 'all',
        limit: PAGE,
        ...(this.pagesCursor() ? { before: this.pagesCursor() as string } : {}),
      });
      this.pages.update((prev) => [...prev, ...(page.sessions ?? [])]);
      this.pagesCursor.set(page.nextBefore ?? null);
    } catch (e) {
      this.pagesError.set(moveErrorText(e));
    } finally {
      this.pagesLoading.set(false);
    }
  }

  private async loadIndex(key: string): Promise<void> {
    const loc = locationFromKey(key);
    if (loc?.kind !== 'storage') return;
    this.index.set({ key, ix: null, loading: true, error: '' });
    try {
      const ix = normalizeBackupIndex(await this.api.getStorageIndex(loc.storage, loc.set));
      if (!ix) throw new Error('The Mac sent an index this app could not read.');
      if (this.index()?.key === key) this.index.set({ key, ix, loading: false, error: '' });
    } catch (e) {
      if (this.index()?.key === key) {
        this.index.set({ key, ix: null, loading: false, error: moveErrorText(e) });
      }
    }
  }
}
