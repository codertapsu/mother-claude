import { Component, DestroyRef, computed, effect, inject, signal, untracked } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { ActivatedRoute, RouterLink } from '@angular/router';

import { DEFAULT_ACCOUNT } from '../../core/accounts';
import { AccountsService } from '../../core/accounts.service';
import { ApiService, errorMessage } from '../../core/api.service';
import { sessionGroups } from '../../core/delete-data';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { JobView, SessionSummary } from '../../core/models';
import { anyTarget } from '../../core/move-data';
import { storageGet, storageSet } from '../../core/platform';
import { RealtimeService } from '../../core/realtime.service';
import { MachineTag, RemoteSessionsService } from '../../core/remote-sessions.service';
import { StorageService } from '../../core/storage.service';
import {
  DeleteDataSheetComponent,
  DeleteStart,
  countedWhat,
  deletedIds,
} from '../../shared/delete-data-sheet.component';
import { FleetSummaryComponent } from '../../shared/fleet-summary.component';
import { IconComponent } from '../../shared/icon.component';
import { LimitOfferComponent } from '../../shared/limit-offer.component';
import { MoveDataSheetComponent, MoveStart } from '../../shared/move-data-sheet.component';
import { SessionCardComponent } from '../../shared/session-card.component';
import {
  SessionFilters,
  filterQuery,
  filtersActive,
  needsYou,
  sessionPasses,
} from '../../shared/session-search';
import { surfaceLabel } from '../../shared/util';
import { SessionsHeadComponent } from './sessions-head.component';

type Filter = 'live' | 'needs' | 'all';

interface Group {
  key: string;
  title: string;
  items: SessionSummary[];
}

const FILTER_KEY = 'mc_sessions_filter';
const HISTORY_PAGE = 25;
/** How long typing waits before earlier sessions are searched again. */
const SEARCH_DEBOUNCE_MS = 300;

function byRecent(a: SessionSummary, b: SessionSummary): number {
  return (b.lastActivityAt ?? b.startedAt ?? 0) - (a.lastActivityAt ?? a.startedAt ?? 0);
}

/** A card's identity: another computer's session may share an id with one
 * here (a session continued on another computer keeps its id). */
function cardKey(s: SessionSummary): string {
  return s.machine ? `${s.machine}\u0000${s.id}` : s.id;
}

/** The Machine filter: every computer, this one (`here`), or another's id. */
const HERE = 'here';

/**
 * Every session in the active scope (live, or active in the last 24 h),
 * searchable and filterable (by the app it runs in, project and — while
 * more than the Default exists — Claude account), grouped by what they
 * need; ended
 * sessions page in on demand. A card with one simple pending request can be
 * answered right here. Earlier sessions are searched and filtered on the
 * server, so a search finds them all, not only the pages read so far.
 * *Select* puts a checkbox on every card and a bar below them: *Select all*
 * (every session the search and filters match, earlier ones not read yet
 * included), *Delete N…* (docs/ACCOUNTS.md §14) and,
 * when there is somewhere to move them, *Move or copy N…*, which opens Move
 * data with those conversations (from several accounts at once: each goes
 * from where it is). Every project on disk is the *Projects* tab
 * (`/projects`).
 *
 * Other computers' sessions (docs/PLAN-MACHINES.md §6.1) join the same
 * groups with their computer's chip, and a *Computer* filter picks every
 * computer, this one or one other (`?machine=<id>` opens it so); an offline
 * computer's sessions stay, greyed, in a group of their own. They are never
 * selected, an account filter (this computer's accounts) leaves them out,
 * and earlier sessions are this computer's only. With no other computer
 * paired the page is exactly this computer's.
 */
@Component({
  selector: 'mc-sessions',
  imports: [
    RouterLink,
    FormsModule,
    FleetSummaryComponent,
    IconComponent,
    DeleteDataSheetComponent,
    LimitOfferComponent,
    MoveDataSheetComponent,
    SessionCardComponent,
    SessionsHeadComponent,
    TPipe,
  ],
  templateUrl: './sessions.component.html',
  styleUrl: './sessions.component.scss',
})
export class SessionsComponent {
  private realtime = inject(RealtimeService);
  private api = inject(ApiService);
  private accounts = inject(AccountsService);
  private storage = inject(StorageService);
  private remote = inject(RemoteSessionsService);

  protected readonly loaded = this.realtime.sessionsLoaded;
  protected readonly connection = this.realtime.connection;
  /** This computer's sessions. */
  protected readonly all = this.realtime.sessions;

