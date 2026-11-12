import {
  Component,
  DestroyRef,
  ElementRef,
  computed,
  inject,
  signal,
  viewChildren,
} from '@angular/core';
import { RouterLink } from '@angular/router';

import { AccountsService } from '../../core/accounts.service';
import { ApiService } from '../../core/api.service';
import { AttentionService } from '../../core/attention.service';
import { AuthService } from '../../core/auth.service';
import { DataJobsService } from '../../core/data-jobs.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { MachineScopes } from '../../core/machine-scope';
import { MachinesService } from '../../core/machines.service';
import { PendingPair, PendingRef, SessionSummary } from '../../core/models';
import { NoticeService } from '../../core/notice.service';
import { PairRequestsService } from '../../core/pair-requests.service';
import { PendingService } from '../../core/pending.service';
import { RealtimeService } from '../../core/realtime.service';
import {
  MachineTag,
  RemotePendingRef,
  RemoteSessionsService,
} from '../../core/remote-sessions.service';
import { AttentionCardComponent } from '../../shared/attention-card.component';
import { EmptyStateComponent } from '../../shared/empty-state.component';
import { IconComponent } from '../../shared/icon.component';
import { Lingering, lingerList } from '../../shared/linger';
import { PageHeadComponent } from '../../shared/page-head.component';
import { PairApprovalDialogComponent } from '../../shared/pair-approval-dialog.component';
import { PendingCardComponent } from '../../shared/pending-card.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';
import { AsksCardComponent } from '../machines/asks-card.component';
import { IntroSheetComponent } from '../machines/intro-sheet.component';
import { thisComputerTitle } from '../machines/machines-text';
import { FixItemComponent } from './fix-item.component';
import { IntroItemComponent } from './intro-item.component';
import { OfferItemComponent } from './offer-item.component';

/** The Computer filter's value for this computer. */
const HERE = 'here';

/** The network a device asked from (English: translated where shown). */
const INGRESS: Record<string, string> = {
  loopback: 'this computer',
  lan: 'local network',
  tunnel: 'internet',
};

interface Entry {
  ref: PendingRef;
  gone: boolean;
  session?: SessionSummary;
  /** Another computer's request: that computer. */
  machine?: MachineTag;
}

/** A card's identity: requests of two computers never mix. */
function entryKey(e: Entry): string {
  return e.machine ? `${e.machine.id}\u0000${e.ref.id}` : e.ref.id;
}

/**
 * Everything that waits for you (docs/UI-REDESIGN.md §6.4), in three
 * sections:
 *
 * - **Needs you** — what can be answered here, oldest first, with how long
 *   it has waited: every session's requests (permission prompts, questions,
 *   plans, browser actions), other computers' online ones answered through
 *   them; on the desktop the devices waiting to pair (*Approve…* opens the
 *   typed-number dialog), what your other computers asked this one, the
 *   computers one of them introduces (*Pair with …?*) and a setting
 *   another computer suggests.
 * - **To fix** — what stopped working or works worse, each with its fix
 *   (`mc-inbox-fix`, core/attention.ts).
 * - **Waiting where it runs** — requests only answerable where the session
 *   runs, with what would let you answer them here.
 *
 * Full requests arrive through `subInbox` (and `GET /inbox` on first load).
 * After an answer the next card takes focus; on a keyboard J/K move between
 * requests and A/D/F answer the focused one. While more than the Default
 * account exists, each request names its account and an *Account* filter
 * shows one account's items; while another computer is paired, a *Computer*
 * filter shows one computer's. A phone sees what only the desktop answers,
 * with where it is done; the badge counts what can be answered here.
 */
