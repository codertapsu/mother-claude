import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';

import { AccountsService } from '../../core/accounts.service';
import { describeError } from '../../core/api.service';
import { AccountView } from '../../core/models';
import { signInLine } from '../../shared/accounts';

/**
 * Desktop: sign a directory account (or the Default) in with `claude auth
 * login` for its own directory. The browser opens on the Mac; the server
 * watches the sign-in and the account flips to signed in (the `accounts`
 * frame, or the Accounts page asking again every few seconds).
 */
@Component({
  selector: 'mc-account-signin',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @let a = account();
    @if (a.signingIn) {
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
          [disabled]="busy()"
          (click)="login()"
        >
          {{ busy() ? 'Opening the browser…' : a.signedIn ? 'Sign in again' : 'Sign in now' }}
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
      <p class="muted small">The sign-in opens in this Mac's browser (claude auth login).</p>
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

  readonly account = input.required<AccountView>();

  protected readonly busy = signal(false);
  protected readonly error = signal('');
  protected readonly consoleAccount = signal(false);
  protected readonly line = computed(() => signInLine(this.account()));

  protected async login(): Promise<void> {
    if (this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      await this.accounts.login(this.account().id, this.consoleAccount());
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(false);
    }
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