  /** Other computers whose sessions show (none while none is paired). */
  protected readonly machines = this.remote.list;
  /** The Machine filter: '' every computer, `here` this one, else a
   * computer's id. */
  protected readonly machine = signal(
    inject(ActivatedRoute).snapshot.queryParamMap.get('machine') ?? '',
  );
  /** The pick in effect: every computer once its computer is gone. */
  protected readonly machineInEffect = computed(() => {
    const m = this.machine();
    return m === HERE || this.remote.byId().has(m) ? m : '';
  });
  /** This computer's sessions the Machine filter keeps. */
  private readonly localShown = computed(() => {
    const m = this.machineInEffect();
    return m && m !== HERE ? [] : this.all();
  });
  /** Other computers' sessions the Machine filter keeps, online or not. */
  private readonly remoteShown = computed(() => {
    const m = this.machineInEffect();
    if (m === HERE) return { online: [], offline: [] };
    const list = this.machines().filter((x) => !m || x.id === m);
    return {
      online: list.filter((x) => x.online).flatMap((x) => x.sessions),
      offline: list.filter((x) => !x.online).flatMap((x) => x.sessions),
    };
  });
  /** Other computers still sending their first list. */
  protected readonly machinesLoading = computed(() => {
    const m = this.machineInEffect();
    return this.machines().filter((x) => x.online && !x.loaded && (!m || x.id === m));
  });
  /** Every session the page has, of every computer. */
  protected readonly everything = computed(() => [...this.all(), ...this.remote.sessions()]);
  protected readonly cardKey = cardKey;
  protected readonly HERE = HERE;

  protected readonly query = signal('');
  protected readonly filter = signal<Filter>(readFilter());
  protected readonly surface = signal('');
  protected readonly project = signal('');
  /** The account filter ('' = every account; shown only while more than
   * the Default exists). */
  protected readonly account = signal('');
  protected readonly accountList = this.accounts.list;
  protected readonly multi = this.accounts.multi;
  /** The pick in effect: none once its account is gone (or the only one). */
  private readonly accountInEffect = computed(() => {
    const id = this.account();
    return this.accounts.multi() && this.accounts.byId(id) ? id : '';
  });

  protected readonly history = signal<SessionSummary[]>([]);
  protected readonly historyOpen = signal(false);
  protected readonly historyLoading = signal(false);
  protected readonly historyError = signal('');
  private historyCursor: string | undefined;
  protected readonly historyDone = signal(false);
  /** Bumped when the filters change: a page read for older ones is dropped. */
  private historyGen = 0;
  private historyKey = '';
  private historyTimer?: ReturnType<typeof setTimeout>;

  /** Sessions that can be live or waiting on someone now: this computer's
   * and the online computers' (an offline one's are as last heard). */
  private readonly current = computed(() => [
    ...this.all(),
    ...this.machines()
      .filter((m) => m.online)
      .flatMap((m) => m.sessions),
  ]);
  protected readonly liveCount = computed(() => this.current().filter((s) => s.live).length);
  protected readonly needsCount = computed(() => this.current().filter(needsYou).length);

  protected readonly surfaces = computed(() =>
    [...new Set([...this.everything(), ...this.history()].map((s) => s.surface))].sort(),
  );
  protected readonly projects = computed(() =>
    [
      ...new Set(
        [...this.everything(), ...this.history()].map((s) => s.projectName).filter(Boolean),
      ),
    ].sort((a, b) => a.localeCompare(b)),
  );

  /** Everything the page filters by: the search, the pickers and the All /
   * Live / Needs you switch. */
  protected readonly filters = computed<SessionFilters>(() => {
    const f = this.filter();
    return {
      q: this.query(),
      surface: this.surface(),
      project: this.project(),
      account: this.accountInEffect(),
      only: f === 'all' ? '' : f,
    };
  });
  /** The same without the switch: earlier sessions are none of Live or
   * Needs you. */
  private readonly historyFilters = computed<SessionFilters>(() => ({
    ...this.filters(),
    only: '',
  }));

  /** An account filter names one of this computer's accounts: other
   * computers' sessions run under their own. */
  protected readonly visible = computed(() => {
    const f = this.filters();
    const remote = f.account ? [] : this.remoteShown().online;
    return [...this.localShown(), ...remote].filter((s) => sessionPasses(s, f));
  });
  /** Offline computers' sessions the filters keep (none under Live or Needs
   * you: they are as last heard). */
  private readonly offlineVisible = computed(() => {
    const f = this.filters();
    if (f.only || f.account) return [];
    return this.remoteShown()
      .offline.filter((s) => sessionPasses(s, f))
      .sort(byRecent);
  });

