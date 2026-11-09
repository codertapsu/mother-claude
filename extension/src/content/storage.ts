// What the page's site keeps in the browser: localStorage, sessionStorage
// (read and changed) and IndexedDB (read only). The content script shares
// the page's origin, so it sees the same storage the page does. Values leave
// only when the command asks for them (`values: true`, which the desktop
// approves first); the desktop redacts them again.

import { CommandError } from '../shared/errors.ts';

export type Area = 'local' | 'session';

/** Entries listed at most, and the longest value handed on. */
const ENTRIES_MAX = 500;
const VALUE_MAX = 4000;
const RECORDS_MAX = 100;
/** How long IndexedDB may take to open or read. */
const IDB_TIMEOUT_MS = 5000;

export function areaOf(v: unknown): Area {
  return v === 'session' ? 'session' : 'local';
}

function storageOf(area: Area): Storage {
  try {
    const s = area === 'session' ? window.sessionStorage : window.localStorage;
    if (!s) throw new Error('none');
    return s;
  } catch {
    throw new CommandError('unsupported', `this page cannot use ${area} storage`);
  }
}

function clipValue(v: string): string {
  return v.length > VALUE_MAX ? `${v.slice(0, VALUE_MAX)}…` : v;
}

/** Keys (and sizes, and values when asked) of local or session storage. */
export function readStorage(
  area: Area,
  key: string | undefined,
  values: boolean,
): Record<string, unknown> {
  const s = storageOf(area);
  const entry = (k: string): Record<string, unknown> => {
    const v = s.getItem(k) ?? '';
    return { key: k, size: v.length, ...(values ? { value: clipValue(v) } : {}) };
  };
  if (key !== undefined) {
    if (s.getItem(key) === null) {
      throw new CommandError('not_found', `${area} storage has no “${key}”`);
    }
    return { area, origin: location.origin, count: s.length, entries: [entry(key)] };
  }
  const keys: string[] = [];
  for (let i = 0; i < s.length && keys.length < ENTRIES_MAX; i++) {
    const k = s.key(i);
    if (k !== null) keys.push(k);
  }
  keys.sort();
  return {
    area,
    origin: location.origin,
    count: s.length,
    entries: keys.map(entry),
    ...(s.length > keys.length ? { truncated: true } : {}),
  };
}

export function setStorage(area: Area, key: string, value: string): Record<string, unknown> {
  const s = storageOf(area);
  try {
    s.setItem(key, value);
  } catch (e) {
    throw new CommandError(
      'unsupported',
      `${area} storage refused it: ${e instanceof Error ? e.message : String(e)}`,
    );
  }
  return { area, key, size: value.length };
}

export function removeStorage(area: Area, key: string): Record<string, unknown> {
  const s = storageOf(area);
  const had = s.getItem(key) !== null;
  s.removeItem(key);
  return { area, key, removed: had };
}

export function clearStorage(area: Area): Record<string, unknown> {
  const s = storageOf(area);
  const n = s.length;
  s.clear();
  return { area, removed: n };
}

// ──────────────────────────────────────────────────────────── IndexedDB

function withTimeout<T>(p: Promise<T>, what: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new CommandError('timeout', `${what} took too long`)),
      IDB_TIMEOUT_MS,
    );
    p.then(
      (v) => {
        clearTimeout(timer);
        resolve(v);
      },
      (e: unknown) => {
        clearTimeout(timer);
        reject(e instanceof CommandError ? e : new CommandError('internal', String(e)));
      },
    );
  });
}

function request<T>(req: IDBRequest<T>): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () =>
      reject(new CommandError('internal', String(req.error?.message ?? 'failed')));
  });
}

