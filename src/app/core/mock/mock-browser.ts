// Dev-only: the browser extension (docs/BROWSER.md) for `?mock=1` — every
// `/api/browser*` route with the server's refusals, and `browser` frames when
// something changes. One paired, connected Chrome; GitHub and the docs site
// open; one session acting in a tab group; a little activity.
// `?mockbrowser=1` adds a browser action waiting for approval (a pending
// item, so the Inbox and the session page show it too). Never bundled in
// production builds.

import {
  BrowserActivity,
  BrowserGrant,
  BrowserLease,
  BrowserOverview,
  BrowserSettings,
  BrowserStop,
  GRANT_ACTIONS,
  GrantAction,
  PairedBrowser,
  SITE_MODES,
  SensitiveSite,
  SiteMode,
  actNeedsDesktop,
  normalizeSite,
  sensitiveCategory,
} from '../browser';
import { PendingItem, ServerFrame } from '../models';
import { urlParam } from '../platform';

/** `?mockbrowser=1`: a browser action waits for approval. */
export const MOCK_BROWSER_APPROVAL = urlParam('mockbrowser') === '1';

const M = 60_000;
/** The session the mock's lease (and approval) belong to. */
export const MOCK_BROWSER_SESSION = 's-docs';

/** What the browser mock needs from the fixture backend. */
export interface BrowserHost {
  readonly desktop: boolean;
  /** Pending items across sessions (browser ones are the approvals). */
  pendingItems(): PendingItem[];
  /** Add a pending item (and show it on its session). */
  addPending(item: PendingItem): void;
  broadcast(frame: ServerFrame): void;
}

const SENSITIVE: SensitiveSite[] = [
  { domain: 'paypal.com', category: 'payment' },
  { domain: 'stripe.com', category: 'payment' },
  { domain: 'chase.com', category: 'banking' },
  { domain: 'schwab.com', category: 'banking' },
  { domain: 'accounts.google.com', category: 'identity' },
  { domain: 'login.microsoftonline.com', category: 'identity' },
  { domain: '1password.com', category: 'identity' },
];

type Body = Record<string, unknown>;

export class MockBrowser {
  private browsers: PairedBrowser[];
  private settings: BrowserSettings;
  private readonly sites = new Map<string, SiteMode>();
  private grants: BrowserGrant[];
  private leases: BrowserLease[];
  private stops: BrowserStop[] = [];
  private activity: BrowserActivity[];
  private codes = 0;

  constructor(private readonly host: BrowserHost) {
    const now = Date.now();
    this.browsers = [
      {
        id: 'brw_chrome',
        name: 'Chrome on MacBook',
        browser: { name: 'Chrome', version: '156.0' },
        extensionVersion: '0.1.0',
        origin: 'chrome-extension://abcdefghijklmnopabcdefghijklmnop',
        createdAt: now - 3 * 24 * 60 * M,
        lastSeenAt: now - 5000,
        revoked: false,
        connected: true,
        connectedAt: now - 42 * M,
        lastActiveAt: now - 5000,
        capabilities: { debugger: true, sidePanel: true, tabGroups: true },
        permissions: ['debugger', 'https://github.com/*'],
      },
    ];
    this.settings = {
      defaultMode: 'off',
      screenshots: true,
      actionsPerMinute: 30,
      pinnedBrowser: null,
      uploadFolders: [],
    };
    this.sites.set('https://github.com', 'act');
    this.sites.set('https://docs.example.com', 'read');
    this.sites.set('http://localhost:4200', 'act');
    this.grants = [
      {
        id: 'g_local',
        action: 'eval',
        origin: 'http://localhost:4200',
        createdAt: now - 26 * 60 * M,
        createdBy: 'Mac',
      },
    ];
    this.leases = [
      {
        id: 'l_docs',
        session: MOCK_BROWSER_SESSION,
        title: 'Docs site polish · website',
        browser: 'brw_chrome',
        color: 'blue',
        groupId: 12,
        openedAt: now - 18 * M,
        lastUsed: now - 40_000,
        activeTab: 412,
        connected: true,
      },
    ];
    const act = (
      ago: number,
      tool: string,
      action: string,
      page: string,
      outcome = 'ok',
      extra: Partial<BrowserActivity> = {},
    ): BrowserActivity => ({
      at: now - ago,
      session: MOCK_BROWSER_SESSION,
      tool,
      action,
      origin: new URL(page).origin,
      page,
      outcome,
      ms: 180 + (ago % 400),
      browser: 'brw_chrome',
      ...extra,
    });
    this.activity = [
      act(40_000, 'browser_click', 'click', 'https://github.com/acme/website/pulls'),
      act(55_000, 'browser_snapshot', 'snapshot', 'https://github.com/acme/website/pulls'),
      act(70_000, 'browser_navigate', 'navigate', 'https://github.com/acme/website/pulls'),
      act(3 * M, 'browser_read', 'read', 'https://docs.example.com/guide/install'),
      act(5 * M, 'browser_navigate', 'navigate', 'https://www.paypal.com/', 'refused', {
        code: 'site_off',
      }),
      act(9 * M, 'browser_eval', 'eval', 'http://localhost:4200/', 'ok', { approved: true }),
      {
        at: now - 18 * M,
        session: MOCK_BROWSER_SESSION,
        tool: 'lease.open',
        outcome: 'ok',
        browser: 'brw_chrome',
      },
    ];
    if (MOCK_BROWSER_APPROVAL) this.seedApproval(now);
  }

