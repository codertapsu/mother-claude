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
import { ApiError, describeError } from '../core/api.service';
import { AccountRef, AccountView, CopyConsent, SessionSummary } from '../core/models';
import { NowService } from '../core/now.service';
import { SessionActionsService } from '../core/session-actions.service';
import { AccountChipComponent } from './account-chip.component';
import {
  accountErrorText,
  cannotRunReason,
  consentNeeded,
  signInLine,
  usageLine,
} from './accounts';
import { IconComponent } from './icon.component';
import { SheetComponent } from './sheet.component';
import { capitalize, sessionName, statusLabel } from './util';

interface Target {
  account: AccountView;
  color: string;
  signIn: string;
  usage: string;
  limited: boolean;
  /** Why it can't be picked ('' when it can). */
  blocked: string;
  /** What picking it means beyond a fork ('' for nothing). */
  note: string;
}

/** The question before the first copy between two accounts. */
interface ConsentAsk extends CopyConsent {
  fromName: string;
  toName: string;
}

/** A session that can be continued from here at all. */
export function canContinue(s: SessionSummary): boolean {
  return s.owned || !!s.control?.fork || !!s.control?.continueOwned;
}

/**
 * "Continue on another account" (docs/ACCOUNTS.md §6.2): pick the account
 * (and, from the sessions list, the conversation) and the conversation goes
 * on there as a copy, the original staying as it was — a session Mother
 * Claude runs keeps running. An account with its own directory gets the
 * conversation copied there first: the desktop or a paired phone is asked
 * once per pair of accounts (docs/ACCOUNTS.md §13.9; the answer is kept,
 * and revocable on the Accounts page). Accounts that can't run say why.
 */
@Component({
  selector: 'mc-continue-on-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, AccountChipComponent, IconComponent, SheetComponent],
  template: `
    <mc-sheet [open]="open()" title="Continue on another account" (closed)="closed.emit()">
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
            The conversation continues on the account you pick, as a copy; this one stays as it was.
          </p>

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

          @if (error()) {
            <p class="notice error small" role="alert">{{ error() }}</p>
          }

          @if (ask(); as a) {
            <div class="notice warn small ask" role="group" aria-label="Allow copies">
              <p>
                Copy conversations from <strong>{{ a.fromName }}</strong> to
                <strong>{{ a.toName }}</strong> when you continue them there? Asked once; you can
                revoke it on the Accounts page.
              </p>
              <div class="row">
                <button
                  class="btn primary small"
                  type="button"
                  [disabled]="busy()"
                  (click)="allow()"
                >
                  {{ busy() ? 'Copying…' : 'Copy and continue' }}
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
                [disabled]="!target() || busy()"
                (click)="go()"
              >
                <mc-icon name="play" [size]="14" />
                {{
                  busy()
                    ? 'Starting…'
                    : target()
                      ? 'Continue on ' + target()!.name
                      : 'Pick an account'
                }}
              </button>
            </div>
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
      .notice {
        margin: 0.75rem 0 0;
      }
      .ask p {
        margin: 0 0 0.6rem;
        line-height: 1.5;
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
export class ContinueOnSheetComponent {
  protected readonly accounts = inject(AccountsService);
  private actions = inject(SessionActionsService);
  private clock = inject(NowService);

  readonly open = input(false);
  /** The conversation(s) to choose from: one from a session's menu, those
   * of an account at its limit from the sessions list. */
  readonly sessions = input<SessionSummary[]>([]);
  /** The account to preselect when it can take the conversation. */
  readonly suggested = input<string | null>(null);
  readonly closed = output<void>();

  protected readonly busy = signal(false);
  protected readonly error = signal('');
  /** Asked to allow copies between two accounts. */
  protected readonly ask = signal<ConsentAsk | null>(null);
  private readonly pickedSession = signal<string | null>(null);
  private readonly pickedTarget = signal<string | null>(null);

  protected readonly session = computed<SessionSummary | undefined>(() => {
    const list = this.sessions();
    return list.find((s) => s.id === this.pickedSession()) ?? list[0];
  });
  /** Where the conversation is now (the Default when it names none). */
  protected readonly source = computed<AccountRef | undefined>(() => this.session()?.account);
  protected readonly targets = computed<Target[]>(() => {
    const from: Pick<AccountRef, 'id' | 'kind'> = this.source() ?? {
      id: DEFAULT_ACCOUNT,
      kind: 'directory',
    };
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
          note = this.accounts.hasCopyConsent(from.id, a.id)
            ? `${where}: the conversation is copied there first.`
            : `${where}: the conversation is copied there first (asked once).`;
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
    const open = this.targets().filter((t) => !t.blocked);
    const has = (id: string | null) => !!id && open.some((t) => t.account.id === id);
    const picked = this.pickedTarget();
    if (has(picked)) return picked as string;
    const suggested = this.suggested();
    if (has(suggested)) return suggested as string;
    return open[0]?.account.id ?? '';
  });
  protected readonly target = computed(() => this.accounts.byId(this.targetId()));

  constructor() {
    // Every opening starts over, with the accounts at hand.
    effect(() => {
      if (!this.open()) return;
      untracked(() => {
        this.pickedSession.set(null);
        this.pickedTarget.set(null);
        this.error.set('');
        this.ask.set(null);
        void this.accounts.ensureLoaded();
      });
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
    this.error.set('');
    this.ask.set(null);
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
    const res = await this.actions.continueOn(s, t);
    this.busy.set(false);
    if (res.ok) {
      this.closed.emit();
      return;
    }
    const pair = consentNeeded(res.error);
    if (pair) {
      this.ask.set({
        ...pair,
        fromName: this.accounts.byId(pair.from)?.name ?? this.source()?.name ?? 'this account',
        toName: this.accounts.byId(pair.to)?.name ?? t.name,
      });
      return;
    }
    this.fail(res.error, t);
  }

  /** Remember the consent, then continue as asked. */
  protected async allow(): Promise<void> {
    const a = this.ask();
    if (!a || this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      await this.accounts.setCopyConsent(a.from, a.to, true);
    } catch (e) {
      this.busy.set(false);
      this.error.set(`Could not allow the copies: ${describeError(e).text}`);
      return;
    }
    this.busy.set(false);
    this.ask.set(null);
    await this.go();
  }

  private fail(e: unknown, t: AccountView): void {
    if (e instanceof ApiError && e.code === 'unknown_account') void this.accounts.refresh();
    this.error.set(accountErrorText(e, t.name));
  }
}
