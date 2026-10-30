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

import { AccountsService } from '../../core/accounts.service';
import { ApiService, describeError } from '../../core/api.service';
import { AccountView, SignInBrowser, SignInState, SignInView } from '../../core/models';
import {
  advanceSignIn,
  browserLabel,
  isLongLivedToken,
  looksLikeToken,
  mergeSignIn,
  normalizeBrowsers,
  normalizeSignIn,
  signInRunning,
} from '../../core/sign-in';
import { signInErrorText } from '../../shared/accounts';
import { CopyButtonComponent } from '../../shared/copy-button.component';
import { IconComponent } from '../../shared/icon.component';

/** A running sign-in is read this often for its link and its state (frames
 * carry its state, never its link). */
const POLL_MS = 1_500;

/**
 * Desktop: an account's sign-in while it runs (docs/ACCOUNTS.md §15.2) — the
 * link with Copy link, *Open in browser* and, when the Mac has Chrome,
 * *Open in a private Chrome window*; the field for the code the browser
 * shows, from the moment there is a link (the page never hands the sign-in
 * back by itself: after approving, Claude shows a code to paste here); and
 * Cancel. A login opened the default browser by itself (the panel says so,
 * and offers the private window in case the wrong Claude account opened
 * there); a token sign-in opens nothing by itself: the panel asks first,
 * recommending the private window, since a browser signed in to another
 * Claude account would give that account's consent.
 *
 * A code sent shows as such until Claude Code answers; one it turns down
 * comes back with its words, and the field with it. A token pasted where
 * the code goes is never sent as one: a token account keeps it as its token
 * (the sign-in then ends), a login says it is not the code.
 */
@Component({
  selector: 'mc-sign-in-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, CopyButtonComponent, IconComponent],
  template: `
    @if (run(); as r) {
      <div class="flow" role="status" [attr.data-state]="r.state">
        @if (r.state === 'starting' || !r.url) {
          <p class="wait">
            <span class="spin" aria-hidden="true"></span>
            {{ r.state === 'starting' ? 'Starting the sign-in…' : 'Waiting for the sign-in link…' }}
          </p>
        } @else if (r.kind === 'login') {
          <!-- The Mac opened it in the default browser itself: never again
               from here; a private window is the other way. -->
          <p class="wait">
            <span class="spin" aria-hidden="true"></span>
            {{
              r.opened
                ? 'The sign-in opened in ' + where(r.opened) + '.'
                : 'The sign-in opens in your default browser…'
            }}
          </p>
          <p class="muted small">Sign in there with the Claude account {{ who() }}.</p>
          <div class="row opens">
            @if (privateOffered() && r.opened !== 'chrome-private') {
              <button
                class="btn small"
                type="button"
                [disabled]="busy()"
                (click)="openIn('chrome-private')"
              >
                <mc-icon name="shield" [size]="14" /> {{ label('chrome-private') }}
              </button>
            }
            <mc-copy-button [text]="r.url" label="Copy link" ariaLabel="Copy the sign-in link" />
          </div>
          @if (r.opened !== 'chrome-private') {
            <p class="muted small">
              {{
                privateOffered()
                  ? 'The wrong Claude account opened? A private window starts signed out.'
                  : 'The wrong Claude account opened? Copy the link into a private window of your browser.'
              }}
            </p>
          }
        } @else {
          <p class="ask">
            {{
              r.opened
                ? 'The sign-in opened in ' +
                  where(r.opened) +
                  ': get the token ' +
                  who() +
                  ' there.'
                : 'Open the sign-in page to get the token ' + who() + '.'
            }}
          </p>
          <p class="muted small why">
            Use a private window if you can: a browser already signed in to another Claude account
            would give its consent to the wrong one.
          </p>
          <div class="row opens">
            @for (b of tokenBrowsers(); track b; let first = $first) {
              <button
                class="btn small"
                type="button"
                [class.primary]="first"
                [disabled]="busy()"
                (click)="openIn(b)"
              >
                <mc-icon [name]="b === 'chrome-private' ? 'shield' : 'external'" [size]="14" />
                {{ label(b) }}{{ first && b === 'chrome-private' ? ' (recommended)' : '' }}
              </button>
            }
            <mc-copy-button [text]="r.url" label="Copy link" ariaLabel="Copy the sign-in link" />
          </div>
        }

        @if (r.url) {
          @if (r.codeSent) {
            <p class="wait">
              <span class="spin" aria-hidden="true"></span>
              Code sent: Claude Code is finishing the sign-in…
            </p>
          } @else {
            @if (r.codeError) {
              <p class="notice warn small" role="alert">
                Claude Code didn't take that code: {{ r.codeError }}
              </p>
            }
            <form class="code" [class.asked]="r.state === 'code'" (submit)="sendCode($event)">
              <label class="field">
                <span class="field-label">Code from the browser</span>
                <input
                  #codeInput
                  name="code"
                  autocomplete="off"
                  spellcheck="false"
                  autocapitalize="off"
                  [ngModel]="code()"
                  (ngModelChange)="code.set($event)"
                />
              </label>
              <p class="field-hint">
                {{
                  r.codeError
                    ? 'Paste the whole code the page shows. A code works once: for a new one, open the sign-in page again and approve.'
                    : 'After you approve in the browser, Claude shows a code: copy it and paste it here.'
                }}
              </p>
              <button class="btn primary small" type="submit" [disabled]="!code().trim() || busy()">
                Send the code
              </button>
            </form>
          }
        }

        <div class="row">
          <button class="btn ghost small" type="button" [disabled]="busy()" (click)="cancel()">
            Cancel
          </button>
        </div>
      </div>
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
        gap: 0.5rem;
      }
      p {
        margin: 0;
      }
      .flow,
      .code {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
      }
      .wait {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        font-size: 0.9rem;
      }
      .ask {
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
      .opens {
        flex-wrap: wrap;
        gap: 0.4rem;
      }
      /* The CLI asks for the code now. */
      .code.asked input {
        border-color: var(--accent);
      }
      /* A long label wraps on a phone rather than leave the sheet. */
      .opens .btn {
        max-width: 100%;
        white-space: normal;
        text-align: left;
      }
    `,
  ],
})
export class SignInPanelComponent {
  private api = inject(ApiService);
  private accounts = inject(AccountsService);

