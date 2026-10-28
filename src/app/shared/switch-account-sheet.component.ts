import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
} from '@angular/core';
import { RouterLink } from '@angular/router';

import { DEFAULT_ACCOUNT, isAccountColor, needsCopy } from '../core/accounts';
import { AccountsService } from '../core/accounts.service';
import { ApiError, ApiService, describeError } from '../core/api.service';
import { AccountRef, AccountView, CopyConsent, SessionSummary } from '../core/models';
import { NoticeService } from '../core/notice.service';
import { NowService } from '../core/now.service';
import { SessionActionsService } from '../core/session-actions.service';
import { AccountChipComponent } from './account-chip.component';
import { cannotRunReason, consentNeeded, signInLine, usageLine } from './accounts';
import { IconComponent } from './icon.component';
import { SheetComponent } from './sheet.component';
import {
  SwitchMode,
  SwitchWhen,
  elsewhereText,
  normalizeSwitchReply,
  runningTasks,
  isBridgeThread,
  switchBusy,
  switchErrorText,
  switchMode,
  switchedText,
  switchingToOf,
} from './switch-account';
import { capitalize, sessionName, statusLabel } from './util';

interface Target {
  account: AccountView;
  color: string;
  signIn: string;
  usage: string;
  limited: boolean;
  /** Why it can't be picked ('' when it can). */
  blocked: string;
  /** What picking it means beyond the switch ('' for nothing). */
  note: string;
}

/** The question before the first move or copy between two accounts. */
interface ConsentAsk extends CopyConsent {
  fromName: string;
  toName: string;
}

/**
 * "Switch account" (docs/ACCOUNTS.md §15.1). A session Mother Claude runs
 * goes on under the account picked — the same session, the same page;
 * busy with a turn, it switches once the turn ends (the default, and the
 * switch shows until then, cancellable) or now, interrupting it; the
 * background tasks it started stop with it, and it says so first. A
 * session that doesn't run switches for its next continue. Another
 * directory moves the conversation there first, once the pair's copies are
 * allowed (asked once, as Continue on asks). A session another app runs (a
 * terminal, VS Code…) can't change account while it runs there: the sheet
 * says so and offers *Continue on* the account instead — a new session from
 * where it stands, the original running on. From a sessions list, the
 * conversation is picked first. The desktop and paired phones alike.
 */
