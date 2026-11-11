import {
  DestroyRef,
  Injectable,
  NgZone,
  Signal,
  computed,
  inject,
  signal,
  untracked,
} from '@angular/core';
import { Observable, Subject } from 'rxjs';

import { normalizeAccounts } from './accounts';
import { ApiError, ApiService } from './api.service';
import { AuthService } from './auth.service';
import { ConfigService } from './config.service';
import { WebSocketLike, activeMock } from './mock/mock-api';
import {
  AccountsView,
  AgentsFrame,
  BrowserFrame,
  ClientFrame,
  FleetFrame,
  HelloFrame,
  NoticeFrame,
  PairRequestFrame,
  PendingFrame,
  PendingRef,
  PendingResolvedFrame,
  PlanUsage,
  ResyncFrame,
  ServerFrame,
  SessionSummary,
  SessionsDiffFrame,
  SessionsFrame,
  StreamFrame,
  TranscriptFrame,
  TranscriptResetFrame,
} from './models';
import { NowService } from './now.service';
import { normalizeAccountUsages, normalizePlanUsage } from './plan-usage';
import type { MachinesFrame, RemoteFrame } from './machines';
import { MoveFrame, normalizeMove } from './moves';
import type { TeamsFrame } from './teams';
import { TranscriptStore, TranscriptStoreOptions } from './transcript-store';
import { computerNoun, tr } from './i18n/i18n';

/**
 * - `connecting`   first connect, or the socket is open but not authed yet
 * - `open`         authed (`hello` received): Live
 * - `limited`      the socket can't be opened (e.g. this phone doesn't trust
 *                  the Mac's LAN certificate for wss) but REST works: polling
 * - `reconnecting` dropped; retrying with backoff (≤ 8 s)
 * - `offline`      the browser reports no network; retry when it returns
 * - `unreachable`  neither the socket nor REST answers
 * - `unpaired`     no token, or the server rejected it (close 4401)
 * - `idle`         not started / stopped
 */
export type ConnectionState =
  | 'idle'
  | 'connecting'
  | 'open'
  | 'limited'
  | 'reconnecting'
  | 'offline'
  | 'unreachable'
  | 'unpaired';

/** Human label for a connection state (the five the phone shows: Live,
 * Limited, Reconnecting, Can't reach Mac, Unpaired). */
export function connectionLabel(state: ConnectionState): string {
  switch (state) {
    case 'open':
      return tr('Live');
    case 'limited':
      return tr('Limited');
    case 'connecting':
      return tr('Connecting…');
    case 'reconnecting':
      return tr('Reconnecting…');
    case 'offline':
      return tr('Offline');
    case 'unreachable':
      return tr("Can't reach the {computer}", { computer: computerNoun() });
    case 'unpaired':
      return tr('Unpaired');
    default:
      return tr('Not connected');
  }
}

/** One sentence explaining a non-Live state (tooltips, the connection sheet). */
export function connectionDetail(state: ConnectionState): string {
  switch (state) {
    case 'open':
      return tr('Connected: updates arrive as they happen.');
    case 'limited':
      return tr(
        "Live updates can't connect (often because this phone doesn't trust your {computer}'s certificate), so the app refreshes every few seconds instead.",
        { computer: computerNoun() },
      );
    case 'connecting':
    case 'reconnecting':
      return tr('Trying to reach your {computer}…', { computer: computerNoun() });
    case 'offline':
      return tr('This device is offline.');
    case 'unreachable':
      return tr("Can't reach your {computer}: it may be asleep, offline, or the tunnel is down.", {
        computer: computerNoun(),
      });
    case 'unpaired':
      return tr('This device is not paired.');
    default:
      return '';
  }
}

/** Why consumers should refetch REST-loaded data. */
export type RefetchReason = 'reconnect' | 'resync' | 'resume' | 'online';

const MAX_BACKOFF_MS = 8000;
/** No hello within this long after the socket opened ⇒ reconnect. */
const AUTH_TIMEOUT_MS = 10_000;
/** The server pings every 25 s; silence this long means a dead socket. */
const SILENCE_LIMIT_MS = 65_000;
/** After this long in the background, treat coming back as a resume. */
const RESUME_AFTER_MS = 30_000;
/** Liveness probe answer time when the page becomes visible again. */
const PROBE_TIMEOUT_MS = 5_000;
/** Socket attempts that never reached `hello` before trying REST polling. */
const HELLO_FAILURES_FOR_POLLING = 3;
/** Polling cadence (sessions + inbox) while visible / hidden. */
const POLL_VISIBLE_MS = 3_000;
const POLL_HIDDEN_MS = 15_000;
/** Open transcripts poll `after=<tail>` this often while limited. */
const TRANSCRIPT_POLL_MS = 2_000;
/** While limited, try the socket again this often. */
const WS_RETRY_LIMITED_MS = 60_000;
/** While limited, fetch the plan usage at most this often (the server reads
 * the account itself; this only copies its last answer). */
const USAGE_POLL_MS = 5 * 60_000;
/** States a frame on an authed socket proves wrong. */
const RESTORABLE: ReadonlySet<ConnectionState> = new Set<ConnectionState>([
  'connecting',
  'reconnecting',
  'offline',
  'unreachable',
]);

interface SubEntry {
  session: string;
  agent?: string;
  count: number;
  tail?: string;
}

function subKey(session: string, agent?: string): string {
  return agent ? `${session}\u0000${agent}` : session;
}

interface RemoteSubEntry extends SubEntry {
  machine: string;
}

function remoteKey(machine: string, session: string, agent?: string): string {
  return `${machine}\u0000${subKey(session, agent)}`;
}

/** Another computer's sessions, as its frames said (docs/PLAN-MACHINES.md
 * §5.6). */
export interface RemoteMachine {
  /** Its sessions by id, each stamped with `machine`. */
  sessions: ReadonlyMap<string, SessionSummary>;
  /** A full list of them arrived. */
  loaded: boolean;
  /** The link to it is up (until an `offline` frame). */
  online: boolean;
  /** When it went offline (ms), once it was heard and lost. */
  since: number | null;
}

