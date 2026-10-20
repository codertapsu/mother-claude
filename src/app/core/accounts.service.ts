import { Injectable, computed, effect, inject, signal, untracked } from '@angular/core';

import {
  AccountChoice,
  DEFAULT_ACCOUNT,
  accountCanRun,
  accountForCwd,
  needsCopy,
} from './accounts';
import { ApiService, describeError } from './api.service';
import {
  AccountDefaults,
  AccountKind,
  AccountPatch,
  AccountView,
  NewAccount,
  PlanUsage,
  PlanWindow,
} from './models';
import { NowService } from './now.service';
import { limitName, limitWindow } from './plan-usage';
import { RealtimeService } from './realtime.service';

/** An account at one of its plan limits right now. */
export interface AccountLimit {
  account: AccountView;
  window: PlanWindow;
  /** "5-hour limit", "weekly limit", … */
  name: string;
}

/**
 * The Claude accounts (docs/ACCOUNTS.md §6) for every view: the registry
 * (`accounts` frames, kept by {@link RealtimeService}, or `GET /accounts`
 * when a view needs it — with only the Default account the server sends no
 * frame, so nothing is fetched until a picker or the Accounts page asks),
 * the accounts at a plan limit and where to continue instead, and the
 * desktop's account administration (every write is desktop-only on the
 * server; the views hide them elsewhere).
 *
 * `multi` is the switch for every account surface: with only the Default
 * registered there are no chips, pickers or switchers anywhere.
 */
@Injectable({ providedIn: 'root' })
export class AccountsService {
  private api = inject(ApiService);
  private realtime = inject(RealtimeService);
  private clock = inject(NowService);

  readonly view = this.realtime.accounts;
  readonly list = computed(() => this.view()?.accounts ?? []);
  /** More than the Default account is registered. */
  readonly multi = computed(() => this.list().length > 1);
  readonly loading = signal(false);
  /** The last failed load, one sentence ('' when fine). */
  readonly loadError = signal('');

  /** Live sessions per account id, from the session store (frames carry no
   * counts; this is always as fresh as the list). */
  readonly liveCounts = computed(() => {
    const out = new Map<string, number>();
    for (const s of this.realtime.sessions()) {
      if (!s.live) continue;
      const id = s.account?.id ?? DEFAULT_ACCOUNT;
      out.set(id, (out.get(id) ?? 0) + 1);
    }
    return out;
  });

  /** Accounts at one of their plan limits now (only while more than one
   * account exists: with one there is nowhere else to go). */
  readonly limits = computed<ReadonlyMap<string, AccountLimit>>(() => {
    const out = new Map<string, AccountLimit>();
    if (!this.multi()) return out;
    const now = this.clock.now();
    for (const account of this.list()) {
      const u = this.usageOf(account.id);
      const window = u?.available ? limitWindow(u.windows, now) : null;
      if (window) out.set(account.id, { account, window, name: limitName(window) });
    }
    return out;
  });

  private inflight?: Promise<void>;
  private again = false;
  private readonly dismissedS = signal<ReadonlySet<string>>(new Set());

  constructor() {
    // A reconnect may hide a change: an `accounts` frame is only sent while
    // more than the Default exists, so a removal down to the Default made
    // while away never arrives. Refetch what was fetched before.
    this.realtime.refetch$.subscribe(() => {
      if (this.view()) void this.refresh();
    });
    // Sessions name an account only while more than the Default exists: a
    // polling connection (no frames) learns of the accounts this way.
    effect(() => {
      if (this.view()) return;
      if (this.realtime.sessions().some((s) => !!s.account)) {
        untracked(() => void this.ensureLoaded());
      }
    });
  }

  byId(id: string | null | undefined): AccountView | undefined {
    return id ? this.list().find((a) => a.id === id) : undefined;
  }

  /** An account's plan usage (the Default's is the original `planUsage`). */
  usageOf(id: string): PlanUsage | null {
    return id === DEFAULT_ACCOUNT
      ? this.realtime.planUsage()
      : (this.realtime.accountUsage().get(id) ?? null);
  }