/** The databases the site has (names and versions). */
export async function listDatabases(): Promise<{ name: string; version: number }[]> {
  if (typeof indexedDB === 'undefined' || typeof indexedDB.databases !== 'function') {
    throw new CommandError('unsupported', 'this page cannot list its IndexedDB databases');
  }
  const list = await withTimeout(indexedDB.databases(), 'listing IndexedDB');
  return list
    .filter((d): d is { name: string; version: number } => typeof d.name === 'string')
    .map((d) => ({ name: d.name, version: d.version ?? 0 }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

/**
 * Open a database the site already has, without creating or upgrading one:
 * only a name `databases()` lists is opened, and an upgrade it would start
 * is aborted.
 */
async function openExisting(name: string): Promise<IDBDatabase> {
  const known = await listDatabases();
  if (!known.some((d) => d.name === name)) {
    throw new CommandError('not_found', `the site has no IndexedDB database “${name}”`);
  }
  return withTimeout(
    new Promise<IDBDatabase>((resolve, reject) => {
      const req = indexedDB.open(name);
      req.onupgradeneeded = () => {
        req.transaction?.abort();
        reject(new CommandError('not_found', `the IndexedDB database “${name}” is gone`));
      };
      req.onsuccess = () => resolve(req.result);
      req.onerror = () =>
        reject(new CommandError('internal', String(req.error?.message ?? 'cannot open')));
      req.onblocked = () =>
        reject(new CommandError('unsupported', `the IndexedDB database “${name}” is busy`));
    }),
    `opening “${name}”`,
  );
}

/** A structured-clone value as text: JSON where it can be, a description elsewhere. */
export function describe(v: unknown): string {
  const seen = new WeakSet<object>();
  try {
    const text = JSON.stringify(v, (_k, x: unknown) => {
      if (typeof x === 'bigint') return `${x.toString()}n`;
      if (x instanceof Blob) return `(Blob, ${x.size} bytes, ${x.type || 'no type'})`;
      if (x instanceof ArrayBuffer) return `(ArrayBuffer, ${x.byteLength} bytes)`;
      if (ArrayBuffer.isView(x)) return `(${x.constructor.name}, ${x.byteLength} bytes)`;
      if (x instanceof Map) return Object.fromEntries(x);
      if (x instanceof Set) return [...x];
      if (typeof x === 'object' && x !== null) {
        if (seen.has(x)) return '(circular)';
        seen.add(x);
      }
      return x;
    });
    return text ?? String(v);
  } catch {
    return String(v);
  }
}

/** A database's stores: names, keys, indexes and how many records. */
export async function describeDatabase(name: string): Promise<Record<string, unknown>> {
  const db = await openExisting(name);
  try {
    const stores = [];
    for (const storeName of Array.from(db.objectStoreNames)) {
      const tx = db.transaction(storeName, 'readonly');
      const store = tx.objectStore(storeName);
      const count = await withTimeout(request(store.count()), `counting “${storeName}”`);
      stores.push({
        name: storeName,
        keyPath: store.keyPath,
        autoIncrement: store.autoIncrement,
        indexes: Array.from(store.indexNames),
        count,
      });
    }
    return { db: name, version: db.version, stores };
  } finally {
    db.close();
  }
}

/** A store's first records: keys and sizes, values when asked. */
export async function readStore(
  name: string,
  storeName: string,
  limit: number,
  values: boolean,
): Promise<Record<string, unknown>> {
  const db = await openExisting(name);
  try {
    if (!db.objectStoreNames.contains(storeName)) {
      throw new CommandError('not_found', `“${name}” has no store “${storeName}”`);
    }
    const store = db.transaction(storeName, 'readonly').objectStore(storeName);
    const count = await withTimeout(request(store.count()), `counting “${storeName}”`);
    const want = Math.max(1, Math.min(RECORDS_MAX, limit));
    const records = await withTimeout(
      new Promise<Record<string, unknown>[]>((resolve, reject) => {
        const out: Record<string, unknown>[] = [];
        const req = store.openCursor();
        req.onerror = () =>
          reject(new CommandError('internal', String(req.error?.message ?? 'cannot read')));
        req.onsuccess = () => {
          const cursor = req.result;
          if (!cursor || out.length >= want) {
            resolve(out);
            return;
          }
          const text = describe(cursor.value);
          out.push({
            key: describe(cursor.key),
            size: text.length,
            ...(values ? { value: clipValue(text) } : {}),
          });
          cursor.continue();
        };
      }),
      `reading “${storeName}”`,
    );
    return {
      db: name,
      store: storeName,
      count,
      records,
      ...(count > records.length ? { truncated: true } : {}),
    };
  } finally {
    db.close();
  }
}
