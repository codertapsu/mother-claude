import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  inject,
  signal,
} from '@angular/core';
import { FormsModule } from '@angular/forms';

import { AccountsService } from '../../core/accounts.service';
import { ApiService, errorMessage } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { formatBytes, normalizeJob, plural } from '../../core/data-jobs';
import { DataJobsService } from '../../core/data-jobs.service';
import {
  DataProject,
  deleteErrorText,
  desktopOs,
  normalizeDataProjects,
  projectGroups,
  projectKey,
  projectMatches,
  revealLabel,
} from '../../core/delete-data';
import { JobView } from '../../core/models';
import { accountLocation, anyTarget } from '../../core/move-data';
import { NoticeService } from '../../core/notice.service';
import { RealtimeService } from '../../core/realtime.service';
import { StorageService } from '../../core/storage.service';
import { AccountChipComponent } from '../../shared/account-chip.component';
import { DataJobComponent } from '../../shared/data-job.component';
import {
  DeleteDataSheetComponent,
  DeleteStart,
  countedWhat,
} from '../../shared/delete-data-sheet.component';
import { IconComponent } from '../../shared/icon.component';
import { MoveDataSheetComponent, MoveStart } from '../../shared/move-data-sheet.component';
import { SheetComponent } from '../../shared/sheet.component';
import { relativeTime, shortPath } from '../../shared/util';

/** Recent deletes listed (their Undo lasts seven days). */
const DELETES_SHOWN = 6;

/** Why Move or copy waits: one request moves one account's projects. */
export const MIXED_ACCOUNTS = 'Pick projects of one account to move or copy them together.';

/**
 * Sessions › Projects (docs/ACCOUNTS.md §14.2, §14.4): every project folder
 * that holds conversations, in every account's directory — its name and
 * folder, the account, how many conversations (and how many run), their
 * size and the last activity. *Select* deletes, moves or copies several; a
 * row's menu reveals its folder on disk (the desktop), copies its path,
 * moves or copies it, or deletes it. Deletes stay undoable below, in
 * *Recently deleted*, for seven days.
 */
