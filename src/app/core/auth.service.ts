import { Injectable, computed, inject, signal } from '@angular/core';
import { Observable, Subject } from 'rxjs';

import { ConfigService } from './config.service';
import { MeInfo } from './models';
import { HostOs, hostOsFromUserAgent, hostOsOf } from './paths';

export type PairState = 'unknown' | 'paired' | 'unpaired';

/** Why this browser is not paired: it never was, or the server rejected its
 * token (revoked on the desktop, or the desktop's device list was reset). */
export type UnpairedReason = 'no-token' | 'revoked';

/**
 * Who this client is to the server, and whether it is paired at all.
 *
 * - `state` becomes `unpaired` when there is no token, and when the server
 *   answers 401 or closes the WebSocket with 4401 (the shell then routes to
 *   /pair with "This device is no longer paired").
 * - `me` comes from `GET /api/me` and every WS `hello` frame.
 * - The desktop webview is never "unpaired": its token belongs to the app.
 */
@Injectable({ providedIn: 'root' })
export class AuthService {
  private config = inject(ConfigService);

  readonly state = signal<PairState>('unknown');
  readonly reason = signal<UnpairedReason | null>(null);
  readonly me = signal<MeInfo | null>(null);
  /** Set when the desktop's own token was rejected (backend mismatch). */
  readonly desktopRejected = signal(false);

  /** Desktop principal: the Tauri webview (or whatever `me` says). */
  readonly isDesktop = computed(() => this.config.desktop() || this.me()?.principal === 'desktop');
  /** May take dangerous actions: the server decides by principal (the
   * desktop, or a device granted "allow dangerous" for the network it is on)
   * and reports it as `me.allowDangerous`. Being local grants nothing else. */
  readonly canDangerous = computed(() => {
    const me = this.me();
    if (!me) return this.config.desktop();
    return me.allowDangerous;
  });
  /** The desktop's system, whichever device this is (a phone may drive a
   * Windows PC): what folder hints and examples follow. Until the server
   * says, the desktop webview's own; elsewhere a Mac's. */
  readonly hostOs = computed<HostOs>(
    () =>
      hostOsOf(this.me()?.hostOs) ??
      (this.config.desktop() ? hostOsFromUserAgent(navigator.userAgent) : undefined) ??
      'macos',
  );

  private readonly unauthorizedS = new Subject<void>();
  /** Emits every time the server rejects our credential. */
  readonly unauthorized$: Observable<void> = this.unauthorizedS.asObservable();

  /** Decide the initial state from the resolved config. */
  async init(): Promise<void> {
    const cfg = await this.config.ensure();
    if (cfg.token || cfg.desktop) {
      this.state.set('paired');
      this.reason.set(null);
    } else {
      this.state.set('unpaired');
      this.reason.set('no-token');
    }
  }

  /** The server rejected our token (HTTP 401 / WS close 4401). */
  handleUnauthorized(): void {
    this.unauthorizedS.next();
    if (this.config.desktop()) {
      this.desktopRejected.set(true);
      return;
    }
    if (this.state() === 'unpaired') return;
    this.config.clearToken();
    this.me.set(null);
    this.reason.set('revoked');
    this.state.set('unpaired');
  }

  /** Pairing succeeded: keep the device token and flip to paired. */
  completePairing(token: string): void {
    this.config.setToken(token);
    this.reason.set(null);
    this.state.set('paired');
  }

  /** "Forget this device" on a phone. */
  forget(): void {
    this.config.clearToken();
    this.me.set(null);
    this.reason.set('no-token');
    this.state.set('unpaired');
  }
}