@Component({
  selector: 'mc-inbox',
  imports: [
    RouterLink,
    AsksCardComponent,
    AttentionCardComponent,
    EmptyStateComponent,
    FixItemComponent,
    IconComponent,
    IntroItemComponent,
    IntroSheetComponent,
    OfferItemComponent,
    PageHeadComponent,
    PairApprovalDialogComponent,
    PendingCardComponent,
    TimeAgoComponent,
    TPipe,
  ],
  template: `
    <div class="page narrow">
      <mc-page-head
        [heading]="'Inbox' | t"
        [lede]="
          'Everything that waits for you: requests from every session and computer, and what to fix.'
            | t
        "
        help="approvals"
      >
        @if (accountFilterShown()) {
          <select class="filter" [attr.aria-label]="'Account' | t" (change)="pickAccount($event)">
            <option value="" [selected]="!account()">{{ 'All accounts' | t }}</option>
            @for (a of accounts.list(); track a.id) {
              <option [value]="a.id" [selected]="a.id === account()">{{ a.name }}</option>
            }
          </select>
        }
        @if (computerFilterShown()) {
          <select class="filter" [attr.aria-label]="'Computer' | t" (change)="pickComputer($event)">
            <option value="" [selected]="!computer()">{{ 'All computers' | t }}</option>
            <option [value]="here" [selected]="computer() === here">{{ thisTitle() }}</option>
            @for (p of peers(); track p.id) {
              <option [value]="p.id" [selected]="p.id === computer()">{{ p.name }}</option>
            }
          </select>
        }
      </mc-page-head>

      @if (!ready()) {
        <div class="skeleton" [attr.aria-label]="'Loading' | t"><span></span><span></span></div>
      } @else if (!needsCount() && !fixes().length && !desk().length) {
        @if (hidden()) {
          <mc-empty-state [heading]="hiddenHeading()" [text]="hiddenText()">
            <button class="btn small" type="button" (click)="showAll()">
              {{ 'Show everything' | t }}
            </button>
          </mc-empty-state>
        } @else {
          <mc-empty-state
            icon="check"
            tone="green"
            [heading]="'Nothing needs you' | t"
            [text]="
              'Permission prompts, questions and plans from every session show up here, and anything to fix.'
                | t
            "
          >
            @if (foreignHint()) {
              <p class="muted small hint">
                {{
                  'Approvals for VS Code and terminal sessions appear here once foreign-session control is installed'
                    | t
                }}
                @if (isDesktop()) {
                  (<a routerLink="/remote" fragment="foreign">{{
                    'Settings › Sessions started elsewhere' | t
                  }}</a
                  >).
                } @else {
                  {{
                    '(on the {computer}: Settings › Sessions started elsewhere).'
                      | t: { computer: computerName() }
                  }}
                }
              </p>
            }
          </mc-empty-state>
        }
      }

      @if (needsCount()) {
        <section aria-labelledby="inbox-needs">
          <h2 id="inbox-needs">
            {{ 'Needs you' | t }} <span class="count-badge">{{ needsCount() }}</span>
          </h2>
          @if (needs().length) {
            <p class="muted small keys">
              {{
                'Tip: on a keyboard, J/K move between requests; A allows, D denies, F adds feedback.'
                  | t
              }}
            </p>
          }
          <div class="stack">
            @for (p of pairsShown(); track p.id) {
              <mc-attention-card
                kind="pair"
                tone="info"
                [heading]="'{name} wants to pair' | t: { name: pairName(p) }"
                [meta]="pairMeta(p)"
                [why]="pairWhy(p)"
                [when]="p.createdAt"
              >
                <button
                  class="btn small primary"
                  type="button"
                  [disabled]="!!p.contested || pairBusy() === p.id"
                  (click)="approving.set(p)"
                >
                  {{ 'Approve…' | t }}
                </button>
                <button
                  class="btn small danger"
                  type="button"
                  [disabled]="pairBusy() === p.id"
                  (click)="denyPair(p)"
                >
                  {{ 'Deny' | t }}
                </button>
              </mc-attention-card>
            }
            @for (e of needs(); track entryKey(e); let i = $index) {
              <div class="entry">
                <p class="waited">
                  <mc-icon name="clock" [size]="12" />
                  <mc-time [ms]="e.ref.createdAt" mode="elapsed" [prefix]="('waiting' | t) + ' '" />
                </p>
                <mc-pending-card
                  [item]="e.ref"
                  [session]="e.session"
                  [showSession]="true"
                  [gone]="e.gone"
                  [machine]="e.machine?.id ?? null"
                  bodyMax="min(55vh, 26rem)"
                  (resolved)="focusCard(i + 1)"
                  (move)="focusCard(i + $event)"
                />
              </div>
            }
            @if (asks().length) {
              <mc-machine-asks
                [asks]="asks()"
                [readonly]="!isDesktop()"
                (answered)="askAnswered()"
              />
            }
            @for (i of intros(); track i.id) {
              <mc-inbox-intro
                [intro]="i"
                (answered)="machines.apply($event)"
                (opened)="introOpen.set(true)"
              />
            }
            @if (offer(); as o) {
              <mc-inbox-offer
                [offer]="o"
                [settings]="machineSettings()!"
                (answered)="machines.apply($event)"
              />
            }
          </div>
        </section>
      }

      @if (fixes().length) {
        <section aria-labelledby="inbox-fix">
          <h2 id="inbox-fix">
            {{ 'To fix' | t }} <span class="count-badge fix-badge">{{ fixes().length }}</span>
          </h2>
          <div class="stack">
            @for (f of fixes(); track f.id) {
              <mc-inbox-fix [item]="f" />
            }
          </div>
        </section>
      }

      @if (desk().length) {
        <section aria-labelledby="inbox-desk">
          <h2 id="inbox-desk">
            {{ 'Waiting where it runs' | t }}
            <span class="count-badge muted-badge">{{ desk().length }}</span>
          </h2>
          <p class="muted small">
            {{
              'These can only be answered where the session runs. What would let you answer them here is under each one.'
                | t
            }}
          </p>
          <div class="stack">
            @for (e of desk(); track entryKey(e)) {
              <mc-pending-card
                [item]="e.ref"
                [session]="e.session"
                [showSession]="true"
                [gone]="e.gone"
                [machine]="e.machine?.id ?? null"
              />
            }
          </div>
        </section>
      }
    </div>

    @if (isDesktop()) {
      <mc-pair-approval-dialog
        [pending]="approving()"
        (closed)="approving.set(null)"
        (settled)="pairService.settled($event.pending.id)"
      />
      <mc-intro-sheet [open]="introOpen()" (closed)="introOpen.set(false)" />
    }
  `,
  styles: [
    `
      h2 {
        display: flex;
        align-items: center;
        gap: 0.4rem;
      }
      h2 .count-badge {
        font-size: 0.7rem;
      }
      .muted-badge {
        background: var(--panel-3);
        color: var(--muted);
      }
      .fix-badge {
        background: var(--amber-soft);
        color: var(--amber);
      }
      section {
        margin-top: 0.5rem;
      }
      .keys {
        margin: -0.25rem 0 0.6rem;
      }
      @media (pointer: coarse) {
        .keys {
          display: none;
        }
      }
      .entry {
        display: flex;
        flex-direction: column;
        gap: 0.2rem;
      }
      .waited {
        display: flex;
        align-items: center;
        gap: 0.3rem;
        margin: 0;
        font-size: 0.75rem;
        color: var(--amber);
      }
      .hint {
        margin: 0 auto;
        max-width: 26rem;
      }
      .filter {
        width: auto;
        flex: 0 1 12rem;
        min-width: 8rem;
      }
      .skeleton {
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
      }
      .skeleton span {
        height: 8rem;
        border-radius: var(--radius);
        background: var(--panel-2);
        animation: mc-shimmer 1.2s ease-in-out infinite alternate;
      }
    `,
  ],
})
export class InboxComponent {
  private realtime = inject(RealtimeService);
  private pendings = inject(PendingService);
  private notices = inject(NoticeService);
  private auth = inject(AuthService);
  private remote = inject(RemoteSessionsService);
  private scopes = inject(MachineScopes);
  private api = inject(ApiService);
  protected readonly machines = inject(MachinesService);
  private jobs = inject(DataJobsService);
  private attention = inject(AttentionService);
  protected readonly pairService = inject(PairRequestsService);
  protected readonly accounts = inject(AccountsService);

