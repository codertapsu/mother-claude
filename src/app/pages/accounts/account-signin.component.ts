import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';

import { AccountsService } from '../../core/accounts.service';
import { AccountView, SignInState } from '../../core/models';
import { NowService } from '../../core/now.service';
import { signInEndedLately, signInRunning, signInWaitText } from '../../core/sign-in';
import { signInErrorText, signInLine } from '../../shared/accounts';
import { SignInPanelComponent } from './sign-in-panel.component';

/**
 * Desktop: sign a directory account (or the Default) in with `claude auth
 * login` for its own directory (docs/ACCOUNTS.md §15.2): while it runs, the
 * sign-in panel (its link opened in the default browser by the Mac, the
 * private window, Copy link, Cancel). One sign-in runs at a time: while
 * another account's runs, the button says which and waits. An older server
 * watches a login without a link: "Finish signing in in the browser".
 */
@Component({
  selector: 'mc-account-signin',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [SignInPanelComponent],
  template: `
    @let a = account();
    @if (runs()) {
      <mc-sign-in-panel [account]="a" (finished)="finished($event)" />
    } @else if (a.signingIn) {
      <p class="wait">
        <span class="spin" aria-hidden="true"></span>
        Finish signing in in the browser on this Mac…
      </p>
      <p class="muted small">
        Sign in there with the Claude account {{ a.isDefault ? 'this Mac uses' : 'for ' + a.name }}.
        This page updates by itself once it is done.
      </p>
      <button class="btn ghost small left" type="button" [disabled]="busy()" (click)="check()">
        Check now
      </button>
    } @else {
      @if (outcome()) {
        <p class="notice warn small" role="status">{{ outcome() }}</p>
      }
      <p class="state small" [attr.data-tone]="line().tone">{{ line().text }}</p>
      @if (a.isDefault && a.signedIn) {
        <p class="muted small">
          Signing the Default in again changes the account of every session that runs without a
          token of its own.
        </p>
      }
      <div class="row">
        <button
          class="btn"
          type="button"
          [class.primary]="!a.signedIn"
          [disabled]="busy() || !!wait()"
          (click)="login()"
        >
          {{ wait() || (busy() ? 'Starting…' : a.signedIn ? 'Sign in again' : 'Sign in now') }}
        </button>
        <label class="check small">
          <input
            type="checkbox"
            [checked]="consoleAccount()"
            (change)="consoleAccount.set($any($event.target).checked)"
          />
          Anthropic Console account (API billing)
        </label>
      </div>
      <p class="muted small">
        The sign-in opens in this Mac's default browser (claude auth login){{
          a.email ? '; it asks for ' + a.email + ' again' : ''
        }}.
      </p>
    }
    @if (error()) {
      <p class="error-text small" role="alert">{{ error() }}</p>
    }
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.45rem;
      }
      p {
        margin: 0;
      }
      .wait {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        font-size: 0.9rem;
      }
      .spin {
        flex: 0 0 auto;
        width: 0.9rem;
        height: 0.9rem;
        border: 2px solid var(--border-strong);
        border-top-color: var(--accent);
        border-radius: 50%;
        animation: mc-spin 0.8s linear infinite;
      }
      .state {
        color: var(--tone, var(--text));
      }
      .left {
        align-self: flex-start;
      }
      .check {
        display: inline-flex;
        align-items: center;
        gap: 0.4rem;
        color: var(--muted);
        cursor: pointer;
      }
    `,
  ],
})
export class AccountSignInComponent {
  private accounts = inject(AccountsService);
  private clock = inject(NowService);

  readonly account = input.required<AccountView>();

  protected readonly busy = signal(false);
  protected readonly error = signal('');
  protected readonly consoleAccount = signal(false);
  protected readonly line = computed(() => signInLine(this.account()));
  /** Its sign-in runs (the panel shows it). */
  protected readonly runs = computed(() => signInRunning(this.account().signIn));
  /** Another account's sign-in runs: "Waiting for Work's sign-in". */
  protected readonly wait = computed(() => signInWaitText(this.account(), this.accounts.list()));
  /** How the last sign-in ended, when it brought nothing — while that is
   * news (the run stays on the account; its news goes quietly). */
  protected readonly outcome = computed(() => {
    const r = this.account().signIn;
    if (r?.kind !== 'login' || !signInEndedLately(r, this.clock.now())) return '';
    if (r.state === 'failed') {
      return r.error
        ? `The sign-in ended without signing in: ${r.error}`
        : 'The sign-in ended without signing in.';
    }
    return r.state === 'cancelled' ? 'The sign-in was cancelled.' : '';
  });

  protected async login(): Promise<void> {
    if (this.busy() || this.wait()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      await this.accounts.login(this.account().id, this.consoleAccount());
    } catch (e) {
      this.error.set(signInErrorText(e, (id) => this.accounts.byId(id)?.name));
    } finally {
      this.busy.set(false);
    }
  }

  /** The sign-in stopped: the accounts are read again for who it is. */
  protected finished(state: SignInState): void {
    if (state === 'done') void this.accounts.refresh();
  }

  protected async check(): Promise<void> {
    this.busy.set(true);
    try {
      await this.accounts.refresh();
    } finally {
      this.busy.set(false);
    }
  }
}
