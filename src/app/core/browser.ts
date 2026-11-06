// The browser extension (docs/BROWSER.md): what `GET /api/browser` and
// `/api/browser/*` answer, and the small pure helpers the Browser page and
// the approval card share. One code path for the desktop and the phone.

/** What a session may do on a site. */
export type SiteMode = 'off' | 'read' | 'act';

export const SITE_MODES: SiteMode[] = ['off', 'read', 'act'];

/** The kind of approval a standing grant skips on one origin. */
export type GrantAction = 'eval' | 'upload' | 'navigate' | 'act' | 'secretForm' | 'burst';

export const GRANT_ACTIONS: GrantAction[] = [
  'eval',
  'upload',
  'navigate',
  'act',
  'secretForm',
  'burst',
];

export interface BrowserInfo {
  name: string;
  version: string;
}

/** A paired browser, with its connection right now. */
export interface PairedBrowser {
  id: string;
  name: string;
  browser: BrowserInfo;
  extensionVersion: string;
  /** The pinned extension (`chrome-extension://<id>`). */
  origin: string;
  createdAt: number;
  lastSeenAt?: number;
  revoked: boolean;
  revokedAt?: number;
  connected: boolean;
  connectedAt?: number;
  lastActiveAt?: number;
  capabilities?: Record<string, unknown> | null;
  /** Optional permissions the user granted (`debugger`, site patterns). */
  permissions?: string[];
  pinned?: boolean;
}

export interface BrowserSettings {
  defaultMode: SiteMode;
  screenshots: boolean;
  actionsPerMinute: number;
  pinnedBrowser?: string | null;
  uploadFolders: string[];
}

/** One site's own mode (`*` is every other site). */
export interface SiteEntry {
  origin: string;
  mode: SiteMode;
  /** Its sensitive category (`payment`, `banking`, `identity`). */
  sensitive?: string | null;
}

export interface SensitiveSite {
  domain: string;
  category: string;
}

export interface BrowserGrant {
  id: string;
  action: GrantAction | string;
  origin: string;
  createdAt: number;
  createdBy?: string;
}

/** A session acting in a browser (its tab group). */
export interface BrowserLease {
  id: string;
  session: string;
  title: string;
  browser: string;
  color: string;
  groupId?: number;
  openedAt: number;
  lastUsed: number;
  activeTab?: number;
  connected: boolean;
}

/** A session the user stopped (refused until `until`, unless resumed). */
export interface BrowserStop {
  session: string;
  browser: string;
  at: number;
  until: number;
  by: string;
}

/** A browser action waiting for approval (a pending item). */
export interface BrowserApproval {
  id: string;
  browser: string;
  lease: string;
  session: string;
  summary: string;
  detail?: string;
  createdAt: number;
}

/** `GET /api/browser`. */
export interface BrowserOverview {
  browsers: PairedBrowser[];
  settings: BrowserSettings;
  sites: SiteEntry[];
  sensitive: SensitiveSite[];
  grants: BrowserGrant[];
  leases: BrowserLease[];
  stops: BrowserStop[];
  approvals: BrowserApproval[];
  pairing: { liveCodes: number };
  /** Where the extension connects (`ws://127.0.0.1:6725/ws/browser`). */
  endpoint: string;
}

/** `POST /api/browser/pair`. */
export interface BrowserPairCode {
  code: string;
  /** `ABCD-EFGH`. */
  display: string;
  expiresAt: number;
  ttlSec: number;
}

/** One action in the activity log (never a value or a page's content). */
export interface BrowserActivity {
  at: number;
  session: string;
  tool: string;
  action?: string;
  origin?: string;
  /** `scheme://host/path`, never the query. */
  page?: string;
  outcome: string;
  code?: string;
  approved?: boolean;
  ms?: number;
  browser?: string;
}

/** `PUT /api/browser/settings` (only what changes). */
export interface BrowserSettingsPatch {
  defaultMode?: SiteMode;
  screenshots?: boolean;
  actionsPerMinute?: number;
  /** `''` unpins. */
  pinnedBrowser?: string;
  uploadFolders?: string[];
}

export const MODE_LABELS: Record<SiteMode, string> = {
  off: 'Off',
  read: 'Read',
  act: 'Act',
};

export const MODE_HINTS: Record<SiteMode, string> = {
  off: 'Sessions cannot open or see it',
  read: 'Sessions may read it, not act',
  act: 'Sessions may read and act (risky steps still ask)',
};

export const GRANT_LABELS: Record<GrantAction, string> = {
  eval: 'run scripts',
  upload: 'upload files',
  navigate: 'open this sensitive site',
  act: 'act on this sensitive site',
  secretForm: 'fill and submit forms with password or card fields',
  burst: 'go over the per-minute limit',
};

export function grantLabel(action: string): string {
  return GRANT_LABELS[action as GrantAction] ?? action;
}

/** A host name or address a site can have (after the URL parser's own
 * normalisation: lower case, IDN in punycode, IPv6 in brackets). */
const HOST = /^(?:[a-z0-9-]+\.)*[a-z0-9-]+\.?$|^\[[0-9a-f:.]+\]$/i;

/**
 * A site key as the server will store it, or null when it cannot be one
 * (`*`, an origin, a wildcard `https://*.example.com`). The server is the
 * judge; this keeps obvious mistakes out of a request.
 */
export function normalizeSite(input: string): string | null {
  const raw = input.trim();
  if (raw === '*') return '*';
  const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(raw) ? raw : `https://${raw}`;
  const wildcard = /^(https?):\/\/\*\.(.+)$/i.exec(withScheme);
  try {
    if (wildcard) {
      const u = new URL(`${wildcard[1]}://${wildcard[2]}`);
      if (!u.hostname.includes('.')) return null;
      return `${u.protocol}//*.${u.host}`.toLowerCase();
    }
    const u = new URL(withScheme);
    if (u.protocol !== 'http:' && u.protocol !== 'https:') return null;
    if (!HOST.test(u.hostname)) return null;
    return `${u.protocol}//${u.host}`.toLowerCase();
  } catch {
    return null;
  }
}

