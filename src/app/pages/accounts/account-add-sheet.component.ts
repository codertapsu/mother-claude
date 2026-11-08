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
import { AuthService } from '../../core/auth.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { AccountKind, NewAccount } from '../../core/models';
import { folderPathProblem, homeFolder, machineNoun } from '../../core/paths';
import { signInLine } from '../../shared/accounts';
import { IconComponent } from '../../shared/icon.component';
import { SheetComponent } from '../../shared/sheet.component';
import { AccountColorsComponent } from './account-colors.component';
import { AccountSignInComponent } from './account-signin.component';
import { AccountTokenComponent } from './account-token.component';
import { tokenStoreText } from './token-store';

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
    TPipe,
  ],
  template: `
    <mc-sheet [open]="open()" [title]="title()" (closed)="closed.emit()">
      @if (open()) {
        @switch (step()) {
          @case ('kind') {
            <div class="kinds" role="radiogroup" [attr.aria-label]="'Kind of account' | t">
              <button
                type="button"
                role="radio"
                class="kind"
                [class.on]="kind() === 'token'"
                [attr.aria-checked]="kind() === 'token'"
                (click)="kind.set('token')"
              >
                <span class="kt">
                  <strong>{{ 'Shared data, another account' | t }}</strong>
                  <span class="badge accent">{{ 'recommended' | t }}</span>
                </span>
                <span class="muted small">
                  {{
                    "Uses this {computer}'s existing projects and conversations with another Claude subscription. You sign in once in the browser; the token is kept in {place}."
                      | t: { computer: computer(), place: place() }
                  }}
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
                <span class="kt"
                  ><strong>{{ 'Separate directory' | t }}</strong></span
                >
                <span class="muted small">
                  {{
                    'A fully separate Claude Code setup — its own settings, plugins, MCP servers and conversations — in a folder like {dir}. It signs in with its own browser login.'
                      | t: { dir: exampleDir() }
                  }}
                </span>
              </button>
            </div>
            <div class="row actions">
              <button class="btn primary" type="button" (click)="step.set('details')">
                {{ 'Next' | t }}
              </button>
              <button class="btn ghost" type="button" (click)="closed.emit()">
                {{ 'Cancel' | t }}
              </button>
            </div>
          }

          @case ('details') {
            <form class="form" (submit)="create($event)">
              <label class="field">
                <span class="field-label">{{ 'Name' | t }}</span>
                <input
                  name="name"
                  autocomplete="off"
                  [attr.maxlength]="nameMax"
                  [placeholder]="'e.g. Work, or Marcus (Pro)' | t"
                  [ngModel]="name()"
                  (ngModelChange)="name.set($event); error.set('')"
                />
              </label>
              @if (nameProblem()) {
                <p class="error-text small">{{ nameProblem() }}</p>
              }

              @if (kind() === 'directory') {
                <label class="field">
                  <span class="field-label">{{ 'Directory' | t }}</span>
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
                  {{
                    "Created when it doesn't exist yet. Claude Code keeps this account's settings, sign-in and conversations there; nothing in your current one changes."
                      | t
                  }}
                </p>
              } @else {
                <p class="muted small">
                  {{
                    'Once it is added you sign in once in the browser on this {computer} and the token is stored for it.'
                      | t: { computer: computer() }
                  }}
                </p>
                @if (!pasteOpen()) {
                  <button class="btn ghost small left" type="button" (click)="pasteOpen.set(true)">
                    {{ 'I already have a token — paste it' | t }}
                  </button>
                } @else {
                  <label class="field">
                    <span class="field-label">{{ 'Token' | t }}</span>
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
                    {{
                      'Sent once to the {computer}, checked with Claude Code and kept in {place}; it is never shown again.'
                        | t: { computer: computer(), place: place() }
                    }}
                  </p>
                }
              }

              <div class="field">
                <span class="field-label">{{ 'Colour' | t }}</span>
                <mc-account-colors
                  [label]="'Colour' | t"
                  [value]="color()"
                  (picked)="color.set($event)"
                />
              </div>

              @if (error()) {
                <p class="error-text small" role="alert">{{ error() }}</p>
              }
              <div class="row actions">
                <button class="btn primary" type="submit" [disabled]="busy()">
                  <mc-icon name="plus" [size]="15" />
                  {{ busy() ? ('Adding…' | t) : ('Add account' | t) }}
                </button>
                <button class="btn ghost" type="button" [disabled]="busy()" (click)="back()">
                  {{ 'Back' | t }}
                </button>
              </div>
            </form>
          }

          @case ('connect') {
            @if (created(); as a) {
              @if (a.kind === 'token') {
                @if (a.tokenStored) {
                  <p class="notice ok small" role="status">
                    <mc-icon name="check" [size]="14" />
                    {{
                      a.signedIn
                        ? ('{name} is ready — {state}.'
                          | t: { name: a.name, state: signedInText(a) })
                        : ('{name} is ready.' | t: { name: a.name })
                    }}
                  </p>
                } @else {
                  <p class="small">
                    {{ 'Added' | t }} <strong>{{ a.name }}</strong
                    >.
                    {{
                      'Now get its token: sign in once in the browser with the Claude account it is for.'
                        | t
                    }}
                  </p>
                  <mc-account-token [account]="a" />
                }
              } @else {
                <p class="small">
                  {{ 'Added' | t }} <strong>{{ a.name }}</strong
                  >.
                  {{
                    'Sign it in now: the browser opens on this {computer}, for the Claude account this directory is for.'
                      | t: { computer: computer() }
                  }}
                </p>
                <mc-account-signin [account]="a" />
              }
            }
            <div class="row actions">
              <button class="btn" type="button" (click)="closed.emit()">
                {{ ready() ? ('Done' | t) : ('Later' | t) }}
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
  /** The desktop's system: the directory is typed its way. */
  private readonly hostOs = inject(AuthService).hostOs;
  protected readonly exampleDir = computed(() => homeFolder(this.hostOs(), '.claude-work'));
  /** The desktop's computer in a sentence ("this {computer}"): "Mac", "PC"
   * (Vietnamese "máy Mac", "máy PC"). */
  protected readonly computer = computed(() => tr(machineNoun(this.hostOs())));

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
    this.step() === 'connect' ? tr('Sign the account in') : tr('Add a Claude account'),
  );
  protected readonly nameProblem = computed(() =>
    this.nameTouched() ? accountNameProblem(this.name(), this.accounts.list()) : '',
  );
  /** The directory typed, else one suggested from the name. */
  protected readonly dirValue = computed(() =>
    this.dirTouched() ? this.dir() : suggestAccountDir(this.name(), this.hostOs()),
  );
  /** Where the token is kept: what the desktop says, else its system's own
   * store (an older server does not say). */
  protected readonly place = computed(() =>
    tokenStoreText(this.accounts.view()?.secretStore, this.hostOs()),
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
      // Either system's full path (or ~): the desktop has the last word.
      const problem = folderPathProblem(dir, this.hostOs(), { noun: 'directory' });
      if (problem) {
        this.error.set(problem);
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
