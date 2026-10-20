import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  linkedSignal,
  output,
  signal,
  untracked,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';

import { ACCOUNT_NAME_MAX, accountNameProblem, isAccountColor } from '../../core/accounts';
import { AccountsService } from '../../core/accounts.service';
import { ApiError, describeError } from '../../core/api.service';
import { AccountView } from '../../core/models';
import { SignInLine, accountKindLabel, secretStoreText, signInLine } from '../../shared/accounts';
import { IconComponent } from '../../shared/icon.component';
import { SheetComponent } from '../../shared/sheet.component';
import { shortPath } from '../../shared/util';
import { AccountColorsComponent } from './account-colors.component';
import { AccountSignInComponent } from './account-signin.component';
import { AccountTokenComponent } from './account-token.component';

/**
 * Desktop: one account's administration — its name and colour, its
 * sign-in (a browser login for a directory account, a token for a token
 * account), a fresh check of the sign-in, and removing it (which never
 * touches its directory or its Anthropic sign-in; refused while sessions
 * Mother Claude started run under it, or while the API runs with it as
 * the account its new conversations start under).
 */
@Component({
  selector: 'mc-account-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    FormsModule,
    RouterLink,
    AccountColorsComponent,
    AccountSignInComponent,
    AccountTokenComponent,
    IconComponent,
    SheetComponent,
  ],
  template: `
    <mc-sheet [open]="!!account()" [title]="account()?.name ?? 'Account'" (closed)="closed.emit()">
      @if (account(); as a) {
        <div class="ahead">
          <span class="adot" aria-hidden="true" [style.background]="color(a)"></span>
          <div class="ameta">
            <span class="small"
              >{{ kind(a) }}{{ a.isDefault ? ' · the Default account' : '' }}</span
            >
            @if (a.dir) {
              <span class="mono muted small break">{{ short(a.dir) }}</span>
            }
            <span class="small state" [attr.data-tone]="signIn(a).tone">{{ signIn(a).text }}</span>
          </div>
        </div>

        <h3>Name and colour</h3>
        <form class="look" (submit)="saveLook($event)">
          <label class="field">
            <span class="field-label">Name</span>
            <input
              name="name"
              autocomplete="off"
              [attr.maxlength]="nameMax"
              [ngModel]="name()"
              (ngModelChange)="name.set($event); error.set('')"
            />
          </label>
          @if (nameProblem()) {
            <p class="error-text small">{{ nameProblem() }}</p>
          }
          <mc-account-colors
            label="Colour"
            [value]="pickedColor()"
            [disabled]="busy()"
            (picked)="pickedColor.set($event)"
          />
          <div class="row">
            <button
              class="btn primary small"
              type="submit"
              [disabled]="!lookChanged() || !!nameProblem() || busy()"
            >
              Save
            </button>
          </div>
        </form>

        <h3>Sign-in</h3>
        @if (a.kind === 'token') {
          <mc-account-token [account]="a" />
        } @else {
          <mc-account-signin [account]="a" />
        }
        <div class="row">
          <button
            class="btn ghost small"
            type="button"
            [disabled]="a.checking || busy()"
            (click)="recheck()"
          >
            <mc-icon name="refresh" [size]="14" />
            {{ a.checking ? 'Checking…' : 'Check the sign-in again' }}
          </button>
        </div>

        @if (!a.isDefault) {
          <h3>Remove</h3>
          @if (!confirmRemove()) {
            <p class="muted small">{{ removeText(a) }}</p>
            <div class="row">
              <button
                class="btn danger small"
                type="button"
                [disabled]="busy()"
                (click)="confirmRemove.set(true)"
              >
                <mc-icon name="trash" [size]="14" /> Remove {{ a.name }}…
              </button>
            </div>
          } @else {
            <p class="notice warn small" role="alert">
              Remove <strong>{{ a.name }}</strong
              >? {{ removeText(a) }}
            </p>
            <div class="row">
              <button class="btn danger" type="button" [disabled]="busy()" (click)="remove(a)">
                {{ busy() ? 'Removing…' : 'Remove' }}
              </button>
              <button class="btn ghost" type="button" (click)="confirmRemove.set(false)">
                Keep it
              </button>
            </div>
          }
        }

        @if (error()) {
          <p class="error-text small" role="alert">{{ error() }}</p>
          @if (usedByApi()) {
            <p class="small">
              <a routerLink="/api" (click)="closed.emit()">Open the API page</a>
            </p>
          }
        }
        @if (notice()) {
          <p class="notice ok small" role="status">{{ notice() }}</p>
        }
      }
    </mc-sheet>
  `,
  styles: [
    `
      .ahead {
        display: flex;
        align-items: flex-start;
        gap: 0.65rem;
      }
      .adot {
        flex: 0 0 auto;
        width: 1rem;
        height: 1rem;
        margin-top: 0.15rem;
        border-radius: 50%;
      }
      .ameta {
        display: flex;
        flex-direction: column;
        gap: 0.1rem;
        min-width: 0;
      }
      .state {
        color: var(--tone, var(--text));
      }
      h3 {
        margin: 1.1rem 0 0.5rem;
        font-size: 0.9rem;
      }
      .look {
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
      }
      .row {
        margin-top: 0.5rem;
      }
      .look .row {
        margin-top: 0;
      }
      p {
        margin: 0.4rem 0 0;
      }
    `,
  ],
})
export class AccountSheetComponent {
  private accounts = inject(AccountsService);