  /** A form with a password field, waiting for approval. */
  private seedApproval(now: number): void {
    this.host.addPending({
      id: 'req-browser-1',
      sessionId: MOCK_BROWSER_SESSION,
      kind: 'permission',
      source: 'browser',
      toolName: 'browser_click',
      title: 'Click “Sign in” on github.com',
      detail: 'It submits a form that has password, card or one-time-code fields.',
      input: {
        action: 'click',
        browser: 'Chrome on MacBook',
        url: 'https://github.com/login',
        origin: 'https://github.com',
        element: { ref: 'e14', role: 'button', name: 'Sign in' },
        reasons: ['It submits a form that has password, card or one-time-code fields.'],
      },
      suggestions: [
        {
          index: 0,
          label:
            'Always allow sessions to fill and submit forms with password or card fields on https://github.com',
          kind: 'browserGrant',
          destination: 'browser',
          standing: true,
          raw: {
            type: 'browserGrant',
            grants: [{ action: 'secretForm', origin: 'https://github.com' }],
          },
        },
      ],
      dangerous: true,
      dangerousReason: 'It submits a form that has password, card or one-time-code fields.',
      answerable: true,
      createdAt: now - 20_000,
    });
  }

  /** The answer to a request this mock takes, or null. */
  route(path: string, method: string, body: unknown, q: URLSearchParams): Response | null {
    if (path !== '/browser' && !path.startsWith('/browser/')) return null;
    const b = (body && typeof body === 'object' ? body : {}) as Body;
    if (path === '/browser' && method === 'GET') return json(this.overview());
    if (path === '/browser/activity' && method === 'GET') {
      const session = q.get('session');
      const limit = Number(q.get('limit') ?? 100) || 100;
      const entries = this.activity
        .filter((e) => !session || e.session === session)
        .sort((x, y) => y.at - x.at)
        .slice(0, limit);
      return json({ entries });
    }
    if (path === '/browser/pair' && method === 'POST') {
      if (!this.host.desktop) return dangerous('Pairing a browser');
      this.codes += 1;
      const code =
        ['K7QD', 'M2XV', 'P9TR', 'W4HN'][this.codes % 4] + ['8F3B', '6CJZ'][this.codes % 2];
      return json(
        {
          code,
          display: `${code.slice(0, 4)}-${code.slice(4)}`,
          expiresAt: Date.now() + 5 * M,
          ttlSec: 300,
        },
        201,
      );
    }
    const browser = /^\/browser\/browsers\/([^/]+)$/.exec(path);
    if (browser && method === 'DELETE') {
      if (!this.host.desktop) return dangerous('Removing a browser');
      const id = decodeURIComponent(browser[1]);
      const found = this.browsers.find((x) => x.id === id && !x.revoked);
      if (!found) return fail(404, 'browser_not_found', 'No such paired browser.');
      Object.assign(found, { revoked: true, revokedAt: Date.now(), connected: false });
      this.leases = this.leases.filter((l) => l.browser !== id);
      this.changed('browsers');
      return new Response(null, { status: 204 });
    }
    if (path === '/browser/sites' && method === 'PUT') return this.putSite(b);
    if (path === '/browser/settings' && method === 'PUT') return this.putSettings(b);
    if (path === '/browser/grants' && method === 'POST') {
      if (!this.host.desktop) return dangerous('A standing browser grant');
      const action = String(b['action'] ?? '') as GrantAction;
      const origin = normalizeSite(String(b['origin'] ?? ''));
      if (!GRANT_ACTIONS.includes(action)) {
        return fail(422, 'invalid_action', 'Unknown action.');
      }
      if (!origin || origin === '*' || origin.includes('://*.')) {
        return fail(422, 'invalid_site', 'Give an origin like https://github.com.');
      }
      const id = `g_${Math.random().toString(36).slice(2, 10)}`;
      this.grants.push({ id, action, origin, createdAt: Date.now(), createdBy: 'Mac' });
      this.changed('grants');
      return json({ id }, 201);
    }
    const grant = /^\/browser\/grants\/([^/]+)$/.exec(path);
    if (grant && method === 'DELETE') {
      const id = decodeURIComponent(grant[1]);
      if (!this.grants.some((g) => g.id === id)) {
        return fail(404, 'grant_not_found', 'No such grant.');
      }
      this.grants = this.grants.filter((g) => g.id !== id);
      this.changed('grants');
      return new Response(null, { status: 204 });
    }
    const session = /^\/browser\/sessions\/([^/]+)\/(stop|resume)$/.exec(path);
    if (session && method === 'POST') {
      const id = decodeURIComponent(session[1]);
      if (session[2] === 'stop') {
        const lease = this.leases.find((l) => l.session === id);
        if (!lease) return fail(409, 'not_browsing', 'This session is not using the browser.');
        this.leases = this.leases.filter((l) => l.session !== id);
        const now = Date.now();
        const stop = {
          session: id,
          browser: lease.browser,
          at: now,
          until: now + M,
          by: 'desktop',
        };
        this.stops = [...this.stops.filter((s) => s.session !== id), stop];
        this.activity.unshift({ at: now, session: id, tool: 'stop', outcome: 'desktop' });
        this.changed('leases');
        return json(stop);
      }
      if (!this.stops.some((s) => s.session === id && s.until > Date.now())) {
        return fail(409, 'not_stopped', "This session's browser use is not stopped.");
      }
      this.stops = this.stops.filter((s) => s.session !== id);
      this.changed('leases');
      return new Response(null, { status: 204 });
    }
    return fail(404, 'not_found', 'No such API endpoint.');
  }