/** Another computer's frame, unwrapped from `{t:'remote'}`: its own kinds,
 * and the link's `online`, `offline` and `forgotten`. */
export type RemoteInner =
  | SessionsFrame
  | TranscriptFrame
  | TranscriptResetFrame
  | StreamFrame
  | PendingFrame
  | PendingResolvedFrame
  | AgentsFrame
  | ResyncFrame
  | MoveFrame
  | { t: 'online' }
  | { t: 'offline'; since?: number | null }
  | { t: 'forgotten' };

/** One of another computer's frames, with whose it is. */
export interface RemoteEvent {
  machine: string;
  frame: RemoteInner;
}

/**
 * RealtimeService — the WebSocket v2 client (design contract §4.6 + the §10
 * amendments), with a REST polling fallback for LAN clients whose socket
 * can't connect.
 *
 * State (signals):
 * - `connection`       {@link ConnectionState}; phones always show it.
 * - `canAct`           Live or Limited: sending and approving work.
 * - `lastDataAt`       when anything last arrived ("Updated 2 min ago").
 * - `sessions`         the live session store (scope `active`).
 * - `sessionsLoaded`   true once the first snapshot arrived.
 * - `pendingItems`     every pending preview across the store (with its
 *                      session id), oldest first. Full items: PendingService.
 * - `session(id)`      a signal of one session from the store.
 * - `planUsage`        the Claude plan's usage (account-wide): `planUsage`
 *                      frames, or `GET /usage` at most every 5 min while
 *                      Limited; null until known (or on an older server).
 *                      Always the Default account's.
 * - `accountUsage`     every other account's plan usage, by account id:
 *                      `accountUsage` frames, or `GET /usage` `accounts`.
 * - `accounts`         the Claude accounts: `accounts` frames (sent on
 *                      connect only once more than the Default exists), or
 *                      a REST copy merged by AccountsService; null until
 *                      known.
 *
 * Streams (RxJS; one emission per frame, never coalesced):
 * - `hello$`, `sessions$`, `transcript$`, `transcriptReset$`, `stream$`,
 *   `pending$`, `pendingResolved$`, `agents$`, `fleet$`, `notice$`,
 *   `devices$`, `pairRequest$`.
 * - `refetch$` — REST-loaded data may be stale (reconnect, resync, resume
 *   after ≥ 30 s hidden, network back).
 * - `pollTick$` — every 2 s while Limited and visible; open views poll.
 *
 * Subscriptions (ref-counted; each call returns its release function):
 * - `subscribe(sessionId, agentId?)` ⇒ `sub{session, agent?, since}`, re-sent
 *   after every reconnect with the last transcript tail seen.
 * - `subscribeFleet()` ⇒ `subFleet{}`; `subscribeInbox()` ⇒ `subInbox{}`.
 * - `noteTail(sessionId, tail, agentId?)` seeds `since` from REST.
 * - `openTranscript(sessionId, opts)` ⇒ a {@link TranscriptStore}.
 *
 * Polling fallback: after 3 socket attempts that never reached `hello`, if
 * REST `GET /me` works and this is not a tunnel client (polling would burn a
 * free tunnel's request quota), the service switches to `limited`: it polls
 * `/sessions` (ETag) and `/inbox` every 3 s (15 s hidden) and `/usage` every
 * 5 min, ticks `pollTick$` for open transcripts, and retries the socket
 * every 60 s.
 *
 * Other computers (docs/PLAN-MACHINES.md §5.6): `remote` frames keep
 * `remote` (each computer's sessions, whether it is online) and feed
 * `remote$`; `subscribeRemote(machine, session, agent?)` is `subscribe` for
 * one of their sessions (`sub` with `machine`), re-sent like the others.
 * Nothing of theirs enters `sessions`: their scoped services
 * (`machine-scope.ts`) read them from here. `move$` carries this computer's
 * moves of sessions to and from the others (§5.8), each record parsed.
 */
@Injectable({ providedIn: 'root' })
export class RealtimeService {
  private config = inject(ConfigService);
  private auth = inject(AuthService);
  private api = inject(ApiService);
  private zone = inject(NgZone);
  private clock = inject(NowService);

  private readonly connectionS = signal<ConnectionState>('idle');
  readonly connection = this.connectionS.asReadonly();
  readonly canAct = computed(() => ['open', 'limited'].includes(this.connectionS()));
  readonly polling = computed(() => this.connectionS() === 'limited');
  private readonly lastDataAtS = signal(0);
  readonly lastDataAt = this.lastDataAtS.asReadonly();

  private readonly store = signal<ReadonlyMap<string, SessionSummary>>(new Map());
  readonly sessions: Signal<SessionSummary[]> = computed(() => [...this.store().values()]);
  private readonly sessionsLoadedS = signal(false);
  readonly sessionsLoaded = this.sessionsLoadedS.asReadonly();

  readonly pendingItems: Signal<PendingRef[]> = computed(() =>
    this.sessions()
      .flatMap((s) => (s.pending ?? []).map((p) => ({ ...p, sessionId: s.id })))
      .sort((a, b) => a.createdAt - b.createdAt),
  );

  private readonly planUsageS = signal<PlanUsage | null>(null);
  readonly planUsage = this.planUsageS.asReadonly();
  private readonly planUsageAtS = signal(0);
  /** When the plan usage's current numbers (its `fetchedAt`) reached this
   * device, on the shared clock ({@link NowService}); 0 before any. Stamped
   * on arrival, whether or not a view shows them. */
  readonly planUsageReceivedAt = this.planUsageAtS.asReadonly();
  /** `planUsage` frames applied so far: a REST copy requested before the
   * latest frame is older than it (see {@link mergePlanUsage}). */
  private planUsageFrames = 0;
  private lastUsagePollAt = 0;

  private readonly accountUsageS = signal<ReadonlyMap<string, PlanUsage>>(new Map());
  readonly accountUsage = this.accountUsageS.asReadonly();
  private readonly accountUsageAtS = signal<ReadonlyMap<string, number>>(new Map());
  /** Per account: when its current numbers reached this device (the shared
   * clock), like {@link planUsageReceivedAt}. */
  readonly accountUsageReceivedAt = this.accountUsageAtS.asReadonly();
  /** `accountUsage` frames applied so far, per account. */
  private readonly accountUsageFrames = new Map<string, number>();

