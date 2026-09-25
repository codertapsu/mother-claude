import { Component, DestroyRef, computed, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { Router, RouterLink } from '@angular/router';

import { ApiError, ApiService, errorMessage } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { describeDevice, isIos, isStandalone, urlParam } from '../../core/platform';
import { IconComponent } from '../../shared/icon.component';

type Phase = 'enter' | 'exchanging' | 'waiting' | 'done' | 'error';

const POLL_MS = 2000;

/** Crockford base32: uppercase, no separators, O→0 and I/L→1. */
export function normalizePairCode(raw: string): string {
  return raw
    .toUpperCase()
    .replace(/[^0-9A-Z]/g, '')
    .replace(/O/g, '0')
    .replace(/[IL]/g, '1');
}

/** A private-network address (RFC 1918, link-local, *.local). */
function isLanOrigin(host: string): boolean {
  return (
    /^10\./.test(host) ||
    /^192\.168\./.test(host) ||
    /^172\.(1[6-9]|2\d|3[01])\./.test(host) ||
    /^169\.254\./.test(host) ||
    host.endsWith('.local')
  );
}

/** "ABCD1234WXYZ" → "ABCD-1234-WXYZ" */
function groupCode(code: string): string {
  return code.replace(/(.{4})(?=.)/g, '$1-');
}

/**
 * Phone entry point. A QR code opens `#/pair?c=CODE`: the code is read and the
 * fragment dropped from the address bar/history right away, then exchanged
 * for a device token. Internet pairings may need approval on the desktop
 * (202): the phone polls and shows the 4-digit fingerprint to compare. A
 * typed code works too — needed by iOS Home Screen apps, which don't share
 * Safari's storage.
 */
@Component({
  selector: 'mc-pair',
  imports: [FormsModule, RouterLink, IconComponent],
  template: `
    <div class="pair">
      <div class="hero">
        <span class="logo" aria-hidden="true"></span>
        <h1>{{ isDesktop() ? 'Pair a phone' : 'Pair this device' }}</h1>
      </div>

      @if (revoked() && phase() === 'enter') {
        <p class="notice warn" role="alert">
          <strong>This device is no longer paired.</strong> It was removed on the desktop (or its
          pairing expired). Pair it again with a new code.
        </p>
      }

      @if (isDesktop()) {
        <div class="card">
          <p>
            This is the desktop app. To connect a phone, open <strong>Remote</strong>, choose LAN or
            Internet, and scan the QR code with the phone's camera.
          </p>
          <a class="btn primary" routerLink="/remote"><mc-icon name="qr" /> Open Remote</a>
        </div>
      } @else if (alreadyPaired() && phase() === 'enter') {
        <div class="card">
          <p><mc-icon name="check" /> This device is paired with Mother Claude.</p>
          <div class="row">
            <a class="btn primary" routerLink="/">Open sessions</a>
            <button class="btn" type="button" (click)="forget()">Forget this device</button>
          </div>
        </div>
      } @else {
        @switch (phase()) {
          @case ('exchanging') {
            <div class="card center" role="status">
              <span class="spinner" aria-hidden="true"></span>
              <p>Pairing…</p>
            </div>
          }
          @case ('waiting') {
            <div class="card center" role="status">
              <p class="big-label">Approve this device on your Mac</p>
              <p class="muted">On the Mac, type this number to approve it:</p>
              <p
                class="fingerprint"
                [attr.aria-label]="'Number: ' + fingerprint().split('').join(' ')"
              >
                {{ fingerprint() }}
              </p>
              <p class="muted small">Waiting for approval… (this page updates by itself)</p>
              <button class="btn" type="button" (click)="cancel()">Cancel</button>
            </div>
          }
          @case ('done') {
            <div class="card center" role="status">
              <p class="big-label"><mc-icon name="check" /> Paired</p>
              <p class="muted">Opening your sessions…</p>
            </div>
          }
          @default {
            @if (phase() === 'error') {
              <p class="notice error" role="alert">{{ error() }}</p>
            }
            <div class="card">
              <ol class="steps">
                <li>On your computer, open <strong>Mother Claude → Remote</strong>.</li>
                <li>
                  Scan the QR code with this device's camera — or type the code shown under it here.
                </li>
              </ol>
              @if (homeScreenApp()) {
                <p class="notice small">
                  This Home Screen app keeps its own storage, separate from Safari, so it needs its
                  own pairing. In a paired Safari tab open Settings › Set up the Home Screen app (or
                  create a code on the Mac), then type the code below.
                </p>
              }
              <form class="form" (submit)="submit($event)">
                <label class="field">
                  <span class="field-label">Pairing code</span>
                  <input
                    class="code"
                    name="code"
                    [ngModel]="code()"
                    (ngModelChange)="code.set($event)"
                    placeholder="XXXX-XXXX-XXXX"
                    autocomplete="one-time-code"
                    autocapitalize="characters"
                    spellcheck="false"
                    inputmode="text"
                    maxlength="20"
                  />
                </label>
                <label class="field">
                  <span class="field-label">Name for this device</span>
                  <input
                    name="deviceName"
                    [ngModel]="deviceName()"
                    (ngModelChange)="deviceName.set($event)"
                    maxlength="60"
                  />
                </label>
                <button class="btn primary block" type="submit" [disabled]="!codeValid()">
                  <mc-icon name="key" /> Pair
                </button>
              </form>
            </div>
            <p class="muted small foot">
              Codes work once and expire after 5 minutes. Pairing stores a device key in this
              browser only; the desktop can revoke it at any time.
            </p>
            @if (lan()) {
              <details class="card trust">
                <summary>
                  <mc-icon name="shield" [size]="16" /> Trust this Mac (recommended)
                </summary>
                <p class="small">
                  On your home network the Mac uses its own certificate, so the browser warns once
                  ("Not private"). Until the phone trusts it, live updates may fall back to
                  refreshing every few seconds and notifications stay off.
                </p>
                @if (caAvailable()) {
                  <ol class="steps small">
                    @if (ios()) {
                      <li>
                        <a href="/api/pair/ca.mobileconfig">Download the profile</a>, then open
                        Settings › General › VPN &amp; Device Management and install it.
                      </li>
                      <li>
                        Settings › General › About › Certificate Trust Settings: turn on full trust
                        for “Mother Claude”.
                      </li>
                    } @else {
                      <li>
                        <a href="/api/pair/ca.crt" download>Download the Mac's certificate</a> and
                        open it to install it (Android: Settings › Security › Install a certificate
                        › CA certificate).
                      </li>
                    }
                    <li>Reload this page.</li>
                  </ol>
                } @else {
                  <p class="small">
                    Tap “Show Details” › “visit this website” on the warning once. If live updates
                    still don't connect, the app keeps working by refreshing every few seconds.
                  </p>
                }
              </details>
            }
          }
        }
      }
    </div>
  `,
  styles: [
    `
      .pair {
        max-width: 28rem;
        margin: 0 auto;
      }
      .hero {
        display: flex;
        align-items: center;
        gap: 0.65rem;
        margin: 0.5rem 0 1rem;
      }
      .hero h1 {
        margin: 0;
      }
      .logo {
        width: 1rem;
        height: 1rem;
        border-radius: 50%;
        background: var(--accent);
        box-shadow: 0 0 0.8rem var(--accent);
      }
      .notice {
        margin: 0 0 0.75rem;
      }
      .steps {
        margin: 0 0 1rem;
        padding-left: 1.2rem;
        line-height: 1.55;
      }
      .form {
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
      }
      .code {
        font-family: var(--mono);
        font-size: 1.35rem;
        letter-spacing: 0.12em;
        text-transform: uppercase;
        text-align: center;
      }
      .center {
        text-align: center;
      }
      .big-label {
        font-size: 1.1rem;
        font-weight: 600;
        display: inline-flex;
        align-items: center;
        gap: 0.4rem;
      }
      .fingerprint {
        font-family: var(--mono);
        font-size: 2.6rem;
        font-weight: 700;
        letter-spacing: 0.3em;
        margin: 0.5rem 0;
        color: var(--accent-text);
      }
      .spinner {
        display: inline-block;
        width: 1.8rem;
        height: 1.8rem;
        border-radius: 50%;
        border: 3px solid var(--border-strong);
        border-top-color: var(--accent);
        animation: spin 0.8s linear infinite;
      }
      @keyframes spin {
        to {
          transform: rotate(360deg);
        }
      }
      .foot {
        margin-top: 0.75rem;
      }
      .trust {
        margin-top: 1rem;
      }
      .trust summary {
        display: flex;
        align-items: center;
        gap: 0.4rem;
        min-height: var(--tap);
        cursor: pointer;
        font-weight: 600;
      }
    `,
  ],
})
export class PairComponent {
  private api = inject(ApiService);
  private auth = inject(AuthService);
  private router = inject(Router);

  protected readonly isDesktop = this.auth.isDesktop;
  protected readonly phase = signal<Phase>('enter');
  protected readonly code = signal('');
  protected readonly deviceName = signal(describeDevice().name);
  protected readonly error = signal('');
  protected readonly fingerprint = signal('');
  protected readonly revoked = signal(urlParam('reason') === 'revoked');
  protected readonly homeScreenApp = signal(isIos() && isStandalone());
  protected readonly alreadyPaired = computed(() => this.auth.state() === 'paired');
  protected readonly codeValid = computed(() => normalizePairCode(this.code()).length >= 8);
  /** Opened over the LAN address (self-signed certificate). */
  protected readonly lan = signal(isLanOrigin(location.hostname) && location.protocol === 'https:');
  protected readonly ios = signal(isIos());
  /** The server offers its local CA for download (`/api/pair/ca.crt`). */
  protected readonly caAvailable = signal(false);

  private pollTimer?: ReturnType<typeof setTimeout>;
  private attempt = 0;

  constructor() {
    inject(DestroyRef).onDestroy(() => clearTimeout(this.pollTimer));
    if (this.lan()) void this.probeCa();
    const fromQr = urlParam('c');
    if (fromQr) {
      // The code must not linger in the address bar or history.
      try {
        history.replaceState(history.state, '', `${location.pathname}${location.search}#/pair`);
      } catch {
        /* best effort */
      }
      if (!this.isDesktop()) {
        this.code.set(groupCode(normalizePairCode(fromQr)));
        void this.exchange(normalizePairCode(fromQr));
      }
    }
  }

  private async probeCa(): Promise<void> {
    try {
      const res = await fetch('/api/pair/ca.crt', { method: 'HEAD', cache: 'no-store' });
      const type = res.headers.get('content-type') ?? '';
      this.caAvailable.set(res.ok && !type.includes('text/html'));
    } catch {
      this.caAvailable.set(false);
    }
  }

  protected submit(event: Event): void {
    event.preventDefault();
    if (!this.codeValid()) return;
    void this.exchange(normalizePairCode(this.code()));
  }

  protected cancel(): void {
    this.attempt += 1; // invalidates the running poll
    clearTimeout(this.pollTimer);
    this.phase.set('enter');
  }

  protected forget(): void {
    this.auth.forget();
  }

  private async exchange(code: string): Promise<void> {
    const attempt = ++this.attempt;
    this.phase.set('exchanging');
    this.error.set('');
    const { platform } = describeDevice();
    try {
      const res = await this.api.exchangePairCode({
        code,
        deviceName: this.deviceName().trim() || describeDevice().name,
        platform,
      });
      if (attempt !== this.attempt) return;
      if (res.status === 'paired') {
        this.finish(res.token);
      } else {
        this.fingerprint.set(res.fingerprint);
        this.phase.set('waiting');
        this.poll(res.pendingId, res.pollSecret, attempt);
      }
    } catch (e) {
      if (attempt === this.attempt) this.fail(e);
    }
  }

  private poll(pendingId: string, secret: string, attempt: number): void {
    clearTimeout(this.pollTimer);
    this.pollTimer = setTimeout(async () => {
      if (attempt !== this.attempt) return;
      try {
        const st = await this.api.pollPairPending(pendingId, secret);
        if (attempt !== this.attempt) return;
        switch (st.status) {
          case 'approved':
            if (st.token) this.finish(st.token);
            else this.fail(new Error('Approved, but the server sent no device key. Try again.'));
            return;
          case 'denied':
            this.fail(new Error('The desktop declined this device.'));
            return;
          case 'expired':
            this.fail(new Error('The request expired before it was approved. Scan a new code.'));
            return;
          case 'contested':
            this.fail(
              new Error(
                st.message ||
                  'This code was used by another device too, so it cannot be approved. Ask for a new code on your Mac.',
              ),
            );
            return;
          default:
            this.poll(pendingId, secret, attempt);
        }
      } catch (e) {
        if (attempt !== this.attempt) return;
        // Transient network trouble: keep waiting; a hard error ends it.
        if (e instanceof ApiError && e.kind === 'network') this.poll(pendingId, secret, attempt);
        else this.fail(e);
      }
    }, POLL_MS);
  }

  private finish(token: string): void {
    this.phase.set('done');
    this.revoked.set(false);
    this.auth.completePairing(token);
    void this.router.navigate(['/'], { replaceUrl: true });
  }

  private fail(e: unknown): void {
    this.phase.set('error');
    if (e instanceof ApiError) {
      if (e.status === 429) {
        this.error.set(
          `Too many attempts. Wait ${e.retryAfter ? `${e.retryAfter} s` : 'a minute'}, then try again.`,
        );
      } else if (e.code === 'wrong_network' || e.code === 'code_already_used') {
        this.error.set(e.message);
      } else if ([400, 403, 404, 409, 410, 422].includes(e.status)) {
        this.error.set(
          'That code is not valid. Codes work once and expire after 5 minutes — create a new one on the desktop.',
        );
      } else {
        this.error.set(errorMessage(e));
      }
    } else {
      this.error.set(errorMessage(e));
    }
  }
}