  protected readonly groups = computed<Group[]>(() => {
    const list = this.visible();
    const needs = list.filter(needsYou);
    const rest = list.filter((s) => !needsYou(s));
    const groups: Group[] = [
      {
        key: 'needs',
        title: tr('Needs you'),
        items: needs.sort((a, b) => oldestPending(a) - oldestPending(b)),
      },
      {
        key: 'working',
        title: tr('Working'),
        items: rest.filter((s) => s.status === 'working' || s.status === 'starting').sort(byRecent),
      },
      {
        // Between turns, but not done: background work or a wakeup resumes
        // them by themselves.
        key: 'background',
        title: tr('Will resume'),
        items: rest.filter((s) => s.status === 'background').sort(byRecent),
      },
      {
        key: 'idle',
        title: tr('Idle'),
        items: rest.filter((s) => s.status === 'idle').sort(byRecent),
      },
      {
        key: 'ended',
        title: tr('Ended recently'),
        items: rest.filter((s) => ['ended', 'failed', 'stopped'].includes(s.status)).sort(byRecent),
      },
    ];
    // Unknown statuses from a newer server still show up.
    const known = new Set(groups.flatMap((g) => g.items.map(cardKey)));
    const other = list.filter((s) => !known.has(cardKey(s)));
    if (other.length) {
      groups.push({ key: 'other', title: tr('Other'), items: other.sort(byRecent) });
    }
    const offline = this.offlineVisible();
    if (offline.length) {
      groups.push({ key: 'offline', title: tr('On computers that are offline'), items: offline });
    }
    return groups.filter((g) => g.items.length);
  });

  /** Earlier sessions are this computer's: not under another computer. */
  protected readonly historyShown = computed(() => {
    const m = this.machineInEffect();
    return !m || m === HERE;
  });

  protected readonly historyVisible = computed(() => {
    const inStore = new Set(this.all().map((s) => s.id));
    const f = this.historyFilters();
    return this.history().filter((s) => !inStore.has(s.id) && sessionPasses(s, f));
  });

  protected readonly surfaceLabel = surfaceLabel;
  /** The desktop's computer in words ("Mac", "PC", "computer"). */
  protected readonly computer = computerNoun;

  /** Somewhere to move conversations to (another account, or a
   * storage): the Select bar offers Move or copy. */
  protected readonly moveOffered = computed(() =>
    anyTarget(this.accounts.list(), this.storage.list() ?? []),
  );
  /** Sessions › Select: the cards have checkboxes. */
  protected readonly selecting = signal(false);
  protected readonly picked = signal<ReadonlySet<string>>(new Set());
  /** *Select all* is asking the server for every match. */
  protected readonly selectingAll = signal(false);
  protected readonly selectAllError = signal('');
  protected readonly selectAllLabel = computed(() =>
    filtersActive(this.filters()) ? tr('Select all matching') : tr('Select all'),
  );
  /** Move data with the picked conversations (null: closed). */
  protected readonly moveStart = signal<MoveStart | null>(null);
  /** The delete dialog with the picked conversations (null: closed). */
  protected readonly deleteStart = signal<DeleteStart | null>(null);

  constructor() {
    void this.storage.ensureLoaded();
    // Earlier sessions are searched on the server: a new search or filter
    // reads them again, once the typing stops.
    effect(() => {
      const key = JSON.stringify(filterQuery(this.historyFilters()));
      untracked(() => this.historyFiltersChanged(key));
    });
    inject(DestroyRef).onDestroy(() => clearTimeout(this.historyTimer));
  }

  setFilter(f: Filter): void {
    this.filter.set(f);
    storageSet(FILTER_KEY, f);
  }

  clearFilters(): void {
    this.query.set('');
    this.surface.set('');
    this.project.set('');
    this.account.set('');
    this.machine.set('');
    this.setFilter('all');
  }

  /** The other computer a card's session runs on (null: this one). */
  protected tagOf(s: SessionSummary): MachineTag | null {
    return this.remote.tagOf(s);
  }

  toggleSelecting(): void {
    const on = !this.selecting();
    this.selecting.set(on);
    if (!on) this.clearPicked();
  }