  readonly account = input.required<AccountView>();
  /** The sign-in stopped running: how it ended. */
  readonly finished = output<SignInState>();

  private readonly codeInput = viewChild<ElementRef<HTMLInputElement>>('codeInput');

  /** The sign-in as the desktop's own read says it (with its link). */
  private readonly own = signal<SignInView | null>(null);
  protected readonly browsers = signal<SignInBrowser[]>(['default']);
  protected readonly busy = signal(false);
  protected readonly error = signal('');
  protected readonly code = signal('');

  private readonly accountId = computed(() => this.account().id);
  /** Ours, moved on by what the frames say. */
  protected readonly run = computed(() => {
    const r = mergeSignIn(this.own(), this.account().signIn);
    return r && signInRunning(r) ? r : null;
  });
  protected readonly who = computed(() => {
    const a = this.account();
    return a.isDefault ? 'this Mac uses' : `for ${a.name}`;
  });
  protected readonly privateOffered = computed(() => this.browsers().includes('chrome-private'));
  /** A token sign-in: the private window first, when the Mac has it. */
  protected readonly tokenBrowsers = computed(() => {
    const list = this.browsers();
    return [
      ...list.filter((b) => b === 'chrome-private'),
      ...list.filter((b) => b !== 'chrome-private'),
    ];
  });

