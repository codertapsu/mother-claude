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

import { normalizeCapture } from '../../core/accounts';
import { AccountsService } from '../../core/accounts.service';
import { ApiError, ApiService, describeError } from '../../core/api.service';
import { AccountView, CaptureView } from '../../core/models';
import { captureProblem, secretStoreText } from '../../shared/accounts';
import { CopyButtonComponent } from '../../shared/copy-button.component';
import { IconComponent } from '../../shared/icon.component';

/** A running capture is asked about this often (frames carry its state but
 * not its sign-in link). */
const POLL_MS = 1_500;

/**
 * Desktop: get a token account its token. First choice: the Mac runs
 * `claude setup-token` and takes the token it shows (the browser opens on
 * the Mac; a "Browser didn't open?" link and, when the terminal asks for
 * it, a field for the code the browser shows). Always offered, and shown
 * outright when the run can't happen or brought nothing: paste a token
 * made elsewhere. A pasted token is read from the field once, the field is
 * cleared, and the token travels only in that one request's body.
 */
@Component({
  selector: 'mc-account-token',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, CopyButtonComponent, IconComponent],
  template: `
    @let a = account();
    @if (capture(); as cap) {
      @if (cap.state === 'running') {
        <div class="flow" role="status">
          <p class="wait">
            <span class="spin" aria-hidden="true"></span>
            Waiting for the browser…
          </p>
          <p class="muted small">
            Sign in there with the Claude account this token is for. The token goes straight to
            {{ place() }}.
          </p>
          @if (cap.url; as url) {
            <p class="small link">
              Browser didn't open?
              <a [href]="url" target="_blank" rel="noopener noreferrer">Open this link</a>
              <mc-copy-button [text]="url" label="Copy link" />
            </p>
          }
          @if (cap.asksForCode) {
            <form class="code" (submit)="sendCode($event)">
              <label class="field">
                <span class="field-label">Paste the code the browser shows</span>
                <input
                  name="code"
                  autocomplete="off"
                  spellcheck="false"
                  autocapitalize="off"
                  [ngModel]="code()"
                  (ngModelChange)="code.set($event)"
                />
              </label>
              <button class="btn primary small" type="submit" [disabled]="!code().trim() || busy()">
                Send the code
              </button>
            </form>
          }
          <div class="row">
            <button class="btn ghost small" type="button" [disabled]="busy()" (click)="cancel()">
              Cancel
            </button>
          </div>
        </div>
      }
    }

    @if (!running()) {
      @if (problem()) {
        <p class="notice warn small" role="status">{{ problem() }}</p>
      } @else if (justSaved()) {
        <p class="notice ok small" role="status">
          <mc-icon name="check" [size]="14" /> Token saved for {{ a.name }}.
        </p>
      } @else if (a.tokenStored) {
        <p class="small muted">A token for this account is stored on this Mac.</p>
      }

      @if (available()) {
        <div class="row">
          <button
            class="btn"
            type="button"
            [class.primary]="!a.tokenStored"
            [disabled]="busy()"
            (click)="start()"
          >
            <mc-icon name="key" [size]="15" />
            {{
              busy()
                ? 'Starting…'
                : a.tokenStored
                  ? 'Get a new token'
                  : capture()
                    ? 'Try again'
                    : 'Sign in to get a token'
            }}
          </button>
        </div>
        <p class="muted small">
          Opens the Claude sign-in in this Mac's browser (claude setup-token); the token it gives is
          kept in {{ place() }}.
        </p>
      } @else {
        <p class="muted small">
          This Mac can't run the token sign-in from here: run <code>claude setup-token</code> in a
          terminal and paste the token it shows.
        </p>
      }

      @if (pasteShown()) {
        <form class="paste" (submit)="paste($event)">
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
              (input)="tokenTyped.set(!!tok.value.trim())"
            />
          </label>
          <div class="row">
            <button class="btn primary small" type="submit" [disabled]="!tokenTyped() || busy()">
              {{ busy() ? 'Checking…' : 'Save the token' }}
            </button>
            @if (pasteOpen()) {
              <button class="btn ghost small" type="button" (click)="closePaste()">Cancel</button>
            }
          </div>
          <p class="field-hint">
            Claude Code checks it first; then it is kept in {{ place() }} and never shown again.
          </p>
        </form>
      } @else {
        <button class="btn ghost small paste-open" type="button" (click)="pasteOpen.set(true)">
          I already have a token — paste it
        </button>
      }
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
      .code,
      .paste {
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
      .spin {
        flex: 0 0 auto;
        width: 0.9rem;
        height: 0.9rem;
        border: 2px solid var(--border-strong);
        border-top-color: var(--accent);
        border-radius: 50%;
        animation: mc-spin 0.8s linear infinite;
      }
      .link {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.4rem;
      }
      .paste-open {
        align-self: flex-start;
      }
    `,
  ],
})
export class AccountTokenComponent {
  private api = inject(ApiService);
  private accounts = inject(AccountsService);

