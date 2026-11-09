// Which browser this is, and which optional permissions the user granted.

import type { BrowserInfo } from '../shared/protocol.ts';
import { hostPattern } from '../shared/site.ts';

declare const __MC_VERSION__: string;
export const EXTENSION_VERSION: string = __MC_VERSION__;

/**
 * The permissions the user grants on the options page. Chrome keeps
 * webNavigation and downloads as optional permissions, asked for with its own
 * prompt; the debugger it refuses as one ("Only permissions specified in the
 * manifest may be requested"), so the manifest requires it and the user's
 * grant is a stored switch instead (DEBUGGER_GRANT_KEY), off until pressed.
 */
export const OPTIONAL_PERMISSIONS = ['debugger', 'webNavigation', 'downloads'] as const;
export type OptionalPermission = (typeof OPTIONAL_PERMISSIONS)[number];

/** chrome.storage.local: `true` once the user pressed Grant for the debugger. */
export const DEBUGGER_GRANT_KEY = 'debuggerGranted';

/** Did the user grant the debugger on the options page (and not remove it since)? */
export async function debuggerGranted(): Promise<boolean> {
  try {
    const v = await chrome.storage.local.get(DEBUGGER_GRANT_KEY);
    return v[DEBUGGER_GRANT_KEY] === true;
  } catch {
    return false;
  }
}

export async function setDebuggerGranted(granted: boolean): Promise<void> {
  if (granted) await chrome.storage.local.set({ [DEBUGGER_GRANT_KEY]: true });
  else await chrome.storage.local.remove(DEBUGGER_GRANT_KEY);
}

interface Brand {
  brand: string;
  version: string;
}
interface UaData {
  brands?: Brand[];
  getHighEntropyValues?: (hints: string[]) => Promise<{ fullVersionList?: Brand[] }>;
}
interface FirefoxRuntime {
  getBrowserInfo?: () => Promise<{ name: string; version: string }>;
}

let cached: BrowserInfo | null = null;

/** "Chrome 156.0.8078.80", "Edge …", "Brave …" (from client hints, else the user agent). */
export async function browserInfo(): Promise<BrowserInfo> {
  if (cached) return cached;
  const ff = (globalThis as { browser?: { runtime?: FirefoxRuntime } }).browser?.runtime;
  if (ff?.getBrowserInfo) {
    const i = await ff.getBrowserInfo();
    cached = { name: i.name, version: i.version };
    return cached;
  }
  const nav = navigator as Navigator & {
    userAgentData?: UaData;
    brave?: { isBrave?: () => Promise<boolean> };
  };
  let brands: Brand[] = nav.userAgentData?.brands ?? [];
  try {
    const hi = await nav.userAgentData?.getHighEntropyValues?.(['fullVersionList']);
    if (hi?.fullVersionList?.length) brands = hi.fullVersionList;
  } catch {
    // keep the low-entropy brands
  }
  const find = (re: RegExp): Brand | undefined => brands.find((b) => re.test(b.brand));
  const chromium = find(/^Chromium$/) ?? find(/Chrom/);
  let isBrave = !!find(/Brave/);
  try {
    if (!isBrave && nav.brave?.isBrave) isBrave = await nav.brave.isBrave();
  } catch {
    // not Brave
  }
  const pick = (name: string, b: Brand | undefined): BrowserInfo | null =>
    b ? { name, version: b.version } : null;
  cached =
    pick('Edge', find(/Microsoft Edge/)) ??
    (isBrave
      ? { name: 'Brave', version: find(/Brave/)?.version ?? chromium?.version ?? '' }
      : null) ??
    pick('Opera', find(/Opera/)) ??
    pick('Vivaldi', find(/Vivaldi/)) ??
    pick('Chrome', find(/Google Chrome/)) ??
    pick('Chromium', chromium) ??
    fromUserAgent(navigator.userAgent);
  return cached;
}

function fromUserAgent(ua: string): BrowserInfo {
  const edge = /Edg\/([\d.]+)/.exec(ua);
  if (edge?.[1]) return { name: 'Edge', version: edge[1] };
  const chrome = /Chrome\/([\d.]+)/.exec(ua);
  if (chrome?.[1]) return { name: 'Chrome', version: chrome[1] };
  const firefox = /Firefox\/([\d.]+)/.exec(ua);
  if (firefox?.[1]) return { name: 'Firefox', version: firefox[1] };
  return { name: 'Browser', version: '' };
}

const OS_NAMES: Record<string, string> = {
  mac: 'Mac',
  win: 'Windows',
  linux: 'Linux',
  cros: 'ChromeOS',
  openbsd: 'OpenBSD',
  android: 'Android',
};

/** A default name for pairing: "Chrome on Mac". */
export async function defaultBrowserName(): Promise<string> {
  const info = await browserInfo();
  const os = await chrome.runtime.getPlatformInfo().then(
    (p) => OS_NAMES[p.os] ?? p.os,
    () => '',
  );
  return os ? `${info.name} on ${os}` : info.name;
}

/** Granted by the user: held from Chrome and, for the debugger, switched on here too. */
export async function hasPermission(p: OptionalPermission): Promise<boolean> {
  try {
    if (!(await chrome.permissions.contains({ permissions: [p] }))) return false;
  } catch {
    return false;
  }
  return p === 'debugger' ? debuggerGranted() : true;
}

/** Optional permissions and origins granted now, as one list (`permission.changed`). */
export async function grantedList(): Promise<string[]> {
  const all = await chrome.permissions.getAll();
  const debuggerOn = await debuggerGranted();
  const perms = (all.permissions ?? []).filter(
    (p): p is OptionalPermission =>
      (OPTIONAL_PERMISSIONS as readonly string[]).includes(p) && (p !== 'debugger' || debuggerOn),
  );
  return [...perms, ...(all.origins ?? [])];
}

/** May the extension script this URL (a granted origin)? */
export async function hasHostAccess(url: string | undefined): Promise<boolean> {
  const pattern = hostPattern(url);
  if (!pattern) return false;
  try {
    return await chrome.permissions.contains({ origins: [pattern] });
  } catch {
    return false;
  }
}

export async function capabilities(): Promise<Record<string, boolean>> {
  return {
    debugger: (await hasPermission('debugger')) && typeof chrome.debugger !== 'undefined',
    sidePanel: typeof chrome.sidePanel !== 'undefined',
    tabGroups: typeof chrome.tabGroups !== 'undefined',
    webNavigation: await hasPermission('webNavigation'),
    downloads: await hasPermission('downloads'),
    screenshotFallback: typeof chrome.tabs.captureVisibleTab === 'function',
  };
}