/** The host of an origin or URL (for labels). */
export function hostOf(originOrUrl: string | undefined): string {
  if (!originOrUrl) return '';
  try {
    return new URL(originOrUrl).host;
  } catch {
    return originOrUrl.replace(/^[a-z]+:\/\//i, '').split('/')[0];
  }
}

/** The sensitive category of a host, from the server's list. */
export function sensitiveCategory(originOrUrl: string, list: SensitiveSite[]): string | undefined {
  const host = hostOf(originOrUrl).split(':')[0].toLowerCase().replace(/\.$/, '');
  return list.find((s) => host === s.domain || host.endsWith(`.${s.domain}`))?.category;
}

/** Raising this site to `act` needs the dangerous right (the desktop): every
 * site, a wildcard, or a sensitive one. */
export function actNeedsDesktop(site: string, sensitive: SensitiveSite[]): boolean {
  return site === '*' || site.includes('://*.') || !!sensitiveCategory(site, sensitive);
}

const ACTION_VERBS: Record<string, string> = {
  'tabs.list': 'Listed tabs',
  'tabs.open': 'Opened a tab',
  'tabs.select': 'Switched tabs',
  'tabs.close': 'Closed a tab',
  navigate: 'Opened',
  back: 'Went back',
  forward: 'Went forward',
  reload: 'Reloaded',
  snapshot: 'Read the page',
  read: 'Read the content',
  find: 'Searched the page',
  click: 'Clicked',
  dblclick: 'Double-clicked',
  hover: 'Hovered',
  type: 'Typed',
  fill: 'Filled a form',
  select: 'Chose an option',
  check: 'Checked a box',
  press: 'Pressed a key',
  scroll: 'Scrolled',
  wait: 'Waited',
  screenshot: 'Took a screenshot',
  eval: 'Ran a script',
  console: 'Read the console',
  dialog: 'Answered a dialog',
  upload: 'Uploaded files',
  'download.started': 'Download started',
};

const LIFECYCLE: Record<string, string> = {
  'lease.open': 'Started using the browser',
  'lease.close': 'Stopped using the browser',
  stop: 'Stopped by you',
  download: 'Download started',
};

/** One activity line: "Clicked · github.com/issues". */
export function activityText(e: BrowserActivity): string {
  const verb =
    (e.action ? ACTION_VERBS[e.action] : undefined) ?? LIFECYCLE[e.tool] ?? e.action ?? e.tool;
  const where = e.page ?? e.origin;
  return where ? `${verb} · ${where.replace(/^https?:\/\//, '')}` : verb;
}

/** How an outcome reads, and its badge class. */
export function outcomeBadge(outcome: string): { label: string; cls: string } {
  switch (outcome) {
    case 'ok':
    case 'started':
      return { label: outcome === 'ok' ? 'done' : 'started', cls: 'ok' };
    case 'denied':
      return { label: 'denied', cls: 'danger' };
    case 'refused':
      return { label: 'refused', cls: 'warn' };
    case 'stopped':
      return { label: 'stopped', cls: 'warn' };
    case 'approval_timeout':
      return { label: 'not approved', cls: 'warn' };
    case 'error':
      return { label: 'failed', cls: 'danger' };
    default:
      return { label: outcome, cls: 'info' };
  }
}

/** What a browser approval's card shows, from the item's `input`. */
export interface BrowserActionView {
  action: string;
  url?: string;
  origin?: string;
  browser?: string;
  element?: { ref: string; role?: string; name?: string };
  selector?: string;
  text?: string;
  key?: string;
  expression?: string;
  touchesStorage?: boolean;
  submit?: boolean;
  fields: { ref: string; name?: string; value?: string }[];
  files: { name: string; size?: number }[];
  reasons: string[];
}

const str = (v: unknown): string | undefined => (typeof v === 'string' ? v : undefined);

/** Read a browser item's `input` tolerantly (a newer server may add more). */
export function browserActionView(input: unknown): BrowserActionView | null {
  if (!input || typeof input !== 'object') return null;
  const i = input as Record<string, unknown>;
  const action = str(i['action']);
  if (!action) return null;
  const el = i['element'] as Record<string, unknown> | undefined;
  const list = (v: unknown) => (Array.isArray(v) ? (v as Record<string, unknown>[]) : []);
  return {
    action,
    url: str(i['url']),
    origin: str(i['origin']),
    browser: str(i['browser']),
    element:
      el && typeof el === 'object' && str(el['ref'])
        ? { ref: str(el['ref'])!, role: str(el['role']), name: str(el['name']) }
        : undefined,
    selector: str(i['selector']),
    text: str(i['text']),
    key: str(i['key']),
    expression: str(i['expression']),
    touchesStorage: i['touchesStorage'] === true,
    submit: i['submit'] === true,
    fields: list(i['fields']).map((f) => ({
      ref: str(f['ref']) ?? '',
      name: str(f['name']),
      value: str(f['value']),
    })),
    files: list(i['files']).map((f) => ({
      name: str(f['name']) ?? '',
      size: typeof f['size'] === 'number' ? (f['size'] as number) : undefined,
    })),
    reasons: Array.isArray(i['reasons'])
      ? (i['reasons'] as unknown[]).filter((r): r is string => typeof r === 'string')
      : [],
  };
}

/** `12 KB`, `3.4 MB`. */
export function fileSize(bytes: number | undefined): string {
  if (bytes === undefined) return '';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
