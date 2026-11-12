import { Injectable, NgZone, computed, inject, signal } from '@angular/core';
import { Router } from '@angular/router';

import { ApiError, ApiService, describeError } from './api.service';
import { MachineScopes } from './machine-scope';
import { MachinesService } from './machines.service';
import {
  MovePreflight,
  MoveRecord,
  SessionMoveBody,
  TakeOverReply,
  moveFinal,
  movedSession,
} from './moves';
import { SessionSummary } from './models';
import { NoticeService } from './notice.service';
import { RealtimeService } from './realtime.service';
import { sessionLink } from './session-links';
import { computerNoun, tr } from './i18n/i18n';
import { MoveWords, moveSummary } from './moves-text';
import type { TakeOverFromCopy } from './copies';

/** A move, with the computer whose record it is. */
export interface MoveEntry {
  /** The computer that keeps this record: null for this one, else the
   * other computer it was read from (through its scope). The same move has
   * a record on each side: the computer it left and the one it went to. */
  at: string | null;
  move: MoveRecord;
}

/** How often a move followed on another computer is asked about while it
 * goes on, and while it waits — placed there but not started (*Try
 * again*), or that computer out of reach. */
const POLL_MS = 1500;
const POLL_SLOW_MS = 5000;
/** A followed move that has not changed for this long is left alone (one
 * that landed and waits for its computer to start it, say). */
const POLL_GIVE_UP_MS = 15 * 60_000;

function keyOf(at: string | null, id: string): string {
  return `${at ?? ''}|${id}`;
}

interface Poll {
  timer?: ReturnType<typeof setTimeout>;
  /** When the record last changed (ms). */
  changed: number;
}

/**
 * Moves of sessions between computers (docs/PLAN-MACHINES.md §5.8, Phase
 * 3a), for every screen that shows one: the move sheet and its progress,
 * the session page's line and *Lives on* banner, *Recent moves* on the
 * Machines page and a session's Info tab.
 *
 * - **This computer's records** (moves it sent or received): `GET /api/moves`
 *   on demand, then the `move` frames.
 * - **Another computer's** — a move of a session that lives there, started
 *   through its scope (`/api/machines/<id>/proxy/…`): its own record, which
 *   that computer does not pass on as frames yet, so it is polled
 *   (`GET /api/moves/<id>`) until it ends. Should it pass them on (`remote`
 *   frames of kind `move`), they are taken as they come.
 *
 * Starting, cancelling, undoing and trying again go to the computer the
 * record belongs to. A move this client started on another computer says
 * how it ended in a toast (this computer's own say it in the server's
 * notices).
 */
@Injectable({ providedIn: 'root' })
export class MovesService {
  private readonly api = inject(ApiService);
  private readonly realtime = inject(RealtimeService);
  private readonly scopes = inject(MachineScopes);
  private readonly machines = inject(MachinesService);
  private readonly notices = inject(NoticeService);
  private readonly router = inject(Router);
  private readonly zone = inject(NgZone);

  private readonly store = signal<ReadonlyMap<string, MoveEntry>>(new Map());
  /** Every move known, newest first. */
  readonly entries = computed(() =>
    [...this.store().values()].sort((a, b) => b.move.createdAt - a.move.createdAt),
  );
  /** This computer's records, newest first. */
  readonly here = computed(() => this.entries().filter((e) => e.at === null));
  /** `GET /api/moves` answered once. */
  readonly loaded = signal(false);
  /** Its last failure ('' when it worked). */
  readonly loadError = signal('');

  /** This computer's machine id ('' until Machines' view is known). */
  readonly selfId = computed(() => this.machines.view()?.self.id ?? '');
  /** Machines is on here. */
  readonly enabled = computed(() => !!this.machines.view()?.enabled);
  readonly peers = computed(() => this.machines.view()?.peers ?? []);

  private readonly polls = new Map<string, Poll>();
  /** Moves this client started or acted on through another computer. */
  private readonly mine = new Set<string>();

  constructor() {
    this.realtime.move$.subscribe((f) => this.put(null, f.move));
    this.realtime.remote$.subscribe((e) => {
      if (e.frame.t === 'move') this.put(e.machine, e.frame.move);
    });
    this.realtime.refetch$.subscribe(() => {
      if (this.loaded()) void this.load();
      for (const key of this.polls.keys()) {
        const [at, id] = key.split('|');
        if (at) this.schedule(at, id, 0);
      }
    });
  }

