import { Injectable, computed, inject, signal } from '@angular/core';

import { ApiError, ApiService, errorMessage } from './api.service';
import { ConfigService } from './config.service';
import { PushPrefs, SessionPushPref } from './models';
import { isIos, isStandalone, isTauri } from './platform';

export type PushState = 'unsupported' | 'checking' | 'off' | 'on' | 'denied' | 'working' | 'error';

/**
 * Web Push for phones (design contract §6). Only possible in a secure context
 * outside the desktop webview: the tunnel's trusted HTTPS origin, or the LAN
 * address once the phone trusts the Mac's certificate (Pair page › Trust
 * this Mac). Until then those phones get the in-app alerts instead
 * (AttentionService).
 *
 * - `register()` registers /sw.js (push + notificationclick only, no cache).
 * - `enable()` must run from a user gesture (iOS requirement).
 */
@Injectable({ providedIn: 'root' })
export class PushService {
  private api = inject(ApiService);
  private config = inject(ConfigService);

  readonly state = signal<PushState>('checking');
  readonly error = signal('');

  /** Why push can't work here (empty when it can). */
  readonly unsupportedReason = computed(() => {
    if (this.config.desktop() || isTauri()) {
      return 'The desktop app shows everything live; push notifications are for your phone.';
    }
    if (!window.isSecureContext) {
      return "Push needs a trusted HTTPS address. On the local network, trust the Mac's certificate first (Pair page › Trust this Mac), or open Mother Claude through your internet address (ngrok, Cloudflare, Tailscale). Until then you get in-app alerts while the app is open.";
    }
    if (isIos() && !isStandalone()) {
      return 'On iPhone and iPad, add Mother Claude to your Home Screen first (Share → Add to Home Screen), open it from there, pair it, then enable push.';
    }
    if (
      !('serviceWorker' in navigator) ||
      !('PushManager' in window) ||
      !('Notification' in window)
    ) {
      return 'This browser does not support push notifications.';
    }
    return '';
  });

  readonly supported = computed(() => !this.unsupportedReason());

  /** The device's notification prefs (loaded on demand). */
  readonly prefs = signal<PushPrefs | null>(null);

  private registration: ServiceWorkerRegistration | null = null;

  async loadPrefs(): Promise<PushPrefs | null> {
    try {
      const p = await this.api.getPushPrefs();
      this.prefs.set(p);
      return p;
    } catch {
      return this.prefs();
    }
  }

  /** This session's follow/mute choice ('' = the per-event defaults). */
  sessionPref(id: string): SessionPushPref | '' {
    return this.prefs()?.sessions?.[id] ?? '';
  }

  /** Follow (always notify), mute, or back to the defaults (null). */
  async setSessionPref(id: string, pref: SessionPushPref | null): Promise<void> {
    const prev = this.prefs();
    const sessions = { ...(prev?.sessions ?? {}) };
    if (pref) sessions[id] = pref;
    else delete sessions[id];
    if (prev) this.prefs.set({ ...prev, sessions });
    try {
      const next = await this.api.setSessionPush(id, pref);
      if (next) this.prefs.set(next);
    } catch (e) {
      this.prefs.set(prev);
      throw e;
    }
  }

  /** Register the service worker (secure context, not Tauri, not mock). */
  async register(): Promise<void> {
    const cfg = await this.config.ensure();
    if (!this.supported() || cfg.mock) {
      this.state.set(this.supported() ? 'off' : 'unsupported');
      return;
    }
    try {
      this.registration = await navigator.serviceWorker.register('sw.js', { scope: './' });
      await this.refresh();
    } catch (e) {
      this.registration = null;
      this.state.set('error');
      this.error.set(`Service worker unavailable: ${errorMessage(e)}`);
    }
  }

  /** Re-read the browser's permission + subscription. */
  async refresh(): Promise<void> {
    if (!this.supported()) {
      this.state.set('unsupported');
      return;
    }
    if (Notification.permission === 'denied') {
      this.state.set('denied');
      return;
    }
    const sub = await this.registration?.pushManager.getSubscription().catch(() => null);
    this.state.set(sub ? 'on' : 'off');
  }