  constructor() {
    // Read the account (its link) now and every little while as it runs.
    effect((onCleanup) => {
      const id = this.accountId();
      const running = signInRunning(mergeSignIn(this.own(), this.account().signIn));
      if (!running) return;
      untracked(() => void this.load(id));
      const timer = setInterval(() => void this.load(id), POLL_MS);
      onCleanup(() => clearInterval(timer));
    });
    // The browsers the link may open in, once there is a link.
    let browsersFor = '';
    effect(() => {
      const r = this.run();
      const id = this.accountId();
      if (!r?.url || browsersFor === id) return;
      browsersFor = id;
      untracked(() => {
        this.api.getSignInBrowsers(id).then(
          (raw) => this.browsers.set(normalizeBrowsers(raw)),
          () => undefined,
        );
      });
    });
    // A frame later than our read: read again at once for its link (a run
    // that starts over shows a new one; frames never carry it).
    effect(() => {
      const frame = this.account().signIn;
      const own = untracked(() => this.own());
      if (!frame || !signInRunning(frame) || frame.rev === undefined) return;
      if (own?.startedAt === frame.startedAt && (own.rev ?? -1) >= frame.rev) return;
      untracked(() => void this.load(this.accountId()));
    });
    // The field takes the focus when the CLI asks for the code (once per
    // ask, once the field is there).
    let asked = '';
    effect(() => {
      const r = this.run();
      const input = this.codeInput()?.nativeElement;
      const now = r?.state === 'code' && !r.codeSent ? `${r.startedAt}:${r.url ?? ''}` : '';
      if (!now) asked = '';
      if (!now || !input || now === asked) return;
      asked = now;
      if (document.activeElement !== input) input.focus();
    });
    // Tell the host once it stops running.
    let wasRunning = false;
    effect(() => {
      const r = mergeSignIn(this.own(), this.account().signIn);
      const running = signInRunning(r);
      if (wasRunning && !running && r) untracked(() => this.finished.emit(r.state));
      wasRunning = running;
    });
  }

  protected label(b: SignInBrowser): string {
    return browserLabel(b);
  }

  /** "your default browser", "Chrome", "a private Chrome window". */
  protected where(b: SignInBrowser): string {
    if (b === 'chrome') return 'Chrome';
    return b === 'chrome-private' ? 'a private Chrome window' : 'your default browser';
  }

  protected async openIn(browser: SignInBrowser): Promise<void> {
    if (this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      this.take(await this.api.openSignIn(this.accountId(), browser));
    } catch (e) {
      this.error.set(signInErrorText(e));
    } finally {
      this.busy.set(false);
    }
  }

  protected async sendCode(e: Event): Promise<void> {
    e.preventDefault();
    const code = this.code().trim();
    if (!code || this.busy()) return;
    this.error.set('');
    if (looksLikeToken(code)) {
      // Never sent as a code, nor kept in the field.
      this.code.set('');
      const kind = this.run()?.kind;
      if (kind === 'token' && isLongLivedToken(code)) {
        await this.keepToken(code);
      } else {
        this.error.set(
          kind === 'token'
            ? "That's a key, not the code or a token from claude setup-token: nothing was sent."
            : "That's a token, not the code: nothing was sent. Paste the code Claude shows after you approve in the browser.",
        );
      }
      return;
    }
    this.busy.set(true);
    try {
      this.take(await this.api.sendSignInCode(this.accountId(), code));
      this.code.set('');
    } catch (err) {
      this.error.set(signInErrorText(err));
    } finally {
      this.busy.set(false);
    }
  }

  /** A long-lived token pasted where the code goes: the account's token
   * (checked, refused when another account holds it); the sign-in, which
   * has what it was for, ends. */
  private async keepToken(token: string): Promise<void> {
    const id = this.accountId();
    this.busy.set(true);
    try {
      await this.api.pasteAccountToken(id, token);
      await this.load(id);
    } catch (err) {
      this.error.set(describeError(err).text);
    } finally {
      this.busy.set(false);
    }
  }

  protected async cancel(): Promise<void> {
    this.busy.set(true);
    this.error.set('');
    try {
      this.take(await this.api.cancelSignIn(this.accountId()));
      await this.accounts.refresh();
    } catch (e) {
      this.error.set(signInErrorText(e));
    } finally {
      this.busy.set(false);
    }
  }

  /** An answer's `{signIn}` (open, code and cancel send it back). */
  private take(res: unknown): void {
    const raw = res && typeof res === 'object' ? (res as Record<string, unknown>)['signIn'] : null;
    const run = normalizeSignIn(raw);
    if (run) this.own.set(advanceSignIn(this.own(), run));
  }

  private async load(id: string): Promise<void> {
    try {
      const view = await this.api.getAccount(id);
      if (id !== this.accountId()) return;
      const r = normalizeSignIn((view as unknown as Record<string, unknown> | null)?.['signIn']);
      this.own.set(r ? advanceSignIn(this.own(), r) : null);
    } catch {
      // The account view still says how it goes.
    }
  }
}