  private readonly cards = viewChildren(PendingCardComponent, { read: ElementRef });
  private readonly fetched = signal(false);
  /** `GET /inbox`, used until the live store has its first snapshot. */
  private readonly fetchedItems = signal<PendingRef[]>([]);
  protected readonly isDesktop = this.auth.isDesktop;
  protected readonly entryKey = entryKey;
  protected readonly here = HERE;
  protected readonly thisTitle = thisComputerTitle;

  protected readonly ready = computed(() => this.realtime.sessionsLoaded() || this.fetched());
  private readonly lingering = lingerList<PendingRef>(() =>
    this.realtime.sessionsLoaded() ? this.realtime.pendingItems() : this.fetchedItems(),
  );
  /** Other computers' requests (none while no computer is paired). */
  private readonly remoteLingering = lingerList<RemotePendingRef>(() => this.remote.pending());
  private readonly all = computed<Entry[]>(() => {
    const byId = new Map(this.realtime.sessions().map((s) => [s.id, s]));
    const machines = this.remote.byId();
    return [
      ...this.lingering().map((l: Lingering<PendingRef>) => ({
        ref: l.item,
        gone: l.gone,
        session: byId.get(l.item.sessionId),
      })),
      ...this.remoteLingering().map((l: Lingering<RemotePendingRef>) => {
        const m = machines.get(l.item.machine);
        return {
          ref: l.item,
          gone: l.gone,
          session: m?.sessions.find((s) => s.id === l.item.sessionId),
          machine: m,
        };
      }),
    ].sort((a, b) => a.ref.createdAt - b.ref.createdAt);
  });