@Component({
  selector: 'mc-projects-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    FormsModule,
    AccountChipComponent,
    DataJobComponent,
    DeleteDataSheetComponent,
    IconComponent,
    MoveDataSheetComponent,
    SheetComponent,
  ],
  template: `
    <div class="toolbar">
      <label class="search">
        <mc-icon name="search" [size]="16" />
        <input
          type="search"
          placeholder="Search projects"
          aria-label="Search projects"
          [ngModel]="query()"
          (ngModelChange)="query.set($event)"
          autocapitalize="off"
          spellcheck="false"
        />
      </label>
      <div class="filters">
        @if (multi()) {
          <select aria-label="Account" [ngModel]="account()" (ngModelChange)="account.set($event)">
            <option value="">All accounts</option>
            @for (a of accountList(); track a.id) {
              <option [value]="a.id">{{ a.name }}</option>
            }
          </select>
        }
        <span class="spacer"></span>
        @if (list()?.length) {
          <button
            class="btn small"
            type="button"
            [attr.aria-pressed]="selecting()"
            (click)="toggleSelecting()"
          >
            @if (selecting()) {
              Done
            } @else {
              <mc-icon name="check" [size]="15" /> Select
            }
          </button>
        }
      </div>
    </div>

    @if (list(); as all) {
      @if (!all.length) {
        <div class="card empty-state">
          <strong>No conversations on disk</strong>
          A project shows up here once Claude Code has written a conversation in it.
        </div>
      } @else if (!rows().length) {
        <div class="card empty-state">
          <strong>No projects match</strong>
          <button class="btn" type="button" (click)="clearFilters()">Clear filters</button>
        </div>
      } @else {
        <section class="card list rows" aria-label="Projects on disk">
          @for (p of rows(); track key(p)) {
            <div
              class="prow"
              [attr.data-selected]="selecting() && picked().has(key(p)) ? '' : null"
            >
              @if (selecting()) {
                <label class="pick">
                  <input
                    type="checkbox"
                    [checked]="picked().has(key(p))"
                    [attr.aria-label]="'Select ' + p.name"
                    (change)="flip(p)"
                  />
                </label>
              }
              <div class="pmain">
                <div class="pline">
                  <strong class="pname">{{ p.name }}</strong>
                  @if (multi()) {
                    @if (accountOf(p); as a) {
                      <mc-account-chip [account]="a" />
                    }
                  }
                </div>
                <span class="mono small pcwd">{{ short(p.cwd) }}</span>
                <span class="muted small pmeta">{{ meta(p) }}</span>
                @if (p.folder) {
                  <span class="mono pfolder" [title]="p.folder">{{ short(p.folder) }}</span>
                }
              </div>
              <button
                class="btn ghost icon-only"
                type="button"
                aria-haspopup="dialog"
                [attr.aria-label]="'Project menu: ' + p.name"
                (click)="menuFor.set(p)"
              >
                <mc-icon name="moreV" />
              </button>
            </div>
          }
        </section>
      }
    } @else if (loadError()) {
      <div class="card empty-state">
        <strong>Can't list the projects</strong>
        {{ loadError() }}
        <div class="empty-actions">
          <button class="btn" type="button" (click)="load()">Retry</button>
        </div>
      </div>
    } @else {
      <div class="skeleton" aria-label="Loading the projects">
        <span></span><span></span><span></span>
      </div>
    }

    @if (deletes().length) {
      <h2>Recently deleted</h2>
      <p class="muted small">
        Undo puts the conversations back where they were, for seven days after the delete.
      </p>
      <section class="card list jobs" aria-label="Recently deleted">
        @for (j of deletes(); track j.id) {
          <mc-data-job class="jrow" [job]="j" (changed)="jobChanged($event)" />
        }
      </section>
    }

    @if (selecting()) {
      <div class="selbar" role="region" aria-label="Selected projects">
        <span class="selcount" aria-live="polite">{{ picked().size }} selected</span>
        <button
          class="btn small ghost"
          type="button"
          title="Every project the search and the account filter match"
          (click)="selectAll()"
        >
          {{ query().trim() || account() ? 'Select all matching' : 'Select all' }}
        </button>
        @if (picked().size) {
          <button class="btn small ghost" type="button" (click)="picked.set(none)">Clear</button>
        }
        <span class="spacer"></span>
        <button
          class="btn small danger"
          type="button"
          [disabled]="!picked().size"
          (click)="deletePicked()"
        >
          <mc-icon name="trash" [size]="14" /> Delete {{ picked().size || '' }}…
        </button>
        @if (moveOffered()) {
          <button
            class="btn small primary"
            type="button"
            [disabled]="!picked().size || mixed()"
            [attr.title]="mixed() ? MIXED : null"
            (click)="movePicked()"
          >
            <mc-icon name="move" [size]="14" /> Move or copy {{ picked().size || '' }}…
          </button>
          @if (mixed()) {
            <span class="muted small why">{{ MIXED }}</span>
          }
        }
      </div>
    }

    @if (menuFor(); as p) {
      <mc-sheet class="menu-sheet" [open]="true" [title]="p.name" (closed)="menuFor.set(null)">
        @if (p.folder) {
          <p class="mono small where">{{ short(p.folder) }}</p>
        }
        <div class="menu">
          @if (isDesktop()) {
            <button class="item" type="button" (click)="reveal(p)">
              <mc-icon name="folder" [size]="18" />
              <span>{{ revealText }}</span>
            </button>
          }
          <button class="item" type="button" (click)="copyPath(p)">
            <mc-icon name="copy" [size]="18" />
            <span>Copy path</span>
          </button>
          @if (moveOffered()) {
            <button class="item" type="button" (click)="moveOne(p)">
              <mc-icon name="move" [size]="18" />
              <span>Move or copy…</span>
            </button>
          }
          <button class="item" type="button" (click)="deleteOne(p)">
            <mc-icon name="trash" [size]="18" />
            <span>Delete…</span>
          </button>
        </div>
      </mc-sheet>
    }

    <mc-move-data-sheet [open]="!!moveStart()" [start]="moveStart()" (closed)="moveClosed()" />
    <mc-delete-data-sheet
      [open]="!!deleteStart()"
      [start]="deleteStart()"
      (closed)="deleteClosed($event)"
    />
  `,
  styles: [
    `
      :host {
        display: block;
      }
      .toolbar {
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
        margin-bottom: 0.75rem;
      }
      .search {
        position: relative;
        display: block;
      }
      .search mc-icon {
        position: absolute;
        left: 0.7rem;
        top: 50%;
        transform: translateY(-50%);
        color: var(--faint);
        pointer-events: none;
      }
      .search input {
        padding-left: 2.2rem;
      }
      .filters {
        display: flex;
        gap: 0.5rem;
        flex-wrap: wrap;
        align-items: center;
      }
      .filters select {
        width: auto;
        flex: 1 1 9rem;
        max-width: 14rem;
      }
      .rows,
      .jobs {
        padding-top: 0;
        padding-bottom: 0;
      }
      .jrow {
        display: block;
        padding: 0.7rem 0;
      }
      .prow {
        display: flex;
        align-items: flex-start;
        gap: 0.55rem;
        padding: 0.7rem 0;
        min-width: 0;
      }
      .prow[data-selected] .pname {
        color: var(--accent);
      }
      .pick {
        flex: 0 0 auto;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        min-width: var(--tap);
        min-height: var(--tap);
        margin: -0.6rem -0.55rem -0.6rem -0.85rem;
        cursor: pointer;
      }
      .pick input {
        width: 1.05rem;
        height: 1.05rem;
      }
      .pmain {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
      }
      .pline {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.4rem;
        min-width: 0;
      }
      .pname {
        overflow-wrap: anywhere;
      }
      .pcwd,
      .pfolder {
        overflow-wrap: anywhere;
      }
      .pfolder {
        font-size: 0.72rem;
        color: var(--faint);
      }
      .prow .icon-only {
        flex: 0 0 auto;
        margin: -0.3rem -0.4rem 0 0;
      }
      h2 {
        margin-top: 1.5rem;
      }
      .empty-actions {
        margin-top: 0.75rem;
      }
      .skeleton {
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
      }
      .skeleton span {
        display: block;
        height: 4.5rem;
        border-radius: var(--radius);
        background: var(--panel-2);
        animation: mc-shimmer 1.2s ease-in-out infinite alternate;
      }
      /* The bar stays in view above the phone's tab bar. */
      .selbar {
        position: sticky;
        bottom: calc(var(--tabbar-h) + env(safe-area-inset-bottom) + 0.5rem);
        z-index: 5;
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.35rem 0.5rem;
        margin-top: 1rem;
        padding: 0.5rem 0.65rem;
        background: var(--panel);
        border: 1px solid var(--border-strong);
        border-radius: var(--radius);
        box-shadow: var(--shadow);
      }
      .selcount {
        font-weight: 600;
        font-variant-numeric: tabular-nums;
        margin-right: 0.15rem;
      }
      .why {
        flex-basis: 100%;
        text-align: right;
      }
      @media (min-width: 768px) {
        .selbar {
          bottom: 1rem;
        }
      }
      .where {
        margin: 0 0 0.5rem;
        color: var(--muted);
        overflow-wrap: anywhere;
      }
      .menu {
        display: flex;
        flex-direction: column;
      }
      .item {
        appearance: none;
        display: flex;
        align-items: center;
        gap: 0.75rem;
        min-height: var(--tap);
        padding: 0.4rem 0.25rem;
        border: 0;
        border-top: 1px solid var(--border);
        background: none;
        color: var(--text);
        font: inherit;
        text-align: left;
        cursor: pointer;
      }
      .item:first-child {
        border-top: 0;
      }
      .item > span {
        flex: 1;
        min-width: 0;
      }
      .item mc-icon {
        color: var(--muted);
      }
    `,
  ],
})
export class ProjectsViewComponent {
  private api = inject(ApiService);
  private accounts = inject(AccountsService);
  private storage = inject(StorageService);
  private notices = inject(NoticeService);
  private jobs = inject(DataJobsService);
  protected readonly isDesktop = inject(AuthService).isDesktop;

