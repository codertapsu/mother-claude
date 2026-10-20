import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
  viewChild,
} from '@angular/core';
import { FormsModule } from '@angular/forms';

import {
  ACCOUNT_COLORS,
  ACCOUNT_NAME_MAX,
  accountNameProblem,
  suggestAccountDir,
} from '../../core/accounts';
import { AccountsService } from '../../core/accounts.service';
import { describeError } from '../../core/api.service';
import { AccountKind, NewAccount } from '../../core/models';
import { secretStoreText, signInLine } from '../../shared/accounts';
import { IconComponent } from '../../shared/icon.component';
import { SheetComponent } from '../../shared/sheet.component';
import { AccountColorsComponent } from './account-colors.component';
import { AccountSignInComponent } from './account-signin.component';
import { AccountTokenComponent } from './account-token.component';

type Step = 'kind' | 'details' | 'connect';

/**
 * Desktop: add a Claude account (docs/ACCOUNTS.md §6.1, D2). Two kinds,
 * one sentence each: shared data with another subscription (a token used
 * with this Mac's own directory: recommended), or a separate directory (a
 * whole Claude Code setup of its own). Then its sign-in right away: the
 * token's `setup-token` run (or a pasted token), or the directory's
 * browser login.
 */
@Component({
  selector: 'mc-account-add-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    FormsModule,
    AccountColorsComponent,
    AccountSignInComponent,
    AccountTokenComponent,
    IconComponent,
    SheetComponent,
  ],
  template: `
    <mc-sheet [open]="open()" [title]="title()" (closed)="closed.emit()">
      @if (open()) {
        @switch (step()) {
          @case ('kind') {
            <div class="kinds" role="radiogroup" aria-label="Kind of account">
              <button
                type="button"
                role="radio"
                class="kind"
                [class.on]="kind() === 'token'"
                [attr.aria-checked]="kind() === 'token'"
                (click)="kind.set('token')"
              >
                <span class="kt">
                  <strong>Shared data, another account</strong>
                  <span class="badge accent">recommended</span>
                </span>
                <span class="muted small">
                  Uses this Mac's existing projects and conversations with another Claude
                  subscription. You sign in once in the browser; the token is kept in
                  {{ place() }}.
                </span>
              </button>
              <button
                type="button"
                role="radio"
                class="kind"
                [class.on]="kind() === 'directory'"
                [attr.aria-checked]="kind() === 'directory'"
                (click)="kind.set('directory')"
              >
                <span class="kt"><strong>Separate directory</strong></span>
                <span class="muted small">
                  A fully separate Claude Code setup — its own settings, plugins, MCP servers and
                  conversations — in a folder like ~/.claude-work. It signs in with its own browser
                  login.
                </span>
              </button>
            </div>
            <div class="row actions">
              <button class="btn primary" type="button" (click)="step.set('details')">Next</button>
              <button class="btn ghost" type="button" (click)="closed.emit()">Cancel</button>
            </div>
          }

          @case ('details') {
            <form class="form" (submit)="create($event)">
              <label class="field">
                <span class="field-label">Name</span>
                <input
                  name="name"
                  autocomplete="off"
                  [attr.maxlength]="nameMax"
                  placeholder="e.g. Work, or Marcus (Pro)"
                  [ngModel]="name()"
                  (ngModelChange)="name.set($event); error.set('')"
                />
              </label>
              @if (nameProblem()) {
                <p class="error-text small">{{ nameProblem() }}</p>
              }

              @if (kind() === 'directory') {
                <label class="field">
                  <span class="field-label">Directory</span>
                  <input
                    name="dir"
                    autocomplete="off"
                    spellcheck="false"
                    autocapitalize="off"
                    [ngModel]="dirValue()"
                    (ngModelChange)="dir.set($event); dirTouched.set(true); error.set('')"
                  />
                </label>
                <p class="field-hint">
                  Created when it doesn't exist yet. Claude Code keeps this account's settings,
                  sign-in and conversations there; nothing in your current one changes.
                </p>
              } @else {
                <p class="muted small">
                  Once it is added you sign in once in the browser on this Mac and the token is
                  stored for it.
                </p>
                @if (!pasteOpen()) {
                  <button class="btn ghost small left" type="button" (click)="pasteOpen.set(true)">
                    I already have a token — paste it
                  </button>
                } @else {
                  <label class="field">
                    <span class="field-label">Token</span>
                    <input
                      #tok
                      type="password"
                      name="token"
                      autocomplete="off"
                      spellcheck="false"
                      autocapitalize="off"
                      placeholder="sk-ant-oat01-…"
                    />
                  </label>
                  <p class="field-hint">
                    Sent once to the Mac, checked with Claude Code and kept in {{ place() }}; it is
                    never shown again.
                  </p>
                }
              }

              <div class="field">
                <span class="field-label">Colour</span>
                <mc-account-colors label="Colour" [value]="color()" (picked)="color.set($event)" />
              </div>

              @if (error()) {
                <p class="error-text small" role="alert">{{ error() }}</p>
              }
              <div class="row actions">
                <button class="btn primary" type="submit" [disabled]="busy()">
                  <mc-icon name="plus" [size]="15" /> {{ busy() ? 'Adding…' : 'Add account' }}
                </button>
                <button class="btn ghost" type="button" [disabled]="busy()" (click)="back()">
                  Back
                </button>
              </div>
            </form>
          }

          @case ('connect') {
            @if (created(); as a) {
              @if (a.kind === 'token') {
                @if (a.tokenStored) {
                  <p class="notice ok small" role="status">
                    <mc-icon name="check" [size]="14" /> {{ a.name }} is ready
                    {{ a.signedIn ? '— ' + signedInText(a) : '' }}.
                  </p>
                } @else {
                  <p class="small">
                    Added <strong>{{ a.name }}</strong
                    >. Now get its token: sign in once in the browser with the Claude account it is
                    for.
                  </p>
                  <mc-account-token [account]="a" />
                }
              } @else {
                <p class="small">
                  Added <strong>{{ a.name }}</strong
                  >. Sign it in now: the browser opens on this Mac, for the Claude account this
                  directory is for.
                </p>
                <mc-account-signin [account]="a" />
              }
            }
            <div class="row actions">
              <button class="btn" type="button" (click)="closed.emit()">
                {{ ready() ? 'Done' : 'Later' }}
              </button>
            </div>
          }
        }
      }
    </mc-sheet>
  `,
  styles: [
    `
      .kinds {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
      }
      .kind {
        appearance: none;
        display: flex;
        flex-direction: column;
        gap: 0.3rem;
        width: 100%;
        padding: 0.75rem 0.85rem;
        text-align: left;
        font: inherit;
        color: var(--text);
        background: var(--panel-2);
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        cursor: pointer;
      }
      .kind.on {
        border-color: var(--accent);
        background: var(--accent-soft);
      }
      .kt {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.4rem;
      }
      .form {
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
      }
      .form p {
        margin: 0;
      }
      .left {
        align-self: flex-start;
      }
      .actions {
        margin-top: 0.9rem;
      }
      .form .actions {
        margin-top: 0.3rem;
      }
    `,
  ],
})
export class AccountAddSheetComponent {
  private accounts = inject(AccountsService);