  flip(id: string): void {
    this.picked.update((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  /** Every session the search and filters match — the earlier ones not
   * read yet too, which the server finds — plus every card shown. If the
   * server cannot be asked, the cards shown are selected and the bar says so. */
  async selectAll(): Promise<void> {
    if (this.selectingAll()) return;
    // Another computer's sessions are never selected (their data is there).
    const shown = [
      ...this.groups().flatMap((g) => g.items),
      ...(this.historyOpen() ? this.historyVisible() : []),
    ]
      .filter((s) => !s.machine)
      .map((s) => s.id);
    this.selectingAll.set(true);
    this.selectAllError.set('');
    let found: string[] = [];
    try {
      found = await this.api.listSessionIds({ scope: 'all', ...filterQuery(this.filters()) });
    } catch (e) {
      this.selectAllError.set(
        tr('Only the sessions shown were selected; earlier ones could not be found: {error}', {
          error: errorMessage(e),
        }),
      );
    } finally {
      this.selectingAll.set(false);
    }
    // Still selecting (Select may have been turned off meanwhile).
    if (!this.selecting()) return;
    this.picked.update((prev) => new Set([...prev, ...shown, ...found]));
  }

  clearPicked(): void {
    this.picked.set(new Set());
    this.selectAllError.set('');
  }

  /** Move data with them, each from the account it is in. */
  openMove(): void {
    const ids = [...this.picked()];
    if (!ids.length) return;
    this.moveStart.set({
      from: null,
      what: 'sessions',
      sessions: ids,
      known: [...this.all(), ...this.history()],
    });
  }

  /** Delete them, each from the account it is recorded as. */
  openDelete(): void {
    const ids = [...this.picked()];
    if (!ids.length) return;
    const known = new Map([...this.history(), ...this.all()].map((s) => [s.id, s]));
    const multi = this.accounts.multi();
    this.deleteStart.set({
      what: countedWhat(ids.length, 'conversation'),
      groups: sessionGroups(
        ids.map((id) => {
          const s = known.get(id);
          const account = s?.account?.id ?? (multi ? undefined : DEFAULT_ACCOUNT);
          return { id, ...(account ? { account } : {}) };
        }),
      ),
      known: [...known.values()].filter((s) => ids.includes(s.id)),
    });
  }

  /** The dialog closed: what it deleted is no longer picked. */
  deleteClosed(jobs: JobView[]): void {
    this.deleteStart.set(null);
    const gone = deletedIds(jobs);
    if (!gone.size) return;
    this.picked.update((prev) => new Set([...prev].filter((id) => !gone.has(id))));
    this.history.update((list) => list.filter((s) => !gone.has(s.id)));
  }

  /** The fleet summary's account: show its sessions (again: every one). */
  showAccount(id: string): void {
    this.account.set(this.account() === id ? '' : id);
  }

  async openHistory(): Promise<void> {
    this.historyOpen.set(true);
    if (!this.history().length) await this.loadMoreHistory();
  }

  async loadMoreHistory(): Promise<void> {
    if (this.historyLoading() || this.historyDone()) return;
    const gen = this.historyGen;
    this.historyLoading.set(true);
    this.historyError.set('');
    try {
      const page = await this.api.listSessions({
        scope: 'all',
        limit: HISTORY_PAGE,
        before: this.historyCursor,
        ...filterQuery(this.historyFilters()),
      });
      if (gen !== this.historyGen) return; // read for other filters
      const seen = new Set(this.history().map((s) => s.id));
      const fresh = (page.sessions ?? []).filter((s) => !seen.has(s.id));
      this.history.update((prev) => [...prev, ...fresh]);
      this.historyCursor = page.nextBefore;
      if (!page.nextBefore) this.historyDone.set(true);
    } catch (e) {
      if (gen === this.historyGen) this.historyError.set(errorMessage(e));
    } finally {
      if (gen === this.historyGen) this.historyLoading.set(false);
    }
  }

  /** The search or a filter changed: earlier sessions are read again for
   * it (once typing stops, and only while they are open; otherwise when
   * they are opened). */
  private historyFiltersChanged(key: string): void {
    if (key === this.historyKey) return;
    const first = !this.historyKey;
    this.historyKey = key;
    if (first) return;
    clearTimeout(this.historyTimer);
    this.historyGen += 1;
    this.history.set([]);
    this.historyCursor = undefined;
    this.historyDone.set(false);
    this.historyError.set('');
    this.historyLoading.set(false);
    if (!this.historyOpen()) {
      // A search looks through earlier sessions too (in the All view: Live
      // and Needs you are current ones only).
      if (!this.query().trim() || this.filter() !== 'all') return;
      this.historyOpen.set(true);
    }
    const gen = this.historyGen;
    this.historyTimer = setTimeout(() => {
      if (gen === this.historyGen) void this.loadMoreHistory();
    }, SEARCH_DEBOUNCE_MS);
  }
}

function oldestPending(s: SessionSummary): number {
  return Math.min(...(s.pending ?? []).map((p) => p.createdAt), s.lastActivityAt ?? Infinity);
}

function readFilter(): Filter {
  const v = storageGet(FILTER_KEY);
  return v === 'live' || v === 'needs' ? v : 'all';
}