  // ─────────────────────────────────────────────────────────── Filters

  /** The account filter: only while more than the Default exists. */
  protected readonly accountFilterShown = this.accounts.multi;
  /** The account picked ('' = every account). */
  protected readonly account = signal('');
  /** The pick in effect: none once its account is gone (or the only one). */
  private readonly accountInEffect = computed(() => {
    const id = this.account();
    return this.accounts.multi() && this.accounts.byId(id) ? id : '';
  });
  /** The paired computers (the Computer filter: only while one is). */
  protected readonly peers = computed(() => this.machines.view()?.peers ?? []);
  protected readonly computerFilterShown = computed(() => this.peers().length > 0);
  /** The computer picked ('' = every one, `here` = this one). */
  protected readonly computer = signal('');
  private readonly computerInEffect = computed(() => {
    const id = this.computer();
    if (!id || !this.computerFilterShown()) return '';
    return id === HERE || this.peers().some((p) => p.id === id) ? id : '';
  });
  /** An item passes the filters: its account (none: only with no account
   * picked) and its computer (null: this one). */
  private passes(account: string | undefined, machine: string | null | undefined): boolean {
    const a = this.accountInEffect();
    const m = this.computerInEffect();
    if (a && account !== a) return false;
    return !m || (machine ?? HERE) === m;
  }

  /** Another computer's requests are none of this computer's accounts:
   * an account picked leaves them out. */
  private readonly entries = computed<Entry[]>(() =>
    this.all().filter((e) =>
      this.passes(e.machine ? undefined : e.session?.account?.id, e.machine?.id),
    ),
  );
  protected readonly needs = computed(() => this.entries().filter((e) => e.ref.answerable));
  protected readonly desk = computed(() => this.entries().filter((e) => !e.ref.answerable));
  /** Devices waiting to pair (the desktop's). */
  protected readonly pairsShown = computed(() =>
    this.attention.pairRequests().filter(() => this.passes(undefined, null)),
  );
  /** What other computers asked this one, not done yet. */
  protected readonly asks = computed(() =>
    this.attention.asks().filter((a) => this.passes(undefined, a.from)),
  );
  /** Introductions to another of your computers, not answered yet. */
  protected readonly intros = computed(() =>
    this.attention.intros().filter((i) => this.passes(undefined, i.from)),
  );
  /** A setting another computer suggests here. */
  protected readonly offer = computed(() => {
    const o = this.attention.offer();
    return o && this.machineSettings() && this.passes(undefined, o.from) ? o : null;
  });
  protected readonly machineSettings = computed(() => this.machines.view()?.settings ?? null);
  protected readonly fixes = computed(() =>
    this.attention.toFix().filter((f) => this.passes(f.account, f.machine)),
  );
  protected readonly needsCount = computed(
    () =>
      this.needs().length +
      this.pairsShown().length +
      this.asks().length +
      this.intros().length +
      (this.offer() ? 1 : 0),
  );