  protected readonly MIXED = MIXED_ACCOUNTS;
  protected readonly none: ReadonlySet<string> = new Set();
  protected readonly revealText = revealLabel(desktopOs(navigator.userAgent));
  protected readonly short = shortPath;
  protected readonly key = projectKey;

  /** `GET /data/projects` (null: not read yet). */
  protected readonly list = signal<DataProject[] | null>(null);
  protected readonly loadError = signal('');
  protected readonly query = signal('');
  /** The account filter ('' = every account). */
  protected readonly account = signal('');
  protected readonly accountList = this.accounts.list;
  protected readonly multi = this.accounts.multi;

  protected readonly rows = computed(() => {
    const q = this.query().trim().toLowerCase();
    const id = this.account();
    const account = this.multi() && this.accounts.byId(id) ? id : '';
    return (this.list() ?? []).filter(
      (p) => (!account || p.account === account) && projectMatches(p, q),
    );
  });

  protected readonly selecting = signal(false);
  protected readonly picked = signal<ReadonlySet<string>>(this.none);
  private readonly pickedRows = computed(() =>
    (this.list() ?? []).filter((p) => this.picked().has(projectKey(p))),
  );
  /** The picked projects are in more than one account. */
  protected readonly mixed = computed(
    () => new Set(this.pickedRows().map((p) => p.account)).size > 1,
  );
  protected readonly moveOffered = computed(() =>
    anyTarget(this.accounts.list(), this.storage.list() ?? []),
  );

  protected readonly menuFor = signal<DataProject | null>(null);
  protected readonly moveStart = signal<MoveStart | null>(null);
  protected readonly deleteStart = signal<DeleteStart | null>(null);
  /** Recent deletes (`GET /data/jobs`), newest first: Undo stays here once
   * the dialog is closed. */
  protected readonly deletes = signal<JobView[]>([]);
  private readonly stopFollowing = new AbortController();

