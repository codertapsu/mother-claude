// Site modes, checked again in the extension before every action (defence in
// depth: the desktop checks first). Modes are per origin, "*" is the default.

import type { ErrorCode, Settings, SiteMode } from './protocol.ts';

/**
 * The origin a site mode is keyed by: `scheme://host[:port]` for http(s),
 * `file://` for local files, null for pages no command may touch
 * (chrome://, the Web Store, other extensions, data:, javascript:, …).
 */
export function originOf(url: string | undefined | null): string | null {
  if (!url) return null;
  let u: URL;
  try {
    u = new URL(url);
  } catch {
    return null;
  }
  if (u.protocol === 'http:' || u.protocol === 'https:') return u.origin;
  if (u.protocol === 'file:') return 'file://';
  return null;
}

/** The key a site-mode map entry stands for ("*" or a normalized origin). */
export function normalizeOriginKey(key: string): string {
  if (key === '*') return key;
  const origin = originOf(key);
  if (origin) return origin;
  return key.replace(/\/+$/, '').toLowerCase();
}

/** The mode for a URL: its origin's entry, else "*", else the default, else off. */
export function modeFor(
  url: string | undefined | null,
  siteModes: Record<string, SiteMode>,
  settings: Settings = {},
): SiteMode {
  const origin = originOf(url);
  if (url === 'about:blank') return fallbackMode(siteModes, settings);
  if (!origin) return 'off';
  // 1. The origin's own entry.
  for (const [key, mode] of Object.entries(siteModes)) {
    if (key !== '*' && !key.includes('://*.') && normalizeOriginKey(key) === origin) return mode;
  }
  // A sensitive site (banking, payment, identity) opens only by its own entry,
  // never through a wildcard or "*".
  if (isSensitive(origin, settings)) return 'off';
  // 2. The longest wildcard `scheme://*.domain[:port]` covering it (subdomains only).
  let best: { length: number; mode: SiteMode } | null = null;
  for (const [key, mode] of Object.entries(siteModes)) {
    const w = parseWildcard(key);
    if (w && wildcardCovers(w, origin) && (!best || w.domain.length > best.length)) {
      best = { length: w.domain.length, mode };
    }
  }
  if (best) return best.mode;
  // 3. "*", then the default.
  return fallbackMode(siteModes, settings);
}

function fallbackMode(siteModes: Record<string, SiteMode>, settings: Settings): SiteMode {
  return siteModes['*'] ?? settings.defaultMode ?? 'off';
}

interface Wildcard {
  scheme: string;
  domain: string;
  port: string;
}

/** `https://*.example.com[:8443]` → its parts (null for anything else). */
export function parseWildcard(key: string): Wildcard | null {
  const m = /^(https?):\/\/\*\.([a-z0-9.-]+)(?::(\d+))?\/?$/i.exec(key.trim());
  if (!m?.[1] || !m[2]) return null;
  const scheme = m[1].toLowerCase();
  const defaultPort = scheme === 'https' ? '443' : '80';
  const port = m[3] && m[3] !== defaultPort ? m[3] : '';
  return { scheme, domain: m[2].toLowerCase(), port };
}

function wildcardCovers(w: Wildcard, origin: string): boolean {
  let u: URL;
  try {
    u = new URL(origin);
  } catch {
    return false;
  }
  return u.protocol === `${w.scheme}:` && u.hostname.endsWith(`.${w.domain}`) && u.port === w.port;
}

/** Is the origin's host one of `settings.sensitiveDomains` (a suffix match)? */
export function isSensitive(origin: string | null, settings: Settings = {}): boolean {
  if (!origin) return false;
  const list = settings['sensitiveDomains'];
  if (!Array.isArray(list)) return false;
  let host: string;
  try {
    host = new URL(origin).hostname.toLowerCase();
  } catch {
    return false;
  }
  return list.some((d) => {
    if (typeof d !== 'string' || !d) return false;
    const domain = d.toLowerCase().replace(/^\*?\./, '');
    return host === domain || host.endsWith(`.${domain}`);
  });
}

/**
 * The host permission pattern that covers a URL (what `chrome.permissions`
 * calls an origin): `https://github.com/*`. Ports are not part of a pattern.
 */
export function hostPattern(url: string | undefined | null): string | null {
  if (!url) return null;
  let u: URL;
  try {
    u = new URL(url);
  } catch {
    return null;
  }
  if (u.protocol === 'http:' || u.protocol === 'https:') return `${u.protocol}//${u.hostname}/*`;
  if (u.protocol === 'file:') return 'file:///*';
  return null;
}

/**
 * What each action needs from the site mode of the page it runs in.
 * - `none`: never touches a page (lease bookkeeping)
 * - `target`: opens a URL; the *target* origin must not be off
 * - `observe`: reads the page; read or act
 * - `act`: changes the page; act only
 */
export type ActionNeed = 'none' | 'target' | 'observe' | 'act';

export const ACTION_NEEDS: Record<string, ActionNeed> = {
  'tabs.list': 'none',
  'tabs.select': 'none',
  'tabs.close': 'none',
  'tabs.open': 'target',
  navigate: 'target',
  back: 'observe',
  forward: 'observe',
  reload: 'observe',
  snapshot: 'observe',
  read: 'observe',
  find: 'observe',
  wait: 'observe',
  screenshot: 'observe',
  console: 'observe',
  highlight: 'observe',
  scroll: 'observe',
  click: 'act',
  hover: 'act',
  dblclick: 'act',
  type: 'act',
  fill: 'act',
  select: 'act',
  check: 'act',
  press: 'act',
  upload: 'act',
  eval: 'act',
  dialog: 'act',
};

export function isKnownAction(action: string): boolean {
  return Object.hasOwn(ACTION_NEEDS, action);
}

/**
 * The refusal, if any, for running `action` on a page in `mode`.
 * `dialogKind` lets a read-only site dismiss an alert (which commits nothing).
 */
export function siteRefusal(
  action: string,
  mode: SiteMode,
  params: Record<string, unknown> = {},
  dialogKind?: string,
): ErrorCode | null {
  const need = ACTION_NEEDS[action];
  if (!need || need === 'none') return null;
  if (mode === 'off') return 'site_off';
  if (need === 'act' && mode === 'read') {
    if (action === 'dialog' && (dialogKind === 'alert' || params['accept'] === false)) return null;
    return 'site_read_only';
  }
  return null;
}

/** A human description of an action for the side panel ("reading github.com"). */
function hostOf(url: string | undefined): string {
  if (!url) return '';
  try {
    return new URL(url).host;
  } catch {
    return '';
  }
}

export function activityText(action: string, url: string | undefined): string {
  const host = hostOf(url);
  const where = host ? ` ${host}` : '';
  switch (ACTION_NEEDS[action]) {
    case 'target':
      return `opening${where}`;
    case 'observe':
      if (action === 'screenshot') return `taking a screenshot of${where}`;
      if (action === 'wait') return `waiting on${where}`;
      if (action === 'scroll') return `scrolling${where}`;
      if (action === 'console') return `reading the console of${where}`;
      if (action === 'back' || action === 'forward' || action === 'reload')
        return `navigating${where}`;
      return `reading${where}`;
    case 'act':
      if (action === 'type' || action === 'fill') return `typing on${where}`;
      if (action === 'upload') return `uploading on${where}`;
      if (action === 'eval') return `running a script on${where}`;
      if (action === 'press') return `pressing keys on${where}`;
      return `clicking on${where}`;
    default:
      return 'managing its tabs';
  }
}
