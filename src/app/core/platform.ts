/**
 * Small, dependency-free platform helpers shared by the core services.
 * Everything here is safe to call on any browser: storage access can throw on
 * locked-down phone browsers (blocked site data, private-mode quotas), so the
 * storage helpers degrade to "nothing stored" instead of crashing.
 */

/** True inside the Tauri desktop webview. */
export function isTauri(): boolean {
  return (
    typeof (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !==
    'undefined'
  );
}

export function storageGet(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

export function storageSet(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* in-memory only */
  }
}

export function storageRemove(key: string): void {
  try {
    localStorage.removeItem(key);
  } catch {
    /* nothing stored */
  }
}

export function sessionStorageGet(key: string): string | null {
  try {
    return sessionStorage.getItem(key);
  } catch {
    return null;
  }
}

export function sessionStorageSet(key: string, value: string): void {
  try {
    sessionStorage.setItem(key, value);
  } catch {
    /* in-memory only */
  }
}

export function isIos(): boolean {
  const ua = navigator.userAgent;
  return (
    /iPhone|iPad|iPod/.test(ua) ||
    // iPadOS reports itself as a Mac; touch support gives it away.
    (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1)
  );
}

/** Launched from the Home Screen / as an installed app. */
export function isStandalone(): boolean {
  const nav = navigator as Navigator & { standalone?: boolean };
  return (
    nav.standalone === true ||
    (typeof matchMedia === 'function' && matchMedia('(display-mode: standalone)').matches)
  );
}

/** A human name + platform id for a new device, e.g. "iPhone · Safari". */
export function describeDevice(): { name: string; platform: string } {
  const ua = navigator.userAgent;
  let platform = 'web';
  let device = 'Browser';
  if (/iPhone/.test(ua)) {
    platform = 'ios';
    device = 'iPhone';
  } else if (/iPad/.test(ua) || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1)) {
    platform = 'ios';
    device = 'iPad';
  } else if (/Android/.test(ua)) {
    platform = 'android';
    device = /Mobile/.test(ua) ? 'Android phone' : 'Android tablet';
  } else if (/Macintosh/.test(ua)) {
    platform = 'macos';
    device = 'Mac';
  } else if (/Windows/.test(ua)) {
    platform = 'windows';
    device = 'Windows PC';
  } else if (/Linux/.test(ua)) {
    platform = 'linux';
    device = 'Linux PC';
  }
  let browser = 'Browser';
  if (/EdgA?\//.test(ua)) browser = 'Edge';
  else if (/Firefox|FxiOS/.test(ua)) browser = 'Firefox';
  else if (/CriOS|Chrome/.test(ua)) browser = 'Chrome';
  else if (/Safari/.test(ua)) browser = 'Safari';
  const name = isStandalone() ? `${device} · Home Screen app` : `${device} · ${browser}`;
  return { name, platform };
}

/** Read a query parameter from the hash route (`#/pair?c=…`) or the real query. */
export function urlParam(name: string): string | null {
  const hash = window.location.hash;
  const q = hash.indexOf('?');
  if (q >= 0) {
    const v = new URLSearchParams(hash.slice(q + 1)).get(name);
    if (v !== null) return v;
  }
  return new URLSearchParams(window.location.search).get(name);
}
