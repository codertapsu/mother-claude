import { Injectable, computed, effect, inject, signal, untracked } from '@angular/core';

import { AccountsService } from './accounts.service';
import { FixItem, fixItems } from './attention';
import { AuthService } from './auth.service';
import { CopiesService } from './copies.service';
import { DataJobsService } from './data-jobs.service';
import { tr } from './i18n/i18n';
import { askOpen } from './machines';
import { MachinesService } from './machines.service';
import { NoticeService } from './notice.service';
import { NowService } from './now.service';
import { PairRequestsService } from './pair-requests.service';
import { PermissionsService } from './permissions.service';
import { RealtimeService } from './realtime.service';
import { RemoteSessionsService } from './remote-sessions.service';
import { storageGet, storageSet } from './platform';
import { SyncService } from './sync.service';
import { UpdaterService } from './updater.service';

const ALERTS_KEY = 'mc_alerts';
const DISMISSED_KEY = 'mc_inbox_dismissed';
/** A put-away item is forgotten after this long (it is gone by then). */
const DISMISSED_FOR_MS = 2 * 24 * 3600_000;
const BASE_TITLE = 'Mother Claude';

/**
 * Everything that waits for you (docs/UI-REDESIGN.md §6.4), for the Inbox
 * and its badge: the requests someone can answer from here ({@link
 * pendingCount}: the badge, the tab title and the app badge) — every
 * session's, other computers' online ones, and on the desktop the devices
 * waiting to pair and what other computers asked — and what to fix ({@link
 * toFix}: a dot). Read only from what the app already loads; once started
 * (the shell, when paired) it follows Machines, the accounts and the data
 * jobs, and code sync and copies while Machines is in use.
 *
 * Also "something needs you" without push: an optional short vibration +
 * tone when a new request arrives while the app is open. This is the
 * fallback on LAN (self-signed certificates can't do Web Push).
 */
@Injectable({ providedIn: 'root' })
export class AttentionService {
  private realtime = inject(RealtimeService);
  private notices = inject(NoticeService);
  private remote = inject(RemoteSessionsService);
  private readonly auth = inject(AuthService);
  private readonly machines = inject(MachinesService);
  private readonly pairs = inject(PairRequestsService);
  private readonly sync = inject(SyncService);
  private readonly copies = inject(CopiesService);
  private readonly jobs = inject(DataJobsService);
  private readonly accounts = inject(AccountsService);
  private readonly permissions = inject(PermissionsService);
  private readonly updater = inject(UpdaterService);
  private readonly clock = inject(NowService);

  private readonly started = signal(false);

  /** Devices waiting to pair (answered on the desktop only). */
  readonly pairRequests = computed(() => (this.auth.isDesktop() ? this.pairs.pending() : []));
  /** What other computers asked this one and is not done yet (shown
   * everywhere, answered on the desktop). */
  readonly asks = computed(() => (this.machines.view()?.asks ?? []).filter(askOpen));

  /** Requests someone can answer from here (observed ones wait where they
   * run), other computers' online ones included; on the desktop the
   * devices waiting to pair and other computers' asks too. */
  readonly pendingCount = computed(() => {
    const sessions =
      this.realtime.pendingItems().filter((p) => p.answerable).length +
      this.remote.pending().filter((p) => p.answerable).length;
    if (!this.auth.isDesktop()) return sessions;
    return (
      sessions + this.pairRequests().length + this.asks().filter((a) => a.state === 'asked').length
    );
  });

  private readonly dismissedS = signal<ReadonlyMap<string, number>>(loadDismissed());
  /** The clock, by the minute: failures leave *To fix* a day on. */
  private readonly minute = computed(() => Math.floor(this.clock.now() / 60_000));
  /** What to fix, the blocking first. */
  readonly toFix = computed<FixItem[]>(() =>
    fixItems({
      desktop: this.auth.isDesktop(),
      now: this.minute() * 60_000,
      permissions: this.permissions.permissions(),
      machines: this.machines.view(),
      setup: this.machines.setup(),
      accounts: this.accounts.list(),
      sync: this.sync.pairs(),
      kept: this.copies.kept(),
      jobs: this.jobs.recent(),
      update: this.updater.attentionNeeded() ? this.updater.newVersion() || '' : null,
      dismissed: new Set(this.dismissedS().keys()),
    }),
  );
  readonly toFixCount = computed(() => this.toFix().length);

  readonly alertsEnabled = signal(storageGet(ALERTS_KEY) !== '0');

  /** Machines is on, being set up, or something is paired. */
  private readonly machinesInUse = computed(() => {
    const v = this.machines.view();
    return !!v && (v.enabled || v.setupStep !== null || v.peers.length > 0);
  });

  private readonly seen = new Set<string>();
  private primed = false;
  private audio?: AudioContext;