  readonly account = input.required<AccountView>();
  /** A token was stored (captured or pasted). */
  readonly done = output<void>();

  private readonly tokenInput = viewChild<ElementRef<HTMLInputElement>>('tok');

  /** The run as the desktop's own reads report it (with its link). */
  private readonly ownCapture = signal<CaptureView | null>(null);
  private readonly availableS = signal(true);
  protected readonly busy = signal(false);
  protected readonly error = signal('');
  protected readonly code = signal('');
  protected readonly pasteOpen = signal(false);
  protected readonly tokenTyped = signal(false);
  protected readonly justSaved = signal(false);

  private readonly accountId = computed(() => this.account().id);
  /** Ours (with the link), else what the account view says (no link). */
  protected readonly capture = computed(() => this.ownCapture() ?? this.account().capture ?? null);
  protected readonly running = computed(() => this.capture()?.state === 'running');
  protected readonly problem = computed(() => captureProblem(this.capture()));
  protected readonly available = computed(
    () => this.availableS() && this.accounts.view()?.tokenCapture !== false,
  );
  /** The paste field is offered outright when the run can't help. */
  protected readonly pasteShown = computed(() => {
    const state = this.capture()?.state;
    return this.pasteOpen() || !this.available() || state === 'failed' || state === 'timedOut';
  });
  protected readonly place = computed(
    () => secretStoreText(this.accounts.view()?.secretStore) || "the Mac's secret store",
  );

  constructor() {
    // A capture may already be running (started from another window).
    effect(() => {
      const id = this.accountId();
      untracked(() => {
        this.ownCapture.set(null);
        this.justSaved.set(false);
        void this.load(id);
      });
    });
    effect((onCleanup) => {
      if (!this.running()) return;
      const timer = setInterval(() => void this.load(this.accountId()), POLL_MS);
      onCleanup(() => clearInterval(timer));
    });
  }

  protected async start(): Promise<void> {
    if (this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    this.justSaved.set(false);
    const id = this.accountId();
    try {
      const res = await this.api.startTokenCapture(id);
      this.apply(id, res?.capture);
    } catch (e) {
      this.error.set(describeError(e).text);
      if (e instanceof ApiError && e.code === 'capture_failed') this.pasteOpen.set(true);
    } finally {
      this.busy.set(false);
    }
  }

  protected async sendCode(e: Event): Promise<void> {
    e.preventDefault();
    const code = this.code().trim();
    if (!code || this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    const id = this.accountId();
    try {
      const res = await this.api.sendTokenCode(id, code);
      this.code.set('');
      this.apply(id, res?.capture);
    } catch (err) {
      this.error.set(describeError(err).text);
    } finally {
      this.busy.set(false);
    }
  }

  protected async cancel(): Promise<void> {
    this.busy.set(true);
    const id = this.accountId();
    try {
      const res = await this.api.cancelTokenCapture(id);
      this.apply(id, res?.capture);
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(false);
    }
  }

  protected async paste(e: Event): Promise<void> {
    e.preventDefault();
    const field = this.tokenInput()?.nativeElement;
    const token = field?.value.trim() ?? '';
    // Out of the form at once: the token lives only in this request.
    if (field) field.value = '';
    this.tokenTyped.set(false);
    if (!token || this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      await this.api.pasteAccountToken(this.accountId(), token);
      this.pasteOpen.set(false);
      this.ownCapture.set(null);
      this.justSaved.set(true);
      await this.accounts.refresh();
      this.done.emit();
    } catch (err) {
      this.error.set(describeError(err).text);
    } finally {
      this.busy.set(false);
    }
  }

  protected closePaste(): void {
    const field = this.tokenInput()?.nativeElement;
    if (field) field.value = '';
    this.tokenTyped.set(false);
    this.pasteOpen.set(false);
  }

  private async load(id: string): Promise<void> {
    try {
      const status = await this.api.getAccountToken(id);
      if (id !== this.accountId()) return;
      this.availableS.set(status?.available !== false);
      this.apply(id, status?.capture);
    } catch {
      // The account view still says what is known.
    }
  }

  /** Take a capture from an answer; a token that just arrived is news. */
  private apply(id: string, raw: unknown): void {
    if (id !== this.accountId()) return;
    const next = normalizeCapture(raw) ?? null;
    const was = this.capture()?.state;
    this.ownCapture.set(next);
    if (next?.state === 'captured' && was === 'running') {
      this.justSaved.set(true);
      void this.accounts.refresh().then(() => this.done.emit());
    }
  }
}