  private readonly accountsS = signal<AccountsView | null>(null);
  readonly accounts = this.accountsS.asReadonly();
  /** `accounts` frames applied so far. */
  private accountsFrames = 0;

  private readonly helloS = new Subject<HelloFrame>();
  private readonly sessionsS = new Subject<SessionsFrame>();
  private readonly transcriptS = new Subject<TranscriptFrame>();
  private readonly transcriptResetS = new Subject<TranscriptResetFrame>();
  private readonly streamS = new Subject<StreamFrame>();
  private readonly pendingS = new Subject<PendingFrame>();
  private readonly pendingResolvedS = new Subject<PendingResolvedFrame>();
  private readonly agentsS = new Subject<AgentsFrame>();
  private readonly fleetS = new Subject<FleetFrame>();
  private readonly noticeS = new Subject<NoticeFrame>();
  private readonly devicesS = new Subject<void>();
  private readonly pairRequestS = new Subject<PairRequestFrame>();
  private readonly browserS = new Subject<BrowserFrame>();
  private readonly refetchS = new Subject<RefetchReason>();
  private readonly pollTickS = new Subject<void>();
  /** Teams of sessions: a team, a task of its board, a removed team, or
   * "the templates changed" (docs/TEAMS.md). */
  private readonly teamsS = new Subject<TeamsFrame>();
  readonly teams$: Observable<TeamsFrame> = this.teamsS.asObservable();
  /** Machines changed (pairing, peers, setup, settings…): refetch
   * `/api/machines` (docs/PLAN-MACHINES.md). */
  private readonly machinesS = new Subject<MachinesFrame>();
  readonly machines$: Observable<MachinesFrame> = this.machinesS.asObservable();
  /** A move of a session to or from another computer changed: its whole
   * record (docs/PLAN-MACHINES.md §5.8). */
  private readonly moveS = new Subject<MoveFrame>();
  readonly move$: Observable<MoveFrame> = this.moveS.asObservable();
  /** Other computers' sessions, by machine id, as their frames said. */
  private readonly remoteS = signal<ReadonlyMap<string, RemoteMachine>>(new Map());
  readonly remote = this.remoteS.asReadonly();
  /** Every other computer's frame, unwrapped (their scoped services filter
   * it by machine). */
  private readonly remoteEventS = new Subject<RemoteEvent>();
  readonly remote$: Observable<RemoteEvent> = this.remoteEventS.asObservable();

  readonly hello$: Observable<HelloFrame> = this.helloS.asObservable();
  readonly sessions$: Observable<SessionsFrame> = this.sessionsS.asObservable();
  readonly transcript$: Observable<TranscriptFrame> = this.transcriptS.asObservable();
  readonly transcriptReset$: Observable<TranscriptResetFrame> =
    this.transcriptResetS.asObservable();
  readonly stream$: Observable<StreamFrame> = this.streamS.asObservable();
  readonly pending$: Observable<PendingFrame> = this.pendingS.asObservable();
  readonly pendingResolved$: Observable<PendingResolvedFrame> =
    this.pendingResolvedS.asObservable();
  readonly agents$: Observable<AgentsFrame> = this.agentsS.asObservable();
  readonly fleet$: Observable<FleetFrame> = this.fleetS.asObservable();
  readonly notice$: Observable<NoticeFrame> = this.noticeS.asObservable();
  readonly devices$: Observable<void> = this.devicesS.asObservable();
  readonly pairRequest$: Observable<PairRequestFrame> = this.pairRequestS.asObservable();
  /** The browser extension's state changed: refetch `/api/browser`. */
  readonly browser$: Observable<BrowserFrame> = this.browserS.asObservable();
  readonly refetch$: Observable<RefetchReason> = this.refetchS.asObservable();
  readonly pollTick$: Observable<void> = this.pollTickS.asObservable();

  private socket: WebSocketLike | null = null;
  private started = false;
  private authed = false;
  private everConnected = false;
  private attempt = 0;
  private helloFailures = 0;
  private lastSeq = -1;
  private lastFrameAt = 0;
  private hiddenAt = 0;
  private reconnectTimer?: ReturnType<typeof setTimeout>;
  private authTimer?: ReturnType<typeof setTimeout>;
  private probeTimer?: ReturnType<typeof setTimeout>;
  private watchdog?: ReturnType<typeof setInterval>;
  private pollTimer?: ReturnType<typeof setTimeout>;
  private transcriptPoll?: ReturnType<typeof setInterval>;
  private pollEtag?: string;
  private pollBusy = false;
  private checkingLimited = false;
  /** This client may not poll (a tunnel client): don't ask again. */
  private noPolling = false;
  /** Set while a gap reload is in flight: the diffs applied meanwhile. */
  private diffsDuringReload: SessionsDiffFrame[] | null = null;
  /** Full `sessions` frames applied so far. */
  private fullFrames = 0;
  private listenersBound = false;
  private readonly subs = new Map<string, SubEntry>();
  /** Other computers' sessions subscribed to, by `remoteKey`. */
  private readonly remoteSubs = new Map<string, RemoteSubEntry>();
  private fleetSubs = 0;
  private inboxSubs = 0;

  /** Open the socket (once a token exists). Safe to call repeatedly. */
  async start(): Promise<void> {
    if (this.started) return;
    this.started = true;
    await this.config.ensure();
    this.bindGlobalListeners();
    this.connect();
  }

  /** Reconnect from scratch, e.g. right after pairing stored a new token. */
  restart(): void {
    this.started = true;
    this.bindGlobalListeners();
    this.attempt = 0;
    this.helloFailures = 0;
    this.noPolling = false;
    this.stopPolling();
    this.teardown();
    this.connect();
  }

  /** Close the socket and stop reconnecting. */
  stop(): void {
    this.started = false;
    this.stopPolling();
    this.teardown();
    this.connectionS.set('idle');
  }