  /** The REST client of the computer `at` (null: this one). */
  client(at: string | null): ApiService {
    return at ? this.scopes.api(at) : this.api;
  }

  /** Keep Machines' view current (who is paired, who is online). */
  ensureMachines(): void {
    this.machines.start();
  }

  // ─────────────────────────────────────────────────────────── Reading

  /** The moves the computer `at` keeps (null: this one); another
   * computer's that are under way are followed until they end. */
  async load(at: string | null = null): Promise<MoveRecord[]> {
    try {
      const list = await this.client(at).listMoves();
      for (const m of list) this.put(at, m);
      if (at === null) {
        this.loaded.set(true);
        this.loadError.set('');
      } else {
        for (const m of list) if (!moveFinal(m)) this.follow(at, m.id);
      }
      return list;
    } catch (e) {
      if (at === null) this.loadError.set(describeError(e).text);
      return [];
    }
  }

  /** One record, as last heard. */
  get(at: string | null, id: string): MoveEntry | undefined {
    return this.store().get(keyOf(at, id));
  }

  /** Every move of a session (by its id, wherever it was read), newest
   * first: one conversation keeps its id across computers. */
  ofSession(id: string): MoveEntry[] {
    return this.entries().filter((e) => e.move.session === id || e.move.targetSession === id);
  }

  /** The move of a session under way now, if one is: the record of the
   * computer it leaves first (it knows every step). */
  current(id: string): MoveEntry | null {
    const running = this.ofSession(id).filter((e) => !moveFinal(e.move));
    return running.find((e) => e.move.role === 'out') ?? running[0] ?? null;
  }

  /** A session's moves, one record each (the computer it left keeps the
   * whole story), newest first. */
  history(id: string): MoveEntry[] {
    const all = this.ofSession(id);
    const out: MoveEntry[] = [];
    for (const e of all) {
      if (out.some((o) => o.move.id === e.move.id)) continue;
      const sender = all.find((o) => o.move.id === e.move.id && o.move.role === 'out');
      out.push(sender ?? e);
    }
    return out;
  }

  /** This computer's own copy of a session, when its live list has it. */
  localCopy(id: string): SessionSummary | undefined {
    return this.realtime.sessions().find((s) => s.id === id);
  }

  /** The session was on this computer before: it moved away from here, or
   * this computer keeps a copy of it. */
  leftHere(id: string): boolean {
    return (
      this.here().some((e) => e.move.role === 'out' && e.move.session === id) ||
      this.realtime.sessions().some((s) => s.id === id)
    );
  }

  // ─────────────────────────────────────────────────────────── Acting

  /** The move sheet's checks, on the computer the session lives on. */
  preflight(at: string | null, session: string, body: SessionMoveBody): Promise<MovePreflight> {
    return this.client(at).moveSessionPreflight(session, body);
  }

  /** Starts a move on the computer the session lives on. */
  async start(at: string | null, session: string, body: SessionMoveBody): Promise<MoveRecord> {
    const m = await this.client(at).moveSession(session, body);
    this.mine.add(keyOf(at, m.id));
    this.put(at, m);
    if (at) this.follow(at, m.id);
    return m;
  }

  /** The computer a session lives on is offline: it lives here from now on
   * (from the copy this computer keeps of it, when it keeps one). */
  takeOver(
    session: string,
    opts: { fromCopy?: boolean; folder?: string } = {},
  ): Promise<TakeOverReply & TakeOverFromCopy> {
    return this.api.takeOverSession(session, this.selfId(), opts);
  }

  /** Cancel (until it is being placed), undo (until `undoUntil`) or try
   * starting it there again; answers the record as it is after. */
  async act(
    e: MoveEntry,
    what: 'cancel' | 'undo' | 'retry' | 'removeWorktree',
  ): Promise<MoveRecord> {
    const c = this.client(e.at);
    const m =
      what === 'cancel'
        ? await c.cancelMove(e.move.id)
        : what === 'undo'
          ? await c.undoMove(e.move.id)
          : what === 'removeWorktree'
            ? await c.removeMoveWorktree(e.move.id)
            : await c.retryMove(e.move.id);
    this.mine.add(keyOf(e.at, m.id));
    this.put(e.at, m);
    if (e.at && !moveFinal(m)) this.follow(e.at, m.id);
    return m;
  }