  /** Ask for permission, subscribe, and register the subscription. */
  async enable(): Promise<void> {
    this.error.set('');
    if (!this.supported()) return;
    this.state.set('working');
    try {
      const permission = await Notification.requestPermission();
      if (permission !== 'granted') {
        this.state.set(permission === 'denied' ? 'denied' : 'off');
        return;
      }
      if (!this.registration) {
        this.registration = await navigator.serviceWorker.register('sw.js', { scope: './' });
      }
      await navigator.serviceWorker.ready;
      const { publicKey } = await this.api.getVapidKey();
      let sub = await this.registration.pushManager.getSubscription();
      // A subscription made with an older server key can't be used: renew it.
      if (sub && !sameKey(sub.options.applicationServerKey, publicKey)) {
        await sub.unsubscribe().catch(() => false);
        sub = null;
      }
      try {
        await this.send(sub ?? (await this.subscribe(publicKey)), publicKey);
      } catch (e) {
        if (!(e instanceof ApiError && e.code === 'vapid_key_mismatch')) throw e;
        // The server's key changed since: subscribe again with the current one.
        const fresh = await this.api.getVapidKey();
        const old = await this.registration.pushManager.getSubscription();
        await old?.unsubscribe().catch(() => false);
        await this.send(await this.subscribe(fresh.publicKey), fresh.publicKey);
      }
      this.state.set('on');
    } catch (e) {
      this.state.set('error');
      this.error.set(errorMessage(e));
    }
  }

  private subscribe(publicKey: string): Promise<PushSubscription> {
    if (!this.registration) throw new Error('The service worker is not registered.');
    return this.registration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: base64UrlToBytes(publicKey),
    });
  }

  private async send(sub: PushSubscription, publicKey: string): Promise<void> {
    const json = sub.toJSON();
    if (!json.endpoint || !json.keys?.['p256dh'] || !json.keys['auth']) {
      throw new Error('The browser returned an incomplete push subscription.');
    }
    await this.api.pushSubscribe({
      endpoint: json.endpoint,
      keys: { p256dh: json.keys['p256dh'], auth: json.keys['auth'] },
      applicationServerKey: publicKey,
    });
  }

  /** Arm or disarm "notify me when this turn finishes" for one session. */
  async setTurnFinished(id: string, on: boolean): Promise<void> {
    const next = await this.api.setTurnFinishedPush(id, on);
    if (next) this.prefs.set(next);
  }

  /** Whether a one-shot "turn finished" push is armed for the session. */
  turnFinishedArmed(id: string): boolean {
    return this.prefs()?.turnFinished?.includes(id) ?? false;
  }

  async disable(): Promise<void> {
    this.error.set('');
    this.state.set('working');
    try {
      const sub = await this.registration?.pushManager.getSubscription();
      if (sub) {
        await this.api.pushUnsubscribe(sub.endpoint).catch(() => undefined);
        await sub.unsubscribe();
      }
      this.state.set('off');
    } catch (e) {
      this.state.set('error');
      this.error.set(errorMessage(e));
    }
  }
}

/** Does the subscription's key equal the server's (base64url) key? */
function sameKey(key: ArrayBuffer | null | undefined, b64url: string): boolean {
  if (!key) return true; // the browser doesn't say: let the server decide
  const want = base64UrlToBytes(b64url);
  const have = new Uint8Array(key);
  return have.length === want.length && have.every((b, i) => b === want[i]);
}

function base64UrlToBytes(b64url: string): Uint8Array<ArrayBuffer> {
  const pad = '='.repeat((4 - (b64url.length % 4)) % 4);
  const b64 = (b64url + pad).replace(/-/g, '+').replace(/_/g, '/');
  const raw = atob(b64);
  const out = new Uint8Array(new ArrayBuffer(raw.length));
  for (let i = 0; i < raw.length; i++) out[i] = raw.charCodeAt(i);
  return out;
}
