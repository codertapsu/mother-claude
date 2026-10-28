import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  input,
  output,
  signal,
} from '@angular/core';

import { AccountsService } from '../../core/accounts.service';
import { ApiService } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { AccountView } from '../../core/models';
import { identityWarningText, signInWaitText } from '../../core/sign-in';
import { signInErrorText } from '../../shared/accounts';

/**
 * Who an account signed in as is not who it was, or is who another account
 * signed in as (docs/ACCOUNTS.md §15.3): the warning, and on the desktop
 * *Keep* (it is fine) or *Sign in again* (the sign-in panel takes over; a
 * login asks for the account's known email again).
 */
@Component({
  selector: 'mc-identity-warning',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (account().identityWarning) {
      <div class="notice warn small" role="status">
        <p>{{ text() }}</p>
        @if (isDesktop()) {
          <div class="row">
            <button class="btn small" type="button" [disabled]="busy()" (click)="keep()">
              Keep
            </button>
            <button
              class="btn small"
              type="button"
              [disabled]="busy() || !!wait()"
              (click)="again()"
            >
              {{ wait() || 'Sign in again' }}
            </button>
          </div>
        }
        @if (error()) {
          <p class="error-text small" role="alert">{{ error() }}</p>
        }
      </div>
    }
  `,
  styles: [
    `
      .notice {
        display: flex;
        flex-direction: column;
        gap: 0.45rem;
      }
      p {
        margin: 0;
      }
      .row {
        flex-wrap: wrap;
        gap: 0.4rem;
      }
    `,
  ],
})
export class IdentityWarningComponent {
  private api = inject(ApiService);
  private accounts = inject(AccountsService);
  protected readonly isDesktop = inject(AuthService).isDesktop;

  readonly account = input.required<AccountView>();
  /** A new sign-in started (the host may show its panel). */
  readonly started = output<void>();

  protected readonly busy = signal(false);
  protected readonly error = signal('');
  protected readonly text = computed(() => {
    const a = this.account();
    const w = a.identityWarning;
    if (!w) return '';
    const said = identityWarningText(w, (id) => this.accounts.byId(id)?.name);
    return a.email && w.kind === 'changed' ? `${said} Now: ${a.email}.` : said;
  });
  /** Another account's sign-in runs: "Waiting for Work's sign-in". */
  protected readonly wait = computed(() => signInWaitText(this.account(), this.accounts.list()));

  protected async keep(): Promise<void> {
    await this.act(async () => {
      await this.api.acceptIdentity(this.account().id);
      await this.accounts.refresh();
    });
  }

  protected async again(): Promise<void> {
    const a = this.account();
    await this.act(async () => {
      if (a.kind === 'token') {
        await this.api.startTokenCapture(a.id);
        await this.accounts.refresh();
      } else {
        await this.accounts.login(a.id);
      }
      this.started.emit();
    });
  }

  private async act(run: () => Promise<void>): Promise<void> {
    if (this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      await run();
    } catch (e) {
      this.error.set(signInErrorText(e, (id) => this.accounts.byId(id)?.name));
    } finally {
      this.busy.set(false);
    }
  }
}