  private putSite(b: Body): Response {
    const key = normalizeSite(String(b['origin'] ?? ''));
    if (!key) return fail(422, 'invalid_site', 'Give an origin like https://github.com.');
    const raw = b['mode'];
    const mode = raw === null || raw === undefined ? null : String(raw);
    if (mode !== null && !SITE_MODES.includes(mode as SiteMode)) {
      return fail(422, 'invalid_mode', 'The mode is one of off, read, act.');
    }
    if (mode === 'act' && actNeedsDesktop(key, SENSITIVE) && !this.host.desktop) {
      return dangerous('Letting sessions act there');
    }
    if (key === '*') this.settings.defaultMode = (mode as SiteMode | null) ?? 'off';
    else if (mode === null) this.sites.delete(key);
    else this.sites.set(key, mode as SiteMode);
    this.changed('sites');
    return json(this.overview());
  }

  private putSettings(b: Body): Response {
    if (b['defaultMode'] === 'act' && !this.host.desktop) {
      return dangerous('Letting sessions act on every site');
    }
    if (typeof b['defaultMode'] === 'string' && SITE_MODES.includes(b['defaultMode'] as SiteMode)) {
      this.settings.defaultMode = b['defaultMode'] as SiteMode;
    }
    if (typeof b['screenshots'] === 'boolean') this.settings.screenshots = b['screenshots'];
    if (typeof b['actionsPerMinute'] === 'number') {
      const n = b['actionsPerMinute'];
      if (n < 1 || n > 600) {
        return fail(422, 'invalid_limit', 'The limit is between 1 and 600 actions a minute.');
      }
      this.settings.actionsPerMinute = n;
    }
    if (typeof b['pinnedBrowser'] === 'string') {
      this.settings.pinnedBrowser = b['pinnedBrowser'] || null;
    }
    if (Array.isArray(b['uploadFolders'])) {
      const next = (b['uploadFolders'] as unknown[]).map(String);
      const added = next.some((f) => !this.settings.uploadFolders.includes(f));
      if (added && !this.host.desktop) {
        return dangerous('Letting sessions upload files from a new folder');
      }
      if (next.some((f) => !f.startsWith('/') && !/^[A-Za-z]:\\/.test(f))) {
        return fail(422, 'invalid_folder', 'Give a full path.');
      }
      this.settings.uploadFolders = next;
    }
    this.changed('settings');
    return json(this.overview());
  }

  private overview(): BrowserOverview {
    const pinned = this.settings.pinnedBrowser;
    return {
      browsers: this.browsers.map((b) => ({ ...b, pinned: b.id === pinned })),
      settings: { ...this.settings },
      sites: [
        { origin: '*', mode: this.settings.defaultMode },
        ...[...this.sites.entries()].map(([origin, mode]) => ({
          origin,
          mode,
          sensitive: origin.includes('://*.') ? undefined : sensitiveCategory(origin, SENSITIVE),
        })),
      ],
      sensitive: SENSITIVE,
      grants: [...this.grants],
      leases: [...this.leases],
      stops: this.stops.filter((s) => s.until > Date.now()),
      approvals: this.host
        .pendingItems()
        .filter((p) => p.source === 'browser')
        .map((p) => ({
          id: p.id,
          browser: 'brw_chrome',
          lease: 'l_docs',
          session: p.sessionId,
          summary: p.title,
          detail: p.detail,
          createdAt: p.createdAt,
        })),
      pairing: { liveCodes: this.codes ? 1 : 0 },
      endpoint: 'ws://127.0.0.1:6725/ws/browser',
    };
  }

  /** Tell the open pages to refetch. */
  private changed(what: string): void {
    this.host.broadcast({ t: 'browser', what });
  }
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function fail(status: number, code: string, message: string): Response {
  return json({ error: { code, message } }, status);
}

function dangerous(what: string): Response {
  return fail(
    403,
    'dangerous_blocked',
    `${what} is restricted to the Mother Claude desktop app, or to a device you granted dangerous actions in Settings › Phone & devices.`,
  );
}
