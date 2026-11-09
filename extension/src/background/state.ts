// What the service worker remembers. Chrome stops an idle MV3 worker and
// starts it again on the next event, so everything that must outlive that is
// written through: the pairing and settings to chrome.storage.local, leases
// and per-tab counters to chrome.storage.session (gone when the browser quits,
// like the tab groups they point at).

import type {
  LeaseInfo,
  PendingItem,
  SessionRef,
  Settings,
  SiteMode,
  TabGroupColor,
} from '../shared/protocol.ts';

declare const __MC_DEFAULT_PORT__: number;

export const DEFAULT_HOST = '127.0.0.1';
/** 6725, the app's main port; the e2e build uses an unusable port so a test can never reach a real app. */
export const DEFAULT_PORT: number = __MC_DEFAULT_PORT__;
const LOOPBACK_HOSTS = new Set(['127.0.0.1', 'localhost', '[::1]', '::1']);

export interface Config {
  host: string;
  port: number;
  browserName: string;
}

export interface Auth {
  token: string;
  browserId: string;
  pairedAt: number;
}

export interface LeaseRecord {
  lease: string;
  session: SessionRef;
  color: TabGroupColor;
  groupId: number | null;
  windowId: number | null;
  /** The "waiting for its first page" tab that holds an empty group open. */
  placeholderTabId: number | null;
  /** Where commands without a tabId run. */
  activeTabId: number | null;
  /** Commands are refused until then (the user pressed Stop). */
  stoppedUntil: number;
  openedAt: number;
}

export interface TabCounters {
  nextRef: number;
  nextFrame: number;
}

export interface DesktopState {
  siteModes: Record<string, SiteMode>;
  settings: Settings;
  pending: PendingItem[];
  leases: LeaseInfo[] | null;
  appVersion?: string;
}

/** A tab the user shared from the side panel after clicking the toolbar button (activeTab). */
export interface SharedTab {
  tabId: number;
  origin: string;
}

export interface SessionData {
  leases: Record<string, LeaseRecord>;
  tabs: Record<string, TabCounters>;
  nonce: string;
  shared: SharedTab | null;
  /** Tabs where the user clicked the toolbar button (Chrome granted activeTab there). */
  activeGrants: Record<string, string>;
  desktop: DesktopState;
  /**
   * The browser started (or the extension was reloaded) since the desktop
   * last welcomed us: chrome.storage.session came up empty, which Chrome does
   * then and never for a service-worker restart. The next `hello` says so
   * (`freshBrowser`), and the desktop ends the leases from before.
   */
  freshBrowser: boolean;
}

export function normalizeHost(raw: string): string | null {
  const h = raw.trim().toLowerCase();
  if (!h) return null;
  if (!LOOPBACK_HOSTS.has(h)) return null;
  return h === '::1' ? '[::1]' : h;
}

export function validPort(n: unknown): n is number {
  return typeof n === 'number' && Number.isInteger(n) && n > 0 && n < 65536;
}

function randomNonce(): string {
  const bytes = new Uint8Array(12);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
}

function emptySession(): SessionData {
  return {
    leases: {},
    tabs: {},
    nonce: randomNonce(),
    shared: null,
    activeGrants: {},
    desktop: { siteModes: {}, settings: {}, pending: [], leases: null },
    freshBrowser: false,
  };
}

export class Store {
  config: Config = { host: DEFAULT_HOST, port: DEFAULT_PORT, browserName: '' };
  auth: Auth | null = null;
  session: SessionData = emptySession();
  private saveTimer: ReturnType<typeof setTimeout> | null = null;

  async load(): Promise<void> {
    const local = await chrome.storage.local.get(['config', 'auth']);
    const config = local['config'] as Partial<Config> | undefined;
    if (config) {
      this.config = {
        host: normalizeHost(config.host ?? '') ?? DEFAULT_HOST,
        port: validPort(config.port) ? config.port : DEFAULT_PORT,
        browserName: typeof config.browserName === 'string' ? config.browserName : '',
      };
    }
    const auth = local['auth'] as Partial<Auth> | undefined;
    this.auth =
      auth && typeof auth.token === 'string' && auth.token
        ? { token: auth.token, browserId: auth.browserId ?? '', pairedAt: auth.pairedAt ?? 0 }
        : null;
    try {
      const s = await chrome.storage.session.get(['mc']);
      const saved = s['mc'] as Partial<SessionData> | undefined;
      if (saved) {
        this.session = { ...emptySession(), ...saved };
      } else {
        // Empty: the browser started, or the extension was reloaded. Kept at
        // once, so a worker restart before the desktop answers still knows.
        this.session = { ...emptySession(), freshBrowser: true };
        await this.flush();
      }
    } catch {
      // storage.session unavailable: start empty, never claiming a fresh start
    }
  }

  async saveConfig(): Promise<void> {
    await chrome.storage.local.set({ config: this.config });
  }

  async saveAuth(auth: Auth | null): Promise<void> {
    this.auth = auth;
    if (auth) await chrome.storage.local.set({ auth });
    else await chrome.storage.local.remove('auth');
  }

  /** Write the session state soon (coalesced). */
  persist(): void {
    if (this.saveTimer) return;
    this.saveTimer = setTimeout(() => {
      this.saveTimer = null;
      chrome.storage.session.set({ mc: this.session }).catch(() => undefined);
    }, 30);
  }

  async flush(): Promise<void> {
    if (this.saveTimer) {
      clearTimeout(this.saveTimer);
      this.saveTimer = null;
    }
    await chrome.storage.session.set({ mc: this.session }).catch(() => undefined);
  }

  counters(tabId: number): TabCounters {
    return this.session.tabs[String(tabId)] ?? { nextRef: 1, nextFrame: 1 };
  }

  setCounters(tabId: number, c: TabCounters): void {
    const prev = this.counters(tabId);
    if (c.nextRef !== prev.nextRef || c.nextFrame !== prev.nextFrame) {
      this.session.tabs[String(tabId)] = {
        nextRef: Math.max(prev.nextRef, c.nextRef),
        nextFrame: Math.max(prev.nextFrame, c.nextFrame),
      };
      this.persist();
    }
  }

  forgetTab(tabId: number): void {
    delete this.session.tabs[String(tabId)];
    delete this.session.activeGrants[String(tabId)];
    if (this.session.shared?.tabId === tabId) this.session.shared = null;
    this.persist();
  }

  get wsUrl(): string {
    return `ws://${this.config.host}:${this.config.port}/ws/browser`;
  }
}