@Component({
  selector: 'mc-switch-account-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, AccountChipComponent, IconComponent, SheetComponent],
  template: `
    <mc-sheet [open]="open()" title="Switch account" (closed)="closed.emit()">
      @if (open()) {
        @if (sessions().length > 1) {
          <h3>Conversation</h3>
          <div class="opts" role="radiogroup" aria-label="Conversation">
            @for (s of sessions(); track s.id) {
              <button
                type="button"
                role="radio"
                class="opt"
                [class.on]="s.id === session()?.id"
                [attr.aria-checked]="s.id === session()?.id"
                [disabled]="busy()"
                (click)="pickSession(s.id)"
              >
                <span class="ot">
                  <strong class="oname">{{ name(s) }}</strong>
                  <span class="muted small">{{ s.projectName }} · {{ status(s) }}</span>
                </span>
              </button>
            }
          </div>
        }

        @if (session(); as s) {
          <p class="lede small">
            @if (source(); as src) {
              Now on <mc-account-chip [account]="src" />.
            }
            @switch (mode()) {
              @case ('here') {
                The same session goes on under the account you pick: the same conversation, on this
                same page.
              }
              @case ('later') {
                It isn't running: its next continue runs under the account you pick.
              }
              @default {
                {{ elsewhere() }} Continue on another account instead: a new session from where it
                stands, while this one keeps running there.
              }
            }
          </p>

          @if (pendingName(); as to) {
            <div class="notice small pending" role="status">
              <p>
                <span class="spin" aria-hidden="true"></span>
                Switching to <strong>{{ to }}</strong> once this turn ends.
              </p>
              <button
                class="btn ghost small"
                type="button"
                [disabled]="busy()"
                (click)="cancelPending()"
              >
                Cancel the switch
              </button>
            </div>
          } @else {
            <h3>Account</h3>
            <div class="opts" role="radiogroup" aria-label="Account">
              @for (t of targets(); track t.account.id) {
                <button
                  type="button"
                  role="radio"
                  class="opt"
                  [class.on]="t.account.id === targetId()"
                  [attr.aria-checked]="t.account.id === targetId()"
                  [disabled]="!!t.blocked || busy()"
                  (click)="pickTarget(t.account.id)"
                >
                  <span class="adot" aria-hidden="true" [style.background]="t.color"></span>
                  <span class="ot">
                    <strong class="oname">{{ t.account.name }}</strong>
                    <span class="muted small">{{ t.signIn }}</span>
                    @if (t.usage) {
                      <span class="small" [class.limit]="t.limited">{{ t.usage }}</span>
                    }
                    @if (t.blocked) {
                      <span class="small why">{{ t.blocked }}</span>
                    } @else if (t.note) {
                      <span class="muted small">{{ t.note }}</span>
                    }
                  </span>
                  @if (t.account.id === targetId()) {
                    <mc-icon name="check" [size]="16" />
                  }
                </button>
              } @empty {
                <p class="muted small">
                  {{ accounts.loading() ? 'Loading the accounts…' : 'There is no other account.' }}
                </p>
              }
            </div>

            @if (bridge()) {
              <p class="muted small between">
                {{
                  busyTurn()
                    ? 'This conversation of the API is in the middle of a turn: its account changes between turns. Try again once the turn is over.'
                    : 'A conversation of the API changes account between turns.'
                }}
              </p>
            } @else if (mode() === 'here' && busyTurn()) {
              <h3>When</h3>
              <div class="whens" role="radiogroup" aria-label="When">
                <label class="when">
                  <input
                    type="radio"
                    name="when"
                    [checked]="when() === 'idle'"
                    [disabled]="busy()"
                    (change)="when.set('idle')"
                  />
                  <span>After this turn <span class="muted small">— it finishes first</span></span>
                </label>
                <label class="when">
                  <input
                    type="radio"
                    name="when"
                    [checked]="when() === 'now'"
                    [disabled]="busy()"
                    (change)="when.set('now')"
                  />
                  <span>Now <span class="muted small">— interrupts the turn</span></span>
                </label>
              </div>
            }
            @if (mode() === 'here' && taskCount() > 0) {
              <p class="notice warn small tasks" role="status">
                {{ tasksText() }}
              </p>
            }

            @if (error()) {
              <p class="notice error small" role="alert">{{ error() }}</p>
            }

            @if (ask(); as a) {
              <div class="notice warn small ask" role="group" aria-label="Allow copies">
                <p>
                  {{ mode() === 'elsewhere' ? 'Copy' : 'Move' }} conversations from
                  <strong>{{ a.fromName }}</strong> to <strong>{{ a.toName }}</strong>
                  {{
                    mode() === 'elsewhere'
                      ? 'when you continue them there'
                      : 'when you switch them there'
                  }}? Asked once; you can revoke it on the Accounts page.
                </p>
                <div class="row">
                  <button
                    class="btn primary small"
                    type="button"
                    [disabled]="busy()"
                    (click)="allow()"
                  >
                    {{ busy() ? 'Working…' : 'Allow and go on' }}
                  </button>
                  <button
                    class="btn ghost small"
                    type="button"
                    [disabled]="busy()"
                    (click)="ask.set(null)"
                  >
                    Not now
                  </button>
                </div>
              </div>
            } @else {
              <div class="row actions">
                <button
                  class="btn primary"
                  type="button"
                  [disabled]="!target() || busy() || (bridge() && busyTurn())"
                  (click)="go()"
                >
                  <mc-icon [name]="mode() === 'elsewhere' ? 'play' : 'user'" [size]="14" />
                  {{ actionLabel() }}
                </button>
              </div>
            }
          }
          <p class="muted small manage">
            <a routerLink="/accounts" (click)="closed.emit()">Manage accounts</a>
          </p>
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
      h3:first-child {
        margin-top: 0;
      }
      .lede {
        margin: 0 0 0.25rem;
        color: var(--muted);
        line-height: 1.6;
      }
      .opts {
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
      }
      .opt {
        appearance: none;
        display: flex;
        align-items: center;
        gap: 0.65rem;
        width: 100%;
        min-height: var(--tap);
        padding: 0.5rem 0.7rem;
        text-align: left;
        font: inherit;
        color: var(--text);
        background: var(--panel-2);
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        cursor: pointer;
      }
      .opt.on {
        border-color: var(--accent);
        background: var(--accent-soft);
      }
      .opt:disabled {
        cursor: not-allowed;
        opacity: 0.7;
      }
      .opt mc-icon {
        color: var(--accent-text);
      }
      .adot {
        flex: 0 0 auto;
        width: 0.7rem;
        height: 0.7rem;
        border-radius: 50%;
      }
      .ot {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.05rem;
      }
      .oname {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .ot .small {
        overflow-wrap: anywhere;
      }
      .limit {
        color: var(--red);
      }
      .why {
        color: var(--amber);
      }
      .between {
        margin: 0.75rem 0 0;
      }
      .whens {
        display: flex;
        flex-direction: column;
        gap: 0.35rem;
      }
      .when {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        min-height: var(--tap);
        cursor: pointer;
      }
      .notice {
        margin: 0.75rem 0 0;
      }
      .pending {
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        gap: 0.45rem;
      }
      .pending p,
      .ask p {
        margin: 0;
        line-height: 1.5;
      }
      .ask p {
        margin-bottom: 0.6rem;
      }
      .spin {
        display: inline-block;
        width: 0.8rem;
        height: 0.8rem;
        margin-right: 0.3rem;
        border: 2px solid var(--border-strong);
        border-top-color: var(--accent);
        border-radius: 50%;
        animation: mc-spin 0.8s linear infinite;
        vertical-align: -0.1rem;
      }
      .actions {
        margin-top: 0.9rem;
      }
      .manage {
        margin: 0.75rem 0 0;
      }
    `,
  ],
})
export class SwitchAccountSheetComponent {
  protected readonly accounts = inject(AccountsService);
  private api = inject(ApiService);
  private actions = inject(SessionActionsService);
  private notices = inject(NoticeService);
  private clock = inject(NowService);

