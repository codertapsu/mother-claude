import { Injectable, computed, signal } from '@angular/core';

import { mockRequested, mockRole, setActiveMock } from './mock/mock-api';
import { loadMockBackend } from './mock/mock-loader';
import { isTauri, storageGet, storageRemove, storageSet } from './platform';

export interface ResolvedConfig {
  /** Origin for REST calls, e.g. `https://192.168.1.5:6725`. */
  baseUrl: string;
  /** WebSocket URL. Never carries a token (auth is the first frame). */
  wsUrl: string;
  /** Bearer credential: the desktop token in the webview, a device token on
   * phones/browsers, or '' when this browser is not paired yet. */
  token: string;
  /** True inside the Tauri desktop webview. */
  desktop: boolean;
  /** Dev-only fixture backend is serving every request. */
  mock: boolean;
}

const TOKEN_KEY = 'mc_token';
const MOCK_TOKEN_KEY = 'mc_mock_token';

/**
 * Resolves how to reach the embedded server, both inside the Tauri desktop
 * webview (via the `server_info` invoke) and in a phone browser (same origin,
 * device token from localStorage). All dashboard data flows through the
 * HTTP/WS server in both cases — never Tauri `invoke`.
 *
 * Tokens never travel in URLs: pairing hands the browser a device token that
 * is kept in localStorage and sent as a Bearer header / WS first frame.
 */
@Injectable({ providedIn: 'root' })
export class ConfigService {
  readonly config = signal<ResolvedConfig | null>(null);
  readonly token = computed(() => this.config()?.token ?? '');
  readonly desktop = computed(() => this.config()?.desktop === true);
  readonly mock = computed(() => this.config()?.mock === true);
  private pending?: Promise<ResolvedConfig>;

  ensure(): Promise<ResolvedConfig> {
    const current = this.config();
    if (current) return Promise.resolve(current);
    if (!this.pending) this.pending = this.resolve();
    return this.pending;
  }

  /** Persist a device token obtained by pairing. */
  setToken(token: string): void {
    const current = this.config();
    storageSet(current?.mock ? MOCK_TOKEN_KEY : TOKEN_KEY, token);
    if (current) this.config.set({ ...current, token });
  }

  /** Forget this browser's device token (revoked, or "forget this device"). */
  clearToken(): void {
    const current = this.config();
    if (current?.desktop) return; // the desktop token is owned by the app
    storageRemove(current?.mock ? MOCK_TOKEN_KEY : TOKEN_KEY);
    if (current) this.config.set({ ...current, token: '' });
  }

  private async resolve(): Promise<ResolvedConfig> {
    this.stripLegacyTokenFromUrl();
    let resolved: ResolvedConfig;
    const mock = mockRequested() ? await loadMockBackend() : null;
    if (mock) {
      setActiveMock(mock);
      const origin = window.location.origin;
      const stored = storageGet(MOCK_TOKEN_KEY);
      resolved = {
        baseUrl: origin,
        wsUrl: this.buildWsUrl(origin),
        token: mockRole() === 'unpaired' ? (stored ?? '') : mock.token,
        desktop: false,
        mock: true,
      };
    } else if (isTauri()) {
      resolved = await this.resolveDesktop();
    } else {
      const baseUrl = window.location.origin;
      resolved = {
        baseUrl,
        wsUrl: this.buildWsUrl(baseUrl),
        token: storageGet(TOKEN_KEY) ?? '',
        desktop: false,
        mock: false,
      };
    }
    this.config.set(resolved);
    return resolved;
  }

  private async resolveDesktop(): Promise<ResolvedConfig> {
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const info = await invoke<{ host: string; port: number; scheme: string; token: string }>(
        'server_info',
      );
      const host = info.host === '0.0.0.0' || info.host === '::' ? 'localhost' : info.host;
      const baseUrl = `${info.scheme}://${host}:${info.port}`;
      return {
        baseUrl,
        token: info.token,
        wsUrl: this.buildWsUrl(baseUrl),
        desktop: true,
        mock: false,
      };
    } catch {
      // Fall back to the documented default if the invoke is unavailable.
      const baseUrl = 'http://localhost:6725';
      return {
        baseUrl,
        token: storageGet(TOKEN_KEY) ?? '',
        wsUrl: this.buildWsUrl(baseUrl),
        desktop: true,
        mock: false,
      };
    }
  }

  private buildWsUrl(baseUrl: string): string {
    return `${baseUrl.replace(/^http/, 'ws')}/ws`;
  }

  /** Old pairing links put a master token in the URL (`#/pair?token=…`).
   * Tokens must never live in URLs: drop it from the address bar and history
   * without using it (the server no longer accepts it from a phone anyway). */
  private stripLegacyTokenFromUrl(): void {
    const hash = window.location.hash;
    const q = hash.indexOf('?');
    if (q < 0) return;
    const params = new URLSearchParams(hash.slice(q + 1));
    if (!params.has('token')) return;
    params.delete('token');
    const rest = params.toString();
    const clean = `${hash.slice(0, q)}${rest ? `?${rest}` : ''}`;
    try {
      history.replaceState(history.state, '', `${location.pathname}${location.search}${clean}`);
    } catch {
      /* best effort */
    }
  }
}