  /** Try the socket right now (the connection sheet's "Reconnect"). */
  reconnectNow(): void {
    if (!this.started) return;
    this.attempt = 0;
    this.teardown();
    this.connect();
  }

  /** A signal of one session from the live store. */
  session(id: string): Signal<SessionSummary | undefined> {
    return computed(() => this.store().get(id));
  }

  /** Merge a REST copy of a session into the store (e.g. the detail page's
   * `GET /sessions/{id}` while polling). Only sessions already in the store
   * are updated, so the active scope stays the server's decision. While the
   * socket is live its diffs are authoritative: a REST copy may be older than
   * the last diff, and the server only diffs again on the next change. */
  mergeSession(s: SessionSummary): void {
    if (this.connectionS() === 'open') return;
    if (!s?.id || !this.store().has(s.id)) return;
    this.store.update((prev) => new Map(prev).set(s.id, s));
  }

  /** Take before a plan-usage REST request; pass to {@link mergePlanUsage}. */
  planUsageMark(): number {
    return this.planUsageFrames;
  }

  /** Apply a REST copy of the plan usage (`GET /usage`, or the answer to a
   * refresh or a settings change). A `planUsage` frame that arrived after
   * `mark` was taken is at least as new, so the copy is then dropped.
   * Returns whether it was applied. */
  mergePlanUsage(raw: PlanUsage | null | undefined, mark = this.planUsageFrames): boolean {
    const u = normalizePlanUsage(raw);
    if (!u || mark !== this.planUsageFrames) return false;
    this.setPlanUsage(u);
    return true;
  }

  /** Change a few fields of the known plan usage in place (e.g. the
   * interval the server just accepted). */
  patchPlanUsage(change: Partial<PlanUsage>): void {
    const cur = untracked(this.planUsageS);
    if (cur) this.setPlanUsage({ ...cur, ...change });
  }

  /** Keep the plan usage, noting when new numbers arrived. */
  private setPlanUsage(u: PlanUsage): void {
    untracked(() => {
      if (this.planUsageS()?.fetchedAt !== u.fetchedAt) this.planUsageAtS.set(this.clock.now());
      this.planUsageS.set(u);
    });
  }

  /** Take before a `GET /usage`; pass to {@link mergeAccountUsages}. */
  accountUsageMark(): ReadonlyMap<string, number> {
    return new Map(this.accountUsageFrames);
  }

  /** Apply the other accounts' usage from a `GET /usage` answer (its
   * `accounts`). An account whose `accountUsage` frame arrived after `mark`
   * was taken keeps the frame's copy. */
  mergeAccountUsages(
    reply: unknown,
    mark: ReadonlyMap<string, number> = this.accountUsageFrames,
  ): void {
    for (const [id, u] of normalizeAccountUsages(reply)) {
      if ((mark.get(id) ?? 0) !== (this.accountUsageFrames.get(id) ?? 0)) continue;
      this.setAccountUsage(id, u);
    }
  }

  private setAccountUsage(id: string, u: PlanUsage): void {
    untracked(() => {
      if (this.accountUsageS().get(id)?.fetchedAt !== u.fetchedAt) {
        this.accountUsageAtS.update((prev) => new Map(prev).set(id, this.clock.now()));
      }
      this.accountUsageS.update((prev) => new Map(prev).set(id, u));
    });
  }

  /** Take before a `GET /accounts`; pass to {@link mergeAccounts}. */
  accountsMark(): number {
    return this.accountsFrames;
  }

  /** Apply a REST copy of the accounts. An `accounts` frame that arrived
   * after `mark` was taken is at least as new, so the copy is then dropped.
   * Returns whether it was applied. */
  mergeAccounts(raw: unknown, mark = this.accountsFrames): boolean {
    const view = normalizeAccounts(raw);
    if (!view || mark !== this.accountsFrames) return false;
    this.accountsS.set(view);
    return true;
  }

  /** Ref-counted `sub`; call the returned function to release it. */
  subscribe(sessionId: string, agentId?: string): () => void {
    const key = subKey(sessionId, agentId);
    const entry = this.subs.get(key);
    if (entry) {
      entry.count += 1;
    } else {
      this.subs.set(key, { session: sessionId, agent: agentId, count: 1 });
      this.sendSub(key);
    }
    let released = false;
    return () => {
      if (released) return;
      released = true;
      const e = this.subs.get(key);
      if (!e) return;
      e.count -= 1;
      if (e.count <= 0) {
        this.subs.delete(key);
        this.send(
          agentId
            ? { t: 'unsub', session: sessionId, agent: agentId }
            : { t: 'unsub', session: sessionId },
        );
      }
    };
  }