  readonly open = input(false);
  /** The conversation(s) to choose from: one from a session's menu, those
   * of an account at its limit from the sessions list. */
  readonly sessions = input<SessionSummary[]>([]);
  /** The account to preselect when it can take the conversation. */
  readonly suggested = input<string | null>(null);
  /** Background tasks the session runs, when the host read them (else
   * the summary's count). */
  readonly tasks = input<number | null>(null);
  readonly closed = output<void>();

  protected readonly busy = signal(false);
  protected readonly error = signal('');
  protected readonly when = signal<SwitchWhen>('idle');
  /** Asked to allow moves or copies between two accounts. */
  protected readonly ask = signal<ConsentAsk | null>(null);
  private readonly pickedSession = signal<string | null>(null);
  private readonly pickedTarget = signal<string | null>(null);
  /** A switch this sheet just asked for, until the summary says so. */
  private readonly askedPending = signal<string | null>(null);
  /** The server said another app runs it, whatever the summary said. */
  private readonly runsElsewhere = signal(false);

  protected readonly session = computed<SessionSummary | undefined>(() => {
    const list = this.sessions();
    return list.find((s) => s.id === this.pickedSession()) ?? list[0];
  });
  protected readonly mode = computed<SwitchMode>(() => {
    const s = this.session();
    if (!s) return 'here';
    return this.runsElsewhere() ? 'elsewhere' : switchMode(s);
  });
  /** A conversation of the API: between turns only. */
  protected readonly bridge = computed(
    () => this.mode() === 'here' && isBridgeThread(this.session()),
  );
  protected readonly busyTurn = computed(() => {
    const s = this.session();
    return !!s && switchBusy(s);
  });
  protected readonly elsewhere = computed(() => {
    const s = this.session();
    return s ? elsewhereText(s) : '';
  });
  protected readonly taskCount = computed(() => {
    const s = this.session();
    if (!s) return 0;
    return this.tasks() ?? runningTasks(s);
  });
  protected readonly tasksText = computed(() => {
    const n = this.taskCount();
    return n === 1
      ? 'The background task it started stops when it switches.'
      : `The ${n} background tasks it started stop when it switches.`;
  });
  /** Where the conversation is now (the Default when it names none). */
  protected readonly source = computed<AccountRef | undefined>(() => this.session()?.account);
  /** The account it switches to once its turn ends. */
  protected readonly pendingName = computed(() => {
    const id = this.askedPending() ?? switchingToOf(this.session());
    if (!id) return '';
    return this.accounts.byId(id)?.name ?? 'another account';
  });
  protected readonly targets = computed<Target[]>(() => {
    const from: Pick<AccountRef, 'id' | 'kind'> = this.source() ?? {
      id: DEFAULT_ACCOUNT,
      kind: 'directory',
    };
    const continuing = this.mode() === 'elsewhere';
    const now = this.clock.now();
    return this.accounts
      .list()
      .filter((a) => a.id !== from.id)
      .map((a) => {
        const reason = cannotRunReason(a);
        let note = '';
        if (needsCopy(from, a)) {
          const where =
            a.kind === 'token' ? "Uses the Default's directory" : 'Keeps its own directory';
          const how = continuing ? 'copied there first' : 'moved there first';
          note = this.accounts.hasCopyConsent(from.id, a.id)
            ? `${where}: the conversation is ${how}.`
            : `${where}: the conversation is ${how} (asked once).`;
        }
        return {
          account: a,
          color: isAccountColor(a.color) ? a.color : 'var(--accent)',
          signIn: signInLine(a).text,
          usage: usageLine(this.accounts.usageOf(a.id), now),
          limited: !!this.accounts.limitOf(a.id),
          blocked: reason ? `${capitalize(reason)}.` : '',
          note,
        };
      });
  });
  /** The pick, else the suggestion, else the first account that can take it. */
  protected readonly targetId = computed(() => {
    const free = this.targets().filter((t) => !t.blocked);
    const has = (id: string | null) => !!id && free.some((t) => t.account.id === id);
    const picked = this.pickedTarget();
    if (has(picked)) return picked as string;
    const suggested = this.suggested();
    if (has(suggested)) return suggested as string;
    return free[0]?.account.id ?? '';
  });
  protected readonly target = computed(() => this.accounts.byId(this.targetId()));
  protected readonly actionLabel = computed(() => {
    const t = this.target();
    if (this.busy()) return this.mode() === 'elsewhere' ? 'Starting…' : 'Switching…';
    if (!t) return 'Pick an account';
    return this.mode() === 'elsewhere' ? `Continue on ${t.name}` : `Switch to ${t.name}`;
  });