  // ─────────────────────────────────────────────────────────── Computers

  /** Whether a computer is online now (this one always is). */
  online(machine: string): boolean {
    if (machine && machine === this.selfId()) return true;
    return (
      this.realtime.remote().get(machine)?.online ?? this.machines.peer(machine)?.online ?? false
    );
  }

  /** Since when a computer is offline (ms), when known. */
  offlineSince(machine: string): number | null {
    return (
      this.realtime.remote().get(machine)?.since ?? this.machines.peer(machine)?.lastSeen ?? null
    );
  }

  /** A computer's name: this one's own words for itself, a paired one's
   * name here. */
  nameOf(machine: string): string {
    return this.machines.nameOf(machine);
  }

  /** The computers of a move in this computer's words: *this Mac* for this
   * one, a paired one by its name here (a record read through another
   * computer names them as that one does). `turn`: the session is in a turn
   * now. Reactive where it is read. */
  words(turn = false): MoveWords {
    return {
      name: (m) => {
        const self = this.selfId();
        if (self && m.id === self) return tr('this {computer}', { computer: computerNoun() });
        return this.machines.peer(m.id)?.name || m.name;
      },
      isHere: (m) => !!this.selfId() && m.id === this.selfId(),
      turn,
    };
  }

  // ─────────────────────────────────────────────────────────── Records

  private put(at: string | null, m: MoveRecord): void {
    const key = keyOf(at, m.id);
    const prev = this.store().get(key)?.move;
    if (prev && prev.updatedAt > m.updatedAt) return;
    const next = new Map(this.store());
    next.set(key, { at, move: m });
    this.store.set(next);
    if (prev && !moveFinal(prev) && moveFinal(m)) this.ended(at, m);
  }

  /** A move this client started on another computer ended: say how (this
   * computer's own moves say it in the server's notices). */
  private ended(at: string | null, m: MoveRecord): void {
    if (at === null || !this.mine.has(keyOf(at, m.id))) return;
    if (m.state === 'resumed' && m.to.id !== this.selfId()) {
      const there = m.to.id;
      this.notices.success(moveSummary(m, this.words()), {
        action: {
          label: tr('Open'),
          run: () => void this.router.navigate(sessionLink(movedSession(m), there)),
        },
      });
    } else if (m.state === 'failed') {
      this.notices.show('error', moveSummary(m, this.words()));
    }
  }

  // ─────────────────────────────────────────────────────────── Following

  /** Ask another computer about one of its moves until it ends. */
  follow(at: string, id: string): void {
    const key = keyOf(at, id);
    if (this.polls.has(key)) return;
    this.polls.set(key, { changed: Date.now() });
    this.schedule(at, id, POLL_MS);
  }

  /** Whether a move is being followed (tests, the progress view). */
  following(at: string, id: string): boolean {
    return this.polls.has(keyOf(at, id));
  }

  private schedule(at: string, id: string, ms: number): void {
    const p = this.polls.get(keyOf(at, id));
    if (!p) return;
    clearTimeout(p.timer);
    this.zone.runOutsideAngular(() => {
      p.timer = setTimeout(() => this.zone.run(() => void this.poll(at, id)), ms);
    });
  }

  private stop(key: string): void {
    clearTimeout(this.polls.get(key)?.timer);
    this.polls.delete(key);
  }

  private async poll(at: string, id: string): Promise<void> {
    const key = keyOf(at, id);
    const p = this.polls.get(key);
    if (!p) return;
    try {
      const m = await this.client(at).getMove(id);
      const prev = this.store().get(key)?.move;
      if (!prev || prev.updatedAt !== m.updatedAt || prev.state !== m.state) {
        p.changed = Date.now();
      }
      this.put(at, m);
      if (moveFinal(m) || Date.now() - p.changed > POLL_GIVE_UP_MS) {
        this.stop(key);
        return;
      }
      this.schedule(at, id, m.resumeError ? POLL_SLOW_MS : POLL_MS);
    } catch (e) {
      // Gone there (its record pruned), or the other computer away for
      // long: stop asking; offline for a while: ask less often.
      if ((e instanceof ApiError && e.status === 404) || Date.now() - p.changed > POLL_GIVE_UP_MS) {
        this.stop(key);
        return;
      }
      this.schedule(at, id, POLL_SLOW_MS);
    }
  }
}
