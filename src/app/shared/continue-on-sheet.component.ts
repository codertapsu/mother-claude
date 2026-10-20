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
import { ApiError } from '../core/api.service';
import { AuthService } from '../core/auth.service';
import { AccountRef, AccountView, SessionSummary } from '../core/models';
import { NowService } from '../core/now.service';
import { SessionActionsService, isAlreadyOwned } from '../core/session-actions.service';
import { AccountChipComponent } from './account-chip.component';
import { accountErrorText, cannotRunReason, signInLine, usageLine } from './accounts';
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
}

/** A session that can be continued from here at all. */
export function canContinue(s: SessionSummary): boolean {
  return s.owned || !!s.control?.fork || !!s.control?.continueOwned;
}

/**
 * "Continue on another account" (docs/ACCOUNTS.md §6.2): pick the account
 * (and, from the sessions list, the conversation) and the conversation goes
 * on there as a copy, the original staying as it was. Accounts that can't
 * take it say why: no token here, signed out, or a separate directory (a
 * copy between directories arrives with Move/Copy). A session Mother
 * Claude is running is let go of only once it stopped: the sheet then
 * offers to stop it first.
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

          @if (needsStop() && target(); as t) {
            <p class="notice warn small" role="status">
              Mother Claude is running this session right now. To continue it on {{ t.name }} it
              stops here first (its current turn too, if one is running); the conversation then
              continues on {{ t.name }} as a copy.
            </p>
            <div class="row actions">
              <button class="btn primary" type="button" [disabled]="busy()" (click)="stopAndGo()">
                <mc-icon name="stop" [size]="14" />
                {{ busy() ? 'Stopping…' : 'Stop it and continue on ' + t.name }}
              </button>
              <button
                class="btn ghost"
                type="button"
                [disabled]="busy()"
                (click)="needsStop.set(false)"
              >
                Cancel
              </button>
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
          @if (isDesktop()) {
            <p class="muted small manage">
              <a routerLink="/accounts" (click)="closed.emit()">Manage accounts</a>
            </p>
          }
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
  protected readonly isDesktop = inject(AuthService).isDesktop;

  readonly open = input(false);
  /** The conversation(s) to choose from: one from a session's menu, those
   * of an account at its limit from the sessions list. */
  readonly sessions = input<SessionSummary[]>([]);
  /** The account to preselect when it can take the conversation. */
  readonly suggested = input<string | null>(null);
  readonly closed = output<void>();

  protected readonly busy = signal(false);
  protected readonly error = signal('');
  /** The server let us know Mother Claude runs it: offer to stop it first. */
  protected readonly needsStop = signal(false);
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
        return {
          account: a,
          color: isAccountColor(a.color) ? a.color : 'var(--accent)',
          signIn: signInLine(a).text,
          usage: usageLine(this.accounts.usageOf(a.id), now),
          limited: !!this.accounts.limitOf(a.id),
          blocked: reason
            ? `${capitalize(reason)}.`
            : needsCopy(from, a)
              ? 'Keeps its own directory: continuing there needs a copy of the conversation, which arrives with Move/Copy.'
              : '',
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
        this.needsStop.set(false);
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
    this.needsStop.set(false);
  }

  protected pickTarget(id: string): void {
    this.pickedTarget.set(id);
    this.error.set('');
    this.needsStop.set(false);
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
    if (isAlreadyOwned(res.error)) {
      this.needsStop.set(true);
      return;
    }
    this.fail(res.error, t);
  }

  protected async stopAndGo(): Promise<void> {
    const s = this.session();
    const t = this.target();
    if (!s || !t || this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    const res = await this.actions.stopThenContinueOn(s, t);
    this.busy.set(false);
    if (res.ok) {
      this.needsStop.set(false);
      this.closed.emit();
      return;
    }
    this.needsStop.set(false);
    this.fail(res.error, t);
  }

  private fail(e: unknown, t: AccountView): void {
    if (e instanceof ApiError && e.code === 'unknown_account') void this.accounts.refresh();
    this.error.set(
      isAlreadyOwned(e)
        ? 'The session has not stopped yet. Try again in a moment.'
        : accountErrorText(e, t.name),
    );
  }
}