  constructor() {
    // Every opening starts over, with the accounts at hand.
    effect(() => {
      if (!this.open()) return;
      untracked(() => {
        this.pickedSession.set(null);
        this.pickedTarget.set(null);
        this.when.set('idle');
        this.reset();
        void this.accounts.ensureLoaded();
      });
    });
    // The summary took over the switch this sheet asked for (or it landed).
    effect(() => {
      const s = this.session();
      const asked = this.askedPending();
      if (!asked || !s) return;
      if (switchingToOf(s) || s.account?.id === asked) {
        untracked(() => this.askedPending.set(null));
      }
    });
  }

  protected name(s: SessionSummary): string {
    return sessionName(s);
  }

  protected status(s: SessionSummary): string {
    return s.live ? statusLabel(s.status) : 'Ended';
  }

  protected pickSession(id: string): void {
    this.pickedSession.set(id);
    this.reset();
  }

  protected pickTarget(id: string): void {
    this.pickedTarget.set(id);
    this.error.set('');
    this.ask.set(null);
  }

  protected async go(): Promise<void> {
    const s = this.session();
    const t = this.target();
    if (!s || !t || this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      if (this.mode() === 'elsewhere') {
        const res = await this.actions.continueOn(s, t);
        if (res.ok) {
          this.closed.emit();
          return;
        }
        throw res.error;
      }
      // An API conversation has no "when": between turns only.
      const when =
        this.mode() === 'here' && this.busyTurn() && !this.bridge() ? this.when() : undefined;
      const reply = normalizeSwitchReply(
        await this.api.switchSessionAccount(s.id, { account: t.id, ...(when ? { when } : {}) }),
        t.id,
      );
      if (reply.pending) {
        this.askedPending.set(t.id);
        this.notices.info(`${sessionName(s)} switches to ${t.name} once its turn ends.`);
        return;
      }
      // The Mac tells every client how the switch of a running session
      // ended ("… goes on under Work"); one that doesn't run, only here.
      if (this.mode() === 'later') this.notices.success(switchedText(t.name, reply.moved));
      this.closed.emit();
    } catch (e) {
      this.refused(e, t);
    } finally {
      this.busy.set(false);
    }
  }

  /** Remember the consent, then go on as asked. */
  protected async allow(): Promise<void> {
    const a = this.ask();
    if (!a || this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      await this.accounts.setCopyConsent(a.from, a.to, true);
    } catch (e) {
      this.busy.set(false);
      this.error.set(`Could not allow it: ${describeError(e).text}`);
      return;
    }
    this.busy.set(false);
    this.ask.set(null);
    await this.go();
  }

  protected async cancelPending(): Promise<void> {
    const s = this.session();
    if (!s || this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      await this.api.cancelSessionSwitch(s.id);
      this.askedPending.set(null);
      this.notices.info('The switch is cancelled: it stays on its account.');
    } catch (e) {
      this.error.set(switchErrorText(e));
    } finally {
      this.busy.set(false);
    }
  }

  private refused(e: unknown, t: AccountView): void {
    const pair = consentNeeded(e);
    if (pair) {
      this.ask.set({
        ...pair,
        fromName: this.accounts.byId(pair.from)?.name ?? this.source()?.name ?? 'this account',
        toName: this.accounts.byId(pair.to)?.name ?? t.name,
      });
      return;
    }
    if (e instanceof ApiError && e.code === 'session_not_owned') {
      // Another app runs it after all: Continue on is what can be done.
      this.runsElsewhere.set(true);
    }
    if (e instanceof ApiError && e.code === 'unknown_account') void this.accounts.refresh();
    this.error.set(switchErrorText(e, t.name));
  }

  private reset(): void {
    this.error.set('');
    this.ask.set(null);
    this.askedPending.set(null);
    this.runsElsewhere.set(false);
  }
}