  /** Ref-counted `subFleet`; call the returned function to release it. */
  subscribeFleet(): () => void {
    this.fleetSubs += 1;
    if (this.fleetSubs === 1) this.send({ t: 'subFleet' });
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.fleetSubs -= 1;
      if (this.fleetSubs === 0) this.send({ t: 'unsubFleet' });
    };
  }

  /** Ref-counted `subInbox` (full `pending{item}` frames for every session). */
  subscribeInbox(): () => void {
    this.inboxSubs += 1;
    if (this.inboxSubs === 1) this.send({ t: 'subInbox' });
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.inboxSubs -= 1;
      if (this.inboxSubs === 0) this.send({ t: 'unsubInbox' });
    };
  }

  /** Record the newest transcript cursor we hold (used as `since` on resubscribe). */
  noteTail(sessionId: string, tail: string | undefined, agentId?: string): void {
    const e = this.subs.get(subKey(sessionId, agentId));
    if (e && tail) e.tail = tail;
  }

  /** A live, deduped transcript for one session (or one of its agents).
   * Pass `destroyRef` to release it automatically, else call `destroy()`. */
  openTranscript(
    sessionId: string,
    opts: TranscriptStoreOptions & { destroyRef?: DestroyRef } = {},
  ): TranscriptStore {
    const store = new TranscriptStore(this.api, this, sessionId, opts);
    opts.destroyRef?.onDestroy(() => store.destroy());
    return store;
  }

  // ─────────────────────────────────────────────────── Other computers

  /** Ref-counted `sub` of another computer's session (or one of its
   * agents): `sub{session, agent?, since?, machine}`, re-sent after every
   * reconnect. Call the returned function to release it. */
  subscribeRemote(machine: string, sessionId: string, agentId?: string): () => void {
    const key = remoteKey(machine, sessionId, agentId);
    const entry = this.remoteSubs.get(key);
    if (entry) {
      entry.count += 1;
    } else {
      this.remoteSubs.set(key, { machine, session: sessionId, agent: agentId, count: 1 });
      this.sendRemoteSub(key);
    }
    let released = false;
    return () => {
      if (released) return;
      released = true;
      const e = this.remoteSubs.get(key);
      if (!e) return;
      e.count -= 1;
      if (e.count <= 0) {
        this.remoteSubs.delete(key);
        this.send(
          agentId
            ? { t: 'unsub', session: sessionId, agent: agentId, machine }
            : { t: 'unsub', session: sessionId, machine },
        );
      }
    };
  }

  /** The newest transcript cursor held for another computer's session. */
  noteRemoteTail(
    machine: string,
    sessionId: string,
    tail: string | undefined,
    agentId?: string,
  ): void {
    const e = this.remoteSubs.get(remoteKey(machine, sessionId, agentId));
    if (e && tail) e.tail = tail;
  }

  /** A REST copy of another computer's session (its page's detail), merged
   * as {@link mergeSession} merges this computer's. */
  mergeRemoteSession(machine: string, s: SessionSummary): void {
    if (this.connectionS() === 'open') return;
    const m = this.remoteS().get(machine);
    if (!s?.id || !m?.sessions.has(s.id)) return;
    this.setRemote(machine, { ...m, sessions: new Map(m.sessions).set(s.id, { ...s, machine }) });
  }

  // ────────────────────────────────────────────────────────── Connection

  private connect(): void {
    clearTimeout(this.reconnectTimer);
    if (!this.started) return;
    const cfg = this.config.config();
    if (!cfg) return;
    if (this.auth.state() === 'unpaired' || (!cfg.token && !cfg.desktop)) {
      this.connectionS.set('unpaired');
      return;
    }
    if (navigator.onLine === false) {
      this.connectionS.set('offline');
      return;
    }
    // Limited and unreachable are sticky until something actually changes.
    if (this.connectionS() !== 'limited' && this.connectionS() !== 'unreachable') {
      this.connectionS.set(this.everConnected ? 'reconnecting' : 'connecting');
    }
    this.authed = false;

    let socket: WebSocketLike;
    try {
      const mock = activeMock();
      socket = mock ? mock.openSocket(cfg.wsUrl) : new WebSocket(cfg.wsUrl);
    } catch {
      this.noteHelloFailure();
      this.scheduleReconnect();
      return;
    }
    this.socket = socket;

    socket.onopen = () => {
      if (this.socket !== socket) return;
      this.lastFrameAt = Date.now();
      this.sendRaw(socket, { t: 'auth', token: this.config.token() });
      clearTimeout(this.authTimer);
      this.authTimer = setTimeout(() => {
        if (this.socket === socket && !this.authed) {
          this.noteHelloFailure();
          this.teardown();
          this.scheduleReconnect();
        }
      }, AUTH_TIMEOUT_MS);
    };
    socket.onmessage = (ev: MessageEvent) => {
      if (this.socket !== socket) return;
      this.lastFrameAt = Date.now();
      this.lastDataAtS.set(this.lastFrameAt);
      // A frame on an authed socket proves the link works, whatever an
      // `offline` event (e.g. a Wi-Fi hand-off the socket survived) said.
      if (this.authed && RESTORABLE.has(this.connectionS())) this.connectionS.set('open');
      this.dispatch(ev.data);
    };
    socket.onclose = (ev: CloseEvent) => {
      if (this.socket !== socket) return;
      const wasAuthed = this.authed;
      this.socket = null;
      this.authed = false;
      clearTimeout(this.authTimer);
      if (ev.code === 4401 || ev.code === 4403) {
        if (this.config.desktop()) {
          // The desktop's token belongs to the app: flag it and keep trying.
          this.auth.handleUnauthorized();
        } else {
          // Don't forget a device token on the socket's word alone: the
          // close may be a late auth frame (a phone frozen right after
          // the upgrade). REST decides; a 401 there unpairs.
          if (this.connectionS() === 'open') this.connectionS.set('reconnecting');
          void this.confirmUnauthorized(wasAuthed);
          return;
        }
      }
      if (!wasAuthed) this.noteHelloFailure();
      this.scheduleReconnect();
    };
    socket.onerror = () => {
      /* onclose follows and handles the retry */
    };
  }

  private scheduleReconnect(): void {
    if (!this.started) return;
    if (navigator.onLine === false) {
      this.connectionS.set('offline');
      return;
    }
    clearTimeout(this.reconnectTimer);
    if (this.connectionS() === 'limited') {
      // Polling keeps the data fresh; only probe the socket now and then.
      this.reconnectTimer = setTimeout(() => this.connect(), WS_RETRY_LIMITED_MS);
      return;
    }
    if (this.connectionS() !== 'unreachable') {
      this.connectionS.set(this.everConnected ? 'reconnecting' : 'connecting');
    }
    const base = Math.min(MAX_BACKOFF_MS, 500 * 2 ** this.attempt);
    const delay = Math.min(MAX_BACKOFF_MS, Math.round(base * (0.75 + Math.random() * 0.5)));
    this.attempt += 1;
    this.reconnectTimer = setTimeout(() => this.connect(), delay);
  }

  /** The socket was closed as unauthorized: ask REST whether the token is
   * really gone. A 401 there unpairs (ApiService reports it to AuthService);
   * a working token or a network failure just means "try again". */
  private async confirmUnauthorized(wasAuthed: boolean): Promise<void> {
    try {
      const me = await this.api.getMe();
      this.auth.me.set(me);
    } catch (e) {
      if (e instanceof ApiError && e.kind === 'unauthorized') {
        this.stopPolling();
        this.connectionS.set('unpaired');
        return;
      }
    }
    if (!this.started || this.socket || this.auth.state() === 'unpaired') return;
    if (!wasAuthed) this.noteHelloFailure();
    this.scheduleReconnect();
  }

  /** Drop the current socket and connect again right away. */
  private forceReconnect(): void {
    this.attempt = 0;
    this.teardown();
    this.connect();
  }

  private teardown(): void {
    clearTimeout(this.reconnectTimer);
    clearTimeout(this.authTimer);
    clearTimeout(this.probeTimer);
    const s = this.socket;
    this.socket = null;
    this.authed = false;
    if (s) {
      s.onopen = s.onmessage = s.onclose = s.onerror = null;
      try {
        s.close(1000, 'client reconnect');
      } catch {
        /* already closed */
      }
    }
  }

  // ───────────────────────────────────────────────── Polling fallback

  private noteHelloFailure(): void {
    this.helloFailures += 1;
    if (
      this.helloFailures >= HELLO_FAILURES_FOR_POLLING &&
      this.connectionS() !== 'limited' &&
      !this.noPolling
    ) {
      void this.maybeStartPolling();
    }
  }

  /** The socket keeps failing: is REST reachable, and may we poll? */
  private async maybeStartPolling(): Promise<void> {
    if (this.checkingLimited || !this.started) return;
    this.checkingLimited = true;
    try {
      const me = await this.api.getMe();
      this.auth.me.set(me);
      if (this.authed) return; // the socket came back meanwhile
      // A tunnel pointed at the main port arrives on the loopback listener:
      // `effectiveIngress` is the network the request really came from.
      if ((me.effectiveIngress ?? me.ingress) === 'tunnel') {
        // Over a tunnel the socket normally works, and polling would burn
        // the tunnel's request quota: keep retrying the socket instead.
        this.noPolling = true;
        if (this.connectionS() === 'unreachable') this.connectionS.set('reconnecting');
        return;
      }
      this.startPolling();
    } catch (e) {
      if (e instanceof ApiError && e.kind === 'network' && !this.authed) {
        this.connectionS.set(navigator.onLine === false ? 'offline' : 'unreachable');
        // Ask again after a few more failed attempts, not on every one.
        this.helloFailures = 0;
      }
    } finally {
      this.checkingLimited = false;
    }
  }

  private startPolling(): void {
    if (this.connectionS() === 'limited') return;
    this.connectionS.set('limited');
    // The socket is probed again in a minute (scheduleReconnect sees limited).
    clearTimeout(this.reconnectTimer);
    this.reconnectTimer = setTimeout(() => this.connect(), WS_RETRY_LIMITED_MS);
    this.zone.runOutsideAngular(() => {
      this.transcriptPoll = setInterval(() => {
        if (document.visibilityState === 'visible') this.zone.run(() => this.pollTickS.next());
      }, TRANSCRIPT_POLL_MS);
    });
    void this.pollOnce();
  }

  private stopPolling(): void {
    clearTimeout(this.pollTimer);
    clearInterval(this.transcriptPoll);
    this.pollTimer = undefined;
    this.transcriptPoll = undefined;
    this.pollEtag = undefined;
  }

  private async pollOnce(): Promise<void> {
    // One loop only: a poll in flight schedules the next one itself.
    if (this.connectionS() !== 'limited' || this.pollBusy) return;
    this.pollBusy = true;
    try {
      const res = await this.api.pollSessions(this.pollEtag);
      if (res) {
        this.pollEtag = res.etag;
        this.replaceStore(res.page.sessions ?? []);
        this.sessionsS.next({
          t: 'sessions',
          full: true,
          sessions: res.page.sessions ?? [],
          seq: -1,
        });
      }
      this.lastDataAtS.set(Date.now());
      const inbox = await this.api.getInbox();
      for (const item of inbox.items ?? []) this.pendingS.next({ t: 'pending', item });
      if (Date.now() - this.lastUsagePollAt >= USAGE_POLL_MS) {
        this.lastUsagePollAt = Date.now();
        void this.pollPlanUsage();
      }
    } catch (e) {
      if (e instanceof ApiError && e.kind === 'network') {
        // REST is gone too; the socket retry decides what comes next.
        this.pollBusy = false;
        this.stopPolling();
        this.connectionS.set(navigator.onLine === false ? 'offline' : 'unreachable');
        this.helloFailures = 0;
        this.scheduleReconnect();
        return;
      }
    }
    this.pollBusy = false;
    if (this.connectionS() !== 'limited') return;
    const wait = document.visibilityState === 'visible' ? POLL_VISIBLE_MS : POLL_HIDDEN_MS;
    clearTimeout(this.pollTimer);
    this.pollTimer = setTimeout(() => void this.pollOnce(), wait);
  }

  /** Copy the server's plan usage (Limited only; never on the 3 s loop). */
  private async pollPlanUsage(): Promise<void> {
    const mark = this.planUsageFrames;
    const accountsMark = this.accountUsageMark();
    try {
      const reply = await this.api.getPlanUsage();
      this.mergePlanUsage(reply, mark);
      this.mergeAccountUsages(reply, accountsMark);
    } catch {
      // An older server has no /usage (the UI stays hidden); a network
      // failure is the sessions poll's business.
    }
  }

  // ───────────────────────────────────────────────────────────── Frames

  private dispatch(raw: unknown): void {
    if (typeof raw !== 'string') return;
    let frame: ServerFrame;
    try {
      frame = JSON.parse(raw) as ServerFrame;
    } catch {
      return;
    }
    if (!frame || typeof frame !== 'object' || typeof frame.t !== 'string') return;
    switch (frame.t) {
      case 'hello':
        this.onHello(frame);
        break;
      case 'sessions':
        this.onSessions(frame);
        break;
      case 'transcript': {
        const e = this.subs.get(subKey(frame.session, frame.agent));
        if (e && frame.tail) e.tail = frame.tail;
        this.transcriptS.next(frame);
        break;
      }
      case 'transcriptReset':
        this.transcriptResetS.next(frame);
        break;
      case 'stream':
        this.streamS.next(frame);
        break;
      case 'pending':
        if (frame.item?.id) this.pendingS.next(frame);
        break;
      case 'pendingResolved':
        if (frame.id) this.pendingResolvedS.next(frame);
        break;
      case 'agents':
        this.agentsS.next(frame);
        break;
      case 'fleet':
        this.fleetS.next(frame);
        break;
      case 'planUsage': {
        const usage = normalizePlanUsage(frame.usage);
        if (usage) {
          this.planUsageFrames += 1;
          this.setPlanUsage(usage);
        }
        break;
      }
      case 'accountUsage': {
        // Another account's usage: never the Default's `planUsage`.
        const usage = normalizePlanUsage(frame.usage);
        const id = typeof frame.accountId === 'string' ? frame.accountId : '';
        if (usage && id) {
          this.accountUsageFrames.set(id, (this.accountUsageFrames.get(id) ?? 0) + 1);
          this.setAccountUsage(id, usage);
        }
        break;
      }
      case 'accounts': {
        const view = normalizeAccounts(frame.accounts);
        if (view) {
          this.accountsFrames += 1;
          this.accountsS.set(view);
        }
        break;
      }
      case 'notice':
        this.noticeS.next(frame);
        break;
      case 'devices':
        this.devicesS.next();
        break;
      case 'pairRequest':
        this.pairRequestS.next(frame);
        break;
      case 'browser':
        this.browserS.next(frame);
        break;
      case 'resync':
        this.resync();
        break;
      case 'ping':
        this.send({ t: 'pong' });
        break;
      case 'pong':
        clearTimeout(this.probeTimer);
        break;
      case 'teams':
        this.teamsS.next(frame);
        break;
      case 'machines':
        this.machinesS.next(frame);
        break;
      case 'move': {
        const move = normalizeMove(frame.move);
        if (move) this.moveS.next({ t: 'move', move });
        break;
      }
      case 'remote':
        this.onRemote(frame);
        break;
      default:
        // Unknown frame kinds from a newer server are ignored.
        break;
    }
  }

  /** Another computer's frame: its sessions and the link's state are kept
   * here; everything it shows is passed on (`remote$`). */
  private onRemote(f: RemoteFrame): void {
    const machine = typeof f.machine === 'string' ? f.machine : '';
    const raw = f.frame as { t?: unknown } | null | undefined;
    if (!machine || !raw || typeof raw !== 'object' || typeof raw.t !== 'string') return;
    const frame = raw as RemoteInner;
    switch (frame.t) {
      case 'sessions':
        this.applyRemoteSessions(machine, frame);
        break;
      case 'online':
        this.patchRemote(machine, { online: true, since: null });
        break;
      case 'offline':
        this.patchRemote(machine, {
          online: false,
          since: typeof frame.since === 'number' ? frame.since : null,
        });
        break;
      case 'forgotten':
        this.remoteS.update((prev) => {
          const next = new Map(prev);
          next.delete(machine);
          return next;
        });
        break;
      case 'transcript': {
        const e = this.remoteSubs.get(remoteKey(machine, frame.session, frame.agent));
        if (e && frame.tail) e.tail = frame.tail;
        break;
      }
      case 'pending':
        if (!frame.item?.id) return;
        break;
      case 'pendingResolved':
        if (!frame.id) return;
        break;
      case 'resync':
        // It dropped frames for us: subscribe again (a full list follows).
        for (const [key, e] of this.remoteSubs) if (e.machine === machine) this.sendRemoteSub(key);
        break;
      case 'move': {
        // Its own moves, should it pass them on (`MovesService` polls the
        // ones it follows meanwhile).
        const move = normalizeMove(frame.move);
        if (!move) return;
        this.remoteEventS.next({ machine, frame: { t: 'move', move } });
        return;
      }
      case 'transcriptReset':
      case 'stream':
      case 'agents':
        break;
      default:
        // Nothing else of another computer's is shown here.
        return;
    }
    this.remoteEventS.next({ machine, frame });
  }

  /** A list of another computer's sessions arrived (whole or a diff): it is
   * online. */
  private applyRemoteSessions(machine: string, frame: SessionsFrame): void {
    const prev = this.remoteS().get(machine);
    const sessions = new Map<string, SessionSummary>(frame.full ? undefined : prev?.sessions);
    if (!frame.full) for (const id of frame.remove ?? []) sessions.delete(id);
    for (const s of (frame.full ? frame.sessions : frame.upsert) ?? []) {
      if (s?.id) sessions.set(s.id, { ...s, machine });
    }
    this.setRemote(machine, {
      sessions,
      loaded: !!prev?.loaded || !!frame.full,
      online: true,
      since: null,
    });
  }

  private patchRemote(machine: string, patch: Partial<RemoteMachine>): void {
    const prev: RemoteMachine = this.remoteS().get(machine) ?? {
      sessions: new Map(),
      loaded: false,
      online: true,
      since: null,
    };
    this.setRemote(machine, { ...prev, ...patch });
  }

  private setRemote(machine: string, m: RemoteMachine): void {
    this.remoteS.update((prev) => new Map(prev).set(machine, m));
  }

  private onHello(frame: HelloFrame): void {
    clearTimeout(this.authTimer);
    const wasLimited = this.connectionS() === 'limited';
    this.stopPolling();
    this.authed = true;
    this.attempt = 0;
    this.helloFailures = 0;
    this.connectionS.set('open');
    if (frame.me) this.auth.me.set(frame.me);
    this.send({ t: 'visible', v: document.visibilityState === 'visible' });
    for (const key of this.subs.keys()) this.sendSub(key);
    for (const key of this.remoteSubs.keys()) this.sendRemoteSub(key);
    if (this.fleetSubs > 0) this.send({ t: 'subFleet' });
    if (this.inboxSubs > 0) this.send({ t: 'subInbox' });
    const reconnect = this.everConnected || wasLimited;
    this.everConnected = true;
    this.helloS.next(frame);
    if (reconnect) this.refetchS.next('reconnect');
  }

  private onSessions(frame: SessionsFrame): void {
    if (frame.full) {
      this.replaceStore(frame.sessions ?? []);
      this.lastSeq = typeof frame.seq === 'number' ? frame.seq : -1;
      this.fullFrames += 1;
    } else {
      const seq = typeof frame.seq === 'number' ? frame.seq : -1;
      if (seq >= 0 && this.lastSeq >= 0 && seq <= this.lastSeq) return; // stale
      const gap = seq >= 0 && this.lastSeq >= 0 && seq !== this.lastSeq + 1;
      this.lastSeq = seq;
      this.applyDiff(frame);
      this.diffsDuringReload?.push(frame);
      if (gap) void this.reloadSessions();
    }
    this.sessionsS.next(frame);
  }

  private applyDiff(frame: SessionsDiffFrame): void {
    this.store.update((prev) => {
      const next = new Map(prev);
      for (const id of frame.remove ?? []) next.delete(id);
      for (const s of frame.upsert ?? []) if (s?.id) next.set(s.id, s);
      return next;
    });
  }

  private replaceStore(list: SessionSummary[]): void {
    const next = new Map<string, SessionSummary>();
    for (const s of list) if (s?.id) next.set(s.id, s);
    this.store.set(next);
    this.sessionsLoadedS.set(true);
  }

  /** The server dropped frames for us: resubscribe and tell REST-loaded
   * views to refetch. The store needs nothing: the server follows `resync`
   * with a full `sessions` frame (a REST reload here could land after newer
   * diffs and roll the store back). */
  private resync(): void {
    for (const key of this.subs.keys()) this.sendSub(key);
    for (const key of this.remoteSubs.keys()) this.sendRemoteSub(key);
    if (this.fleetSubs > 0) this.send({ t: 'subFleet' });
    if (this.inboxSubs > 0) this.send({ t: 'subInbox' });
    this.refetchS.next('resync');
  }

  /** Replace the store from REST after a sequence gap. Diffs that arrive
   * while the request is in flight are replayed on top of the REST copy
   * (it may predate them); a full frame meanwhile makes the copy moot. */
  private async reloadSessions(): Promise<void> {
    if (this.diffsDuringReload) return;
    const diffs: SessionsDiffFrame[] = [];
    this.diffsDuringReload = diffs;
    const fullBefore = this.fullFrames;
    try {
      const page = await this.api.listSessions({ scope: 'active' });
      if (this.fullFrames !== fullBefore) return;
      this.replaceStore(page.sessions ?? []);
      for (const d of diffs) this.applyDiff(d);
    } catch {
      // The next reconnect's full snapshot will repair the store.
    } finally {
      this.diffsDuringReload = null;
    }
  }

  private sendSub(key: string): void {
    const e = this.subs.get(key);
    if (!e) return;
    const frame: Extract<ClientFrame, { t: 'sub' }> = { t: 'sub', session: e.session };
    if (e.agent) frame.agent = e.agent;
    if (e.tail) frame.since = e.tail;
    this.send(frame);
  }

  private sendRemoteSub(key: string): void {
    const e = this.remoteSubs.get(key);
    if (!e) return;
    const frame: Extract<ClientFrame, { t: 'sub' }> = {
      t: 'sub',
      session: e.session,
      machine: e.machine,
    };
    if (e.agent) frame.agent = e.agent;
    if (e.tail) frame.since = e.tail;
    this.send(frame);
  }

  private send(frame: ClientFrame): void {
    const s = this.socket;
    if (!s || !this.authed) return; // re-sent after the next hello
    this.sendRaw(s, frame);
  }

  private sendRaw(socket: WebSocketLike, frame: ClientFrame): void {
    if (socket.readyState !== 1) return;
    try {
      socket.send(JSON.stringify(frame));
    } catch {
      /* the close handler reconnects */
    }
  }

  // ────────────────────────────────────── Visibility, network, watchdog

  private bindGlobalListeners(): void {
    if (this.listenersBound) return;
    this.listenersBound = true;
    document.addEventListener('visibilitychange', () => this.onVisibility());
    window.addEventListener('online', () => {
      this.attempt = 0;
      if (!this.socket) this.connect();
      else this.probe();
      this.refetchS.next('online');
    });
    window.addEventListener('offline', () => {
      if (this.started && this.connectionS() !== 'unpaired') {
        this.stopPolling();
        this.connectionS.set('offline');
        // The socket may survive (a localhost socket always does): the probe
        // either brings a frame back (⇒ open again) or reconnects.
        this.probe();
      }
    });
    this.zone.runOutsideAngular(() => {
      this.watchdog = setInterval(() => {
        if (!this.socket || !this.authed) return;
        if (Date.now() - this.lastFrameAt > SILENCE_LIMIT_MS) {
          this.zone.run(() => this.forceReconnect());
        }
      }, 15_000);
    });
  }

  private onVisibility(): void {
    const visible = document.visibilityState === 'visible';
    this.send({ t: 'visible', v: visible });
    if (!visible) {
      this.hiddenAt = Date.now();
      return;
    }
    if (!this.started || this.connectionS() === 'unpaired') return;
    const away = this.hiddenAt ? Date.now() - this.hiddenAt : 0;
    this.hiddenAt = 0;
    if (this.connectionS() === 'limited') {
      // Poll right away instead of waiting out the hidden cadence.
      clearTimeout(this.pollTimer);
      void this.pollOnce();
      return;
    }
    if (!this.socket) {
      // Waiting in backoff: don't make a returning user wait.
      this.attempt = 0;
      this.connect();
      return;
    }
    // Phones freeze sockets in the background without closing them: check
    // the socket still answers, else reconnect (which refetches).
    this.probe();
    if (away >= RESUME_AFTER_MS && this.authed) this.refetchS.next('resume');
  }

  /** Ping the server; no frame back within 5 s ⇒ the socket is dead. */
  private probe(): void {
    if (!this.socket || !this.authed) return;
    const sentAt = Date.now();
    this.send({ t: 'ping' });
    clearTimeout(this.probeTimer);
    this.probeTimer = setTimeout(() => {
      if (this.lastFrameAt < sentAt) this.forceReconnect();
    }, PROBE_TIMEOUT_MS);
  }
}
