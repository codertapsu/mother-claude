import { Injectable, signal } from '@angular/core';

export interface ResolvedConfig {
  /** Base for REST calls, e.g. `https://192.168.1.5:6725` or '' (same-origin). */
  baseUrl: string;
  /** WebSocket URL including the token query. */
  wsUrl: string;
  /** API token, or '' if not yet paired (browser without a token). */
  token: string;
  /** True when running inside the Tauri desktop webview. */
  desktop: boolean;
}

const TOKEN_KEY = 'mc_token';

/**
 * Resolves how to reach the embedded server, working both inside the Tauri
 * desktop webview (via the `server_info` invoke) and in a phone browser
 * (same-origin, token from the pairing URL or localStorage). All dashboard data
 * flows through the HTTP/WS server in both cases — never Tauri `invoke`.
 */
@Injectable({ providedIn: 'root' })
export class ConfigService {
  readonly config = signal<ResolvedConfig | null>(null);
  private pending?: Promise<ResolvedConfig>;

  ensure(): Promise<ResolvedConfig> {
    if (this.config()) return Promise.resolve(this.config()!);
    if (!this.pending) this.pending = this.resolve();
    return this.pending;
  }

  /** Persist a token captured from a pairing link / manual entry. */
  setToken(token: string): void {
    localStorage.setItem(TOKEN_KEY, token);
    const current = this.config();
    if (current) {
      this.config.set({ ...current, token, wsUrl: this.buildWsUrl(current.baseUrl, token) });
    }
  }

  hasToken(): boolean {
    return !!this.config()?.token;
  }

  private async resolve(): Promise<ResolvedConfig> {
    const captured = this.captureTokenFromUrl();
    let resolved: ResolvedConfig;

    if (this.isTauri()) {
      resolved = await this.resolveDesktop();
    } else {
      const baseUrl = window.location.origin;
      const token = captured ?? localStorage.getItem(TOKEN_KEY) ?? '';
      resolved = { baseUrl, token, wsUrl: this.buildWsUrl(baseUrl, token), desktop: false };
    }
    if (resolved.token) localStorage.setItem(TOKEN_KEY, resolved.token);
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
        wsUrl: this.buildWsUrl(baseUrl, info.token),
        desktop: true,
      };
    } catch {
      // Fallback to the documented default if the invoke is unavailable.
      const baseUrl = 'http://localhost:6725';
      const token = localStorage.getItem(TOKEN_KEY) ?? '';
      return { baseUrl, token, wsUrl: this.buildWsUrl(baseUrl, token), desktop: true };
    }
  }

  private buildWsUrl(baseUrl: string, token: string): string {
    const base = baseUrl || window.location.origin;
    const ws = base.replace(/^http/, 'ws');
    return `${ws}/ws${token ? `?token=${encodeURIComponent(token)}` : ''}`;
  }

  private captureTokenFromUrl(): string | null {
    // Pairing links look like .../#/pair?token=XYZ
    const hash = window.location.hash;
    const q = hash.indexOf('?');
    if (q >= 0) {
      const params = new URLSearchParams(hash.slice(q + 1));
      const token = params.get('token');
      if (token) {
        localStorage.setItem(TOKEN_KEY, token);
        return token;
      }
    }
    return null;
  }

  private isTauri(): boolean {
    return (
      typeof (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !==
      'undefined'
    );
  }
}