  readonly open = input(false);
  readonly closed = output<void>();

  private readonly tokenInput = viewChild<ElementRef<HTMLInputElement>>('tok');

  protected readonly nameMax = ACCOUNT_NAME_MAX;
  protected readonly step = signal<Step>('kind');
  protected readonly kind = signal<AccountKind>('token');
  protected readonly name = signal('');
  protected readonly dir = signal('');
  protected readonly dirTouched = signal(false);
  protected readonly color = signal<string | null>(null);
  protected readonly pasteOpen = signal(false);
  protected readonly busy = signal(false);
  protected readonly error = signal('');
  private readonly createdId = signal<string | null>(null);
  private readonly nameTouched = computed(() => this.name().length > 0);

  protected readonly created = computed(() => this.accounts.byId(this.createdId()));
  protected readonly ready = computed(() => {
    const a = this.created();
    return !!a && (a.kind === 'token' ? a.tokenStored : a.signedIn);
  });
  protected readonly title = computed(() =>
    this.step() === 'connect' ? 'Sign the account in' : 'Add a Claude account',
  );
  protected readonly nameProblem = computed(() =>
    this.nameTouched() ? accountNameProblem(this.name(), this.accounts.list()) : '',
  );
  /** The directory typed, else one suggested from the name. */
  protected readonly dirValue = computed(() =>
    this.dirTouched() ? this.dir() : suggestAccountDir(this.name()),
  );
  protected readonly place = computed(
    () => secretStoreText(this.accounts.view()?.secretStore) || 'the Keychain',
  );

  constructor() {
    // Every opening starts over.
    effect(() => {
      if (!this.open()) return;
      untracked(() => {
        this.step.set('kind');
        this.kind.set('token');
        this.name.set('');
        this.dir.set('');
        this.dirTouched.set(false);
        // The next colour no account has yet, as the server would pick it.
        const used = new Set(this.accounts.list().map((a) => a.color));
        this.color.set(ACCOUNT_COLORS.find((c) => !used.has(c)) ?? ACCOUNT_COLORS[1]);
        this.pasteOpen.set(false);
        this.error.set('');
        this.createdId.set(null);
      });
    });
  }

  protected signedInText(a: Parameters<typeof signInLine>[0]): string {
    return signInLine(a).text;
  }

  protected back(): void {
    this.clearToken();
    this.error.set('');
    this.step.set('kind');
  }

  protected async create(e: Event): Promise<void> {
    e.preventDefault();
    if (this.busy()) return;
    const name = this.name().trim();
    const problem = accountNameProblem(name, this.accounts.list());
    if (problem) {
      this.error.set(problem);
      return;
    }
    const color = this.color();
    let body: NewAccount;
    if (this.kind() === 'directory') {
      const dir = this.dirValue().trim();
      if (!dir.startsWith('/') && !dir.startsWith('~/')) {
        this.error.set('Enter the directory as a full path, starting with / or ~/.');
        return;
      }
      body = { name, kind: 'directory', dir, ...(color ? { color } : {}) };
    } else {
      // Read once and clear the field: the token travels only in this body.
      const token = this.clearToken();
      body = { name, kind: 'token', ...(token ? { token } : {}), ...(color ? { color } : {}) };
    }
    this.busy.set(true);
    this.error.set('');
    try {
      const account = await this.accounts.add(body);
      this.createdId.set(account.id);
      this.step.set('connect');
    } catch (err) {
      this.error.set(describeError(err).text);
    } finally {
      this.busy.set(false);
    }
  }

  /** Empty the token field; returns what it held. */
  private clearToken(): string {
    const field = this.tokenInput()?.nativeElement;
    const token = field?.value.trim() ?? '';
    if (field) field.value = '';
    return token;
  }
}