  constructor() {
    effect(() => {
      const n = this.pendingCount();
      document.title = n > 0 ? `(${n}) ${BASE_TITLE}` : BASE_TITLE;
      const nav = navigator as Navigator & {
        setAppBadge?: (n?: number) => Promise<void>;
        clearAppBadge?: () => Promise<void>;
      };
      try {
        if (n > 0) void nav.setAppBadge?.(n).catch(() => undefined);
        else void nav.clearAppBadge?.().catch(() => undefined);
      } catch {
        /* Badging API unavailable */
      }
    });

    // The devices waiting to pair, once this client is known to be the
    // desktop (`/me` may come after the start).
    effect(() => {
      if (this.started() && this.auth.isDesktop()) untracked(() => this.pairs.start());
    });
    // Code sync's and the copies' state, while Machines is in use.
    effect((onCleanup) => {
      if (!this.started() || !this.machinesInUse()) return;
      const releaseSync = untracked(() => this.sync.watch());
      const releaseCopies = untracked(() => this.copies.watch());
      onCleanup(() => {
        releaseSync();
        releaseCopies();
      });
    });
    // The failing checks behind Machines' health dot, while it shows.
    effect((onCleanup) => {
      if (!this.started() || !this.machines.alarm()) return;
      const release = untracked(() => this.machines.watchSetup());
      untracked(() => void this.machines.loadSetup());
      onCleanup(release);
    });

    effect(() => {
      const items = this.realtime.pendingItems();
      const loaded = this.realtime.sessionsLoaded();
      const fresh = [];
      for (const item of items) {
        if (!this.seen.has(item.id)) {
          this.seen.add(item.id);
          fresh.push(item);
        }
      }
      // The first snapshot is history, not news.
      if (!loaded) return;
      if (!this.primed) {
        this.primed = true;
        return;
      }
      if (!fresh.length) return;
      const first = fresh[0];
      const session = this.realtime.sessions().find((s) => s.id === first.sessionId);
      const what =
        first.kind === 'question'
          ? tr('Question')
          : first.kind === 'plan'
            ? tr('Plan to review')
            : first.kind === 'elicitation'
              ? tr('Input needed')
              : tr('Permission needed');
      const name = session?.title || session?.name || tr('a session');
      const params = { what, name, title: first.title, n: fresh.length - 1 };
      this.notices.announce(
        fresh.length > 1
          ? tr('{what} in {name}: {title} (and {n} more)', params)
          : tr('{what} in {name}: {title}', params),
      );
      if (this.alertsEnabled()) this.alert();
    });

    this.realtime.refetch$.subscribe(() => {
      if (this.started()) void this.jobs.loadRecent();
    });
  }

  /**
   * Follow what waits (the shell, once this device is paired): Machines'
   * summary, the devices waiting to pair (desktop), the accounts' sign-ins
   * and the recent data jobs.
   */
  start(): void {
    if (untracked(this.started)) return;
    this.started.set(true);
    this.machines.start();
    void this.accounts.ensureLoaded();
    void this.jobs.loadRecent();
  }

  /** Put a failed job or ask away, on this device. */
  dismiss(id: string): void {
    const next = new Map(this.dismissedS());
    next.set(id, Date.now());
    this.dismissedS.set(next);
    storageSet(DISMISSED_KEY, JSON.stringify([...next]));
  }

  setAlerts(on: boolean): void {
    this.alertsEnabled.set(on);
    storageSet(ALERTS_KEY, on ? '1' : '0');
  }

  private alert(): void {
    try {
      // Browsers refuse (and log) vibration before the first tap.
      if (navigator.userActivation?.hasBeenActive ?? true) navigator.vibrate?.([80, 60, 80]);
    } catch {
      /* not supported */
    }
    try {
      const Ctx =
        window.AudioContext ??
        (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
      if (!Ctx) return;
      this.audio ??= new Ctx();
      const ctx = this.audio;
      if (ctx.state === 'suspended') void ctx.resume().catch(() => undefined);
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.type = 'sine';
      osc.frequency.setValueAtTime(880, ctx.currentTime);
      osc.frequency.setValueAtTime(1320, ctx.currentTime + 0.09);
      gain.gain.setValueAtTime(0.0001, ctx.currentTime);
      gain.gain.exponentialRampToValueAtTime(0.08, ctx.currentTime + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + 0.25);
      osc.connect(gain).connect(ctx.destination);
      osc.start();
      osc.stop(ctx.currentTime + 0.26);
    } catch {
      /* audio blocked until a user gesture — the title badge still shows */
    }
  }
}

/** The items this device put away lately (id → when). */
function loadDismissed(): ReadonlyMap<string, number> {
  try {
    const raw = JSON.parse(storageGet(DISMISSED_KEY) ?? '[]') as unknown;
    if (!Array.isArray(raw)) return new Map();
    const now = Date.now();
    return new Map(
      raw.filter(
        (e): e is [string, number] =>
          Array.isArray(e) &&
          typeof e[0] === 'string' &&
          typeof e[1] === 'number' &&
          now - e[1] < DISMISSED_FOR_MS,
      ),
    );
  } catch {
    return new Map();
  }
}