  constructor() {
    void this.accounts.ensureLoaded();
    void this.storage.ensureLoaded();
    void this.load();
    void this.loadDeletes();
    const sub = inject(RealtimeService).refetch$.subscribe(() => void this.load());
    inject(DestroyRef).onDestroy(() => {
      sub.unsubscribe();
      this.stopFollowing.abort();
    });
  }

  async load(): Promise<void> {
    try {
      const list = normalizeDataProjects(await this.api.getDataProjects());
      this.list.set(list);
      this.loadError.set('');
      // What is gone is no longer picked.
      const keys = new Set(list.map(projectKey));
      const kept = [...this.picked()].filter((k) => keys.has(k));
      if (kept.length !== this.picked().size) this.picked.set(new Set(kept));
    } catch (e) {
      this.loadError.set(errorMessage(e));
    }
  }

  protected accountOf(p: DataProject) {
    return this.accounts.byId(p.account) ?? null;
  }

  /** "12 conversations · 1 running · 4.2 MB · 3h ago". */
  protected meta(p: DataProject): string {
    return [
      plural(p.sessions, 'conversation'),
      p.live ? `${p.live} running` : '',
      p.bytes ? formatBytes(p.bytes) : '',
      p.lastActivityAt ? relativeTime(p.lastActivityAt) : '',
    ]
      .filter(Boolean)
      .join(' · ');
  }

  protected clearFilters(): void {
    this.query.set('');
    this.account.set('');
  }

  protected toggleSelecting(): void {
    const on = !this.selecting();
    this.selecting.set(on);
    if (!on) this.picked.set(this.none);
  }

  protected flip(p: DataProject): void {
    const k = projectKey(p);
    this.picked.update((prev) => {
      const next = new Set(prev);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      return next;
    });
  }

  /** Every project the search and the account filter match (they are all
   * read at once, so every match is a row). */
  protected selectAll(): void {
    this.picked.update((prev) => new Set([...prev, ...this.rows().map(projectKey)]));
  }

  protected deletePicked(): void {
    const rows = this.pickedRows();
    if (!rows.length) return;
    this.deleteStart.set({
      what: countedWhat(rows.length, 'project'),
      groups: projectGroups(rows),
    });
  }

  protected movePicked(): void {
    const rows = this.pickedRows();
    if (!rows.length || this.mixed()) return;
    this.moveStart.set({
      from: accountLocation(rows[0].account),
      what: 'projects',
      projects: rows.map((p) => p.cwd),
    });
  }

  protected async reveal(p: DataProject): Promise<void> {
    this.menuFor.set(null);
    try {
      await this.api.revealProject(p.account, p.cwd);
    } catch (e) {
      this.notices.show('error', deleteErrorText(e));
    }
  }

  protected async copyPath(p: DataProject): Promise<void> {
    this.menuFor.set(null);
    const path = p.folder || p.cwd;
    try {
      await navigator.clipboard.writeText(path);
      this.notices.success('Copied the path.');
    } catch {
      this.notices.show('error', "Couldn't copy the path.");
    }
  }

  protected moveOne(p: DataProject): void {
    this.menuFor.set(null);
    this.moveStart.set({
      from: accountLocation(p.account),
      what: 'projects',
      projects: [p.cwd],
    });
  }

  protected deleteOne(p: DataProject): void {
    this.menuFor.set(null);
    this.deleteStart.set({
      what: 'this project',
      groups: projectGroups([p]),
    });
  }

  protected moveClosed(): void {
    this.moveStart.set(null);
    void this.load();
  }

  /** The dialog closed: the list is read again, and its jobs join the
   * deletes below. */
  protected deleteClosed(jobs: JobView[]): void {
    this.deleteStart.set(null);
    if (!jobs.length) return;
    void this.load();
    void this.loadDeletes();
  }

  protected jobChanged(j: JobView): void {
    const before = this.deletes().find((x) => x.id === j.id);
    this.deletes.update((list) => list.map((x) => (x.id === j.id ? j : x)));
    // Undone: its conversations are back.
    if (before && before.state !== j.state) void this.load();
  }

  /** The deletes the Mac still knows of; a running one is followed. */
  private async loadDeletes(): Promise<void> {
    let list: JobView[];
    try {
      const { jobs } = await this.api.listDataJobs();
      list = (Array.isArray(jobs) ? jobs : [])
        .map(normalizeJob)
        .filter((j): j is JobView => !!j && j.kind === 'delete')
        .sort((a, b) => b.startedAt - a.startedAt)
        .slice(0, DELETES_SHOWN);
    } catch {
      // Only a convenience: the dialog has its own.
      return;
    }
    this.deletes.set(list);
    for (const j of list) {
      if (j.state !== 'running') continue;
      void this.jobs
        .follow(j, (next) => this.jobChanged(next), this.stopFollowing.signal)
        .catch(() => undefined);
    }
  }
}