  /** The account to manage (null: closed). */
  readonly accountId = input<string | null>(null);
  readonly closed = output<void>();

  protected readonly nameMax = ACCOUNT_NAME_MAX;
  protected readonly busy = signal(false);
  protected readonly error = signal('');
  protected readonly notice = signal('');
  protected readonly confirmRemove = signal(false);
  /** The removal was refused because the running API uses the account. */
  protected readonly usedByApi = signal(false);

  protected readonly account = computed(() => this.accounts.byId(this.accountId()));
  private readonly savedName = computed(() => this.account()?.name ?? '');
  private readonly savedColor = computed(() => this.account()?.color ?? '');
  protected readonly name = linkedSignal(() => this.savedName());
  protected readonly pickedColor = linkedSignal(() => this.savedColor());
  protected readonly nameProblem = computed(() => {
    const a = this.account();
    return a ? accountNameProblem(this.name(), this.accounts.list(), a.id) : '';
  });
  protected readonly lookChanged = computed(
    () =>
      this.name().trim() !== this.savedName() ||
      (this.pickedColor() || '') !== (this.savedColor() || ''),
  );

  constructor() {
    // Every opening starts over.
    effect(() => {
      this.accountId();
      untracked(() => {
        this.error.set('');
        this.notice.set('');
        this.confirmRemove.set(false);
        this.usedByApi.set(false);
      });
    });
  }

  protected kind(a: AccountView): string {
    return accountKindLabel(a);
  }

  protected signIn(a: AccountView): SignInLine {
    return signInLine(a);
  }

  protected short(p: string): string {
    return shortPath(p);
  }

  protected color(a: AccountView): string {
    return isAccountColor(a.color) ? a.color : 'var(--accent)';
  }

  protected removeText(a: AccountView): string {
    const fallback = 'New sessions, the API and projects that used it go back to the Default.';
    if (a.kind === 'token') {
      const place = secretStoreText(this.accounts.view()?.secretStore) || 'this Mac';
      return `Mother Claude forgets it and deletes its token from ${place}; the Anthropic account itself and its other sign-ins stay as they are. ${fallback}`;
    }
    const dir = a.dir ? ` (${shortPath(a.dir)})` : '';
    return `Mother Claude forgets it. Its directory${dir} and its Anthropic sign-in stay as they are: add it again any time to use them. ${fallback}`;
  }

  protected async saveLook(e: Event): Promise<void> {
    e.preventDefault();
    const a = this.account();
    if (!a || !this.lookChanged() || this.nameProblem() || this.busy()) return;
    const name = this.name().trim();
    const color = this.pickedColor();
    await this.run(
      () =>
        this.accounts.update(a.id, {
          ...(name !== a.name ? { name } : {}),
          ...(color && color !== a.color ? { color } : {}),
        }),
      'Saved.',
    );
  }

  protected async recheck(): Promise<void> {
    const a = this.account();
    if (a) await this.run(() => this.accounts.recheck(a.id), '');
  }

  protected async remove(a: AccountView): Promise<void> {
    const ok = await this.run(() => this.accounts.remove(a.id), '');
    if (ok) this.closed.emit();
    else this.confirmRemove.set(false);
  }

  private async run(fn: () => Promise<unknown>, ok: string): Promise<boolean> {
    this.busy.set(true);
    this.error.set('');
    this.notice.set('');
    this.usedByApi.set(false);
    try {
      await fn();
      if (ok) this.notice.set(ok);
      return true;
    } catch (e) {
      this.usedByApi.set(e instanceof ApiError && e.code === 'account_used_by_bridge');
      this.error.set(this.refusal(e));
      return false;
    } finally {
      this.busy.set(false);
    }
  }

  private refusal(e: unknown): string {
    const name = this.account()?.name ?? 'This account';
    if (e instanceof ApiError && e.code === 'account_in_use')
      return `${e.message} Stop them first.`;
    if (e instanceof ApiError && e.code === 'account_used_by_bridge') {
      return `The API is running with ${name} as the account its new conversations start under. Stop the API, or start it again with another account, then remove ${name}.`;
    }
    return describeError(e).text;
  }
}