  limitOf(id: string | null | undefined): AccountLimit | undefined {
    return id ? this.limits().get(id) : undefined;
  }

  /** Where a conversation of `from` can continue now: another account that
   * is signed in, can run and is not at a limit itself — one sharing its
   * directory first (a plain fork), else one with its own directory (the
   * conversation is copied there first). */
  suggestionFor(from: { id: string; kind: AccountKind | string }): AccountView | null {
    const limits = this.limits();
    const free = this.list().filter(
      (a) => a.id !== from.id && a.signedIn && accountCanRun(a) && !limits.has(a.id),
    );
    return free.find((a) => !needsCopy(from, a)) ?? free[0] ?? null;
  }

  /** The desktop allowed copying conversations from `from` to `to` for
   * "continue on" (docs/ACCOUNTS.md §11.2). */
  hasCopyConsent(from: string, to: string): boolean {
    return (this.view()?.copyConsent ?? []).some((c) => c.from === from && c.to === to);
  }

  /** The account a new session in `cwd` gets unless someone picks one. */
  accountFor(cwd: string): AccountChoice {
    return accountForCwd(this.view(), cwd);
  }

  /** Fetch the accounts once when nothing brought them yet. */
  ensureLoaded(): Promise<void> {
    return this.view() ? Promise.resolve() : this.refresh();
  }

  /** `GET /accounts` now. A request made while one is in flight runs again
   * after it (its answer may predate a change), and every caller waits for
   * the last answer. */
  refresh(): Promise<void> {
    if (this.inflight) {
      this.again = true;
      return this.inflight;
    }
    const run = async () => {
      do {
        this.again = false;
        await this.fetchOnce();
      } while (this.again);
      // Cleared in the same step as the last check: a later call starts over.
      this.inflight = undefined;
    };
    this.inflight = run();
    return this.inflight;
  }

  private async fetchOnce(): Promise<void> {
    this.loading.set(true);
    const mark = this.realtime.accountsMark();
    try {
      this.realtime.mergeAccounts(await this.api.getAccounts(), mark);
      this.loadError.set('');
    } catch (e) {
      this.loadError.set(describeError(e).text);
    } finally {
      this.loading.set(false);
    }
  }

  // ─────────────────────────────── Administration (desktop only) ──────
  // Each one throws the server's refusal for the caller to show, and
  // refreshes the list after a change.

  async add(body: NewAccount): Promise<AccountView> {
    const account = await this.api.addAccount(body);
    await this.refresh();
    return account;
  }

  async update(id: string, patch: AccountPatch): Promise<void> {
    await this.api.updateAccount(id, patch);
    await this.refresh();
  }

  async remove(id: string): Promise<void> {
    await this.api.removeAccount(id);
    await this.refresh();
  }

  async setDefaults(defaults: AccountDefaults): Promise<void> {
    await this.api.setAccountDefaults(defaults);
    await this.refresh();
  }

  async setCopyConsent(from: string, to: string, granted: boolean): Promise<void> {
    await this.api.setCopyConsent(from, to, granted);
    await this.refresh();
  }

  async recheck(id: string): Promise<void> {
    await this.api.recheckAccount(id);
    await this.refresh();
  }

  async login(id: string, consoleAccount = false): Promise<void> {
    await this.api.loginAccount(id, consoleAccount);
    await this.refresh();
  }

  /** The hooks plugin in a directory account's own directory. */
  async setHooksPlugin(id: string, install: boolean): Promise<void> {
    await (install ? this.api.installAccountHooks(id) : this.api.uninstallAccountHooks(id));
    await this.refresh();
  }

  // ─────────────────────────────────────────────── Limit offers ──────

  /** The offer was waved away for this window (until it resets). */
  isDismissed(limit: AccountLimit): boolean {
    return this.dismissedS().has(offerKey(limit));
  }

  dismiss(limit: AccountLimit): void {
    this.dismissedS.update((prev) => new Set(prev).add(offerKey(limit)));
  }
}

function offerKey(l: AccountLimit): string {
  return `${l.account.id}|${l.window.key}|${l.window.resetsAt ?? ''}`;
}