  /** Items the filters hide. */
  protected readonly hidden = computed(() => {
    const total =
      this.all().length +
      this.attention.pairRequests().length +
      this.attention.asks().length +
      this.attention.intros().length +
      (this.attention.offer() && this.machineSettings() ? 1 : 0) +
      this.attention.toFix().length;
    const shown =
      this.needs().length +
      this.desk().length +
      this.pairsShown().length +
      this.asks().length +
      this.intros().length +
      (this.offer() ? 1 : 0) +
      this.fixes().length;
    return total - shown;
  });
  protected readonly hiddenHeading = computed(() => {
    const account = this.accounts.byId(this.accountInEffect())?.name ?? '';
    const m = this.computerInEffect();
    const computer = computerNoun();
    if (m === HERE) {
      return account
        ? tr('Nothing from {account} on this {computer} needs you', { account, computer })
        : tr('Nothing on this {computer} needs you', { computer });
    }
    const name = this.peers().find((p) => p.id === m)?.name ?? '';
    if (name) {
      return account
        ? tr('Nothing from {account} on {computer} needs you', { account, computer: name })
        : tr('Nothing on {name} needs you', { name });
    }
    return tr('Nothing from {name} needs you', { name: account });
  });
  protected readonly hiddenText = computed(() => {
    const n = this.hidden();
    return n === 1
      ? tr('1 more item waits outside this filter.')
      : tr('{n} more items wait outside this filter.', { n });
  });

  /** The desktop's computer in words ("Mac", "PC", "computer"). */
  protected readonly computerName = computerNoun;
  /** A live foreign session nobody can answer from here: suggest the plugin. */
  protected readonly foreignHint = computed(() =>
    this.realtime
      .sessions()
      .some((s) => s.live && !s.owned && s.control?.channel === 'none' && !s.control.hooksActive),
  );

  // ─────────────────────────────────────────────────────────── Pairing

  /** The pairing an accepted introduction runs. */
  protected readonly introOpen = signal(false);
  /** The request whose typed-number dialog is open. */
  protected readonly approving = signal<PendingPair | null>(null);
  protected readonly pairBusy = signal<string | null>(null);

  constructor() {
    const release = this.realtime.subscribeInbox();
    const sub = this.realtime.refetch$.subscribe(() => void this.load());
    inject(DestroyRef).onDestroy(() => {
      release();
      sub.unsubscribe();
    });
    void this.load();
    // What failed lately, and the accounts' sign-ins, as they are now.
    void this.jobs.loadRecent();
    void this.accounts.ensureLoaded();
  }

  protected pickAccount(e: Event): void {
    this.account.set((e.target as HTMLSelectElement).value);
  }

  protected pickComputer(e: Event): void {
    this.computer.set((e.target as HTMLSelectElement).value);
  }

  protected showAll(): void {
    this.account.set('');
    this.computer.set('');
  }

  protected pairName(p: PendingPair): string {
    return p.deviceName || p.name || tr('A device');
  }

  /** What approving takes, or why it can't be approved. */
  protected pairWhy(p: PendingPair): string {
    return p.contested
      ? tr(
          "This code was used by two devices, so this request can't be approved. Deny it and create a new code.",
        )
      : tr("The device shows a 4-digit number: you'll type it to approve.");
  }

  /** "iPhone · via local network". */
  protected pairMeta(p: PendingPair): string {
    const parts = [p.platform || tr('unknown platform')];
    const net = INGRESS[p.ingress ?? ''];
    if (net) parts.push(tr('via {network}', { network: tr(net) }));
    return parts.join(' · ');
  }

  protected async denyPair(p: PendingPair): Promise<void> {
    this.pairBusy.set(p.id);
    try {
      await this.api.denyPair(p.id);
      this.pairService.settled(p.id);
    } catch (e) {
      this.notices.error(e);
    } finally {
      this.pairBusy.set(null);
    }
  }

  /** An ask was answered: the view (and the checks it fixed) change. */
  protected askAnswered(): void {
    void this.machines.load();
    void this.machines.loadSetup();
  }

  protected focusCard(i: number): void {
    const list = this.cards();
    const target = list[Math.max(0, Math.min(list.length - 1, i))];
    queueMicrotask(() => (target?.nativeElement as HTMLElement | undefined)?.focus());
  }

  private async load(): Promise<void> {
    // Other computers' full requests, through each one's scope.
    for (const m of this.remote.list()) {
      if (m.online)
        void this.scopes
          .pending(m.id)
          .loadInbox()
          .catch(() => undefined);
    }
    try {
      this.fetchedItems.set(await this.pendings.loadInbox());
    } catch (e) {
      if (!this.realtime.sessionsLoaded()) {
        this.notices.error(e, () => void this.load(), tr('Inbox'));
      }
    } finally {
      this.fetched.set(true);
    }
  }
}
