import { Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DomSanitizer, SafeHtml } from '@angular/platform-browser';

import { ApiService } from '../core/api.service';
import { ConfigService } from '../core/config.service';
import { Pairing } from '../core/models';

/** Reusable "connect a device" UI: QR + addresses + fingerprint + manual token.
 * Shared by the Settings screen and the dedicated Pair screen. */
@Component({
  selector: 'mc-pairing',
  imports: [FormsModule],
  template: `
    <div class="card pair">
      @if (qr(); as svg) {
        <div class="qr" [innerHTML]="svg"></div>
      }
      @if (pairing(); as p) {
        <div class="info">
          <div class="mono url">{{ p.url }}</div>
          <div class="muted">Addresses: {{ p.addresses.join(', ') || 'localhost' }}</div>
          <div class="muted">TLS: {{ p.tls ? 'on' : 'off (loopback)' }}</div>
          @if (p.tls) {
            <div class="muted fp">Fingerprint: {{ p.fingerprint }}</div>
          }
          <p class="muted small">
            Scan on the same Wi-Fi. With a self-signed cert your phone warns once — verify the
            fingerprint, then trust it.
          </p>
        </div>
      } @else if (error()) {
        <p class="muted">{{ error() }}</p>
      }
    </div>

    <div class="card">
      <div class="row token-row">
        <input
          [ngModel]="tokenInput()"
          (ngModelChange)="tokenInput.set($event)"
          placeholder="…or paste an API token"
        />
        <button class="btn primary" (click)="applyToken()">Save &amp; reload</button>
      </div>
      @if (cfg(); as c) {
        <p class="muted small">This device: {{ c.token ? 'paired' : 'not paired yet' }}.</p>
      }
    </div>
  `,
  styles: [
    `
      .card {
        margin-bottom: 0.5rem;
      }
      .pair {
        display: flex;
        gap: 1rem;
        align-items: flex-start;
        flex-wrap: wrap;
      }
      .qr {
        width: 220px;
        height: 220px;
        max-width: 100%;
        background: #fff;
        border-radius: 0.5rem;
        padding: 0.4rem;
        flex: 0 0 auto;
      }
      .qr ::ng-deep svg {
        width: 100%;
        height: 100%;
        display: block;
      }
      .info {
        flex: 1;
        min-width: 12rem;
      }
      .info .url,
      .info .fp {
        word-break: break-all;
      }
      .info .fp {
        font-size: 0.78rem;
      }
      .small {
        font-size: 0.8rem;
      }
      .token-row {
        gap: 0.5rem;
      }
      .token-row input {
        min-width: 0;
      }
    `,
  ],
})
export class PairingComponent {
  private api = inject(ApiService);
  private config = inject(ConfigService);
  private sanitizer = inject(DomSanitizer);

  protected readonly cfg = this.config.config;
  protected readonly pairing = signal<Pairing | null>(null);
  protected readonly qr = signal<SafeHtml | null>(null);
  protected readonly tokenInput = signal('');
  protected readonly error = signal('');

  constructor() {
    this.loadPairing();
  }

  private async loadPairing(): Promise<void> {
    try {
      const p = await this.api.getPairing();
      this.pairing.set(p);
      this.qr.set(this.sanitizer.bypassSecurityTrustHtml(p.svg));
    } catch (e) {
      this.error.set(`Pairing unavailable (need a valid token first): ${(e as Error).message}`);
    }
  }

  applyToken(): void {
    const t = this.tokenInput().trim();
    if (!t) return;
    this.config.setToken(t);
    this.tokenInput.set('');
    location.reload();
  }
}
