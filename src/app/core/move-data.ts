import { DEFAULT_ACCOUNT, homeOf, sharersOf } from './accounts';
import { ApiError } from './api.service';
import { backupErrorText } from './backup';
import { formatBytes, plural, sentence } from './data-jobs';
import { AccountKind, BackupIndexSession, JobView, SessionSummary, TransferMode } from './models';

/**
 * Moving data anywhere (docs/ACCOUNTS.md §13): where data can be — an
 * account (a directory of its own, or the shared folder the Default and
 * every token account use) or a set of a storage — the `items` body of
 * `POST /data/move`, its dry run and its jobs' results, read tolerantly like
 * everything else from the wire; which ways data can go between two places,
 * what each way does, and the words for all of it.
 */

// ───────────────────────────────────────────────────────────── Storages

export type StorageKind = 'icloud' | 'gdrive' | 'onedrive' | 'dropbox' | 'folder';

/** The backup folder's storage id (the first storage). */
export const BACKUP_STORAGE = 'backup';

/** What the server calls the backup folder's storage. */
export const BACKUP_LABEL = 'Backup folder';

/** A storage's label already says it is the backup folder. */
export function namesBackupFolder(label: string): boolean {
  return label.trim().toLowerCase() === BACKUP_LABEL.toLowerCase();
}

/** One set of a storage: `<storage>/<name>/`. */
export interface StorageSet {
  name: string;
  /** The account it is named after: that account's backup. Absent for a
   * set only Move data reads and writes ("Shared", "Archive 2026"). */
  account?: string;
  exists: boolean;
  /** In the folder without its key (deleted, or a cloud placeholder). */
  keyMissing: boolean;
  sessions?: number;
  /** Entries archived into it: in the storage only. */
  archived?: number;
}

/** A folder that holds encrypted sets (`GET /backup` `storage`). */
export interface StorageView {
  /** `backup` (the backup folder) or `stor_` + 12 hex. */
  id: string;
  label: string;
  path: string;
  kind: StorageKind;
  /** The backup folder: listed first, set on Backup & sync. */
  primary: boolean;
  /** Its folder is there now (a disk can be unplugged). */
  available: boolean;
  sets: StorageSet[];
}

const STORAGE_KINDS: readonly StorageKind[] = ['icloud', 'gdrive', 'onedrive', 'dropbox', 'folder'];

const KIND_LABELS: Record<StorageKind, string> = {
  icloud: 'iCloud Drive',
  gdrive: 'Google Drive',
  onedrive: 'OneDrive',
  dropbox: 'Dropbox',
  folder: 'Backup folder',
};

/** What kind of folder a path is, from where the cloud apps keep theirs. */
export function storageKindOf(path: string): StorageKind {
  if (/com~apple~CloudDocs|\/iCloud Drive\b/i.test(path)) return 'icloud';
  if (/GoogleDrive|Google Drive/i.test(path)) return 'gdrive';
  if (/OneDrive/i.test(path)) return 'onedrive';
  if (/Dropbox/i.test(path)) return 'dropbox';
  return 'folder';
}

function str(v: unknown): string | undefined {
  return typeof v === 'string' && v.trim() ? v : undefined;
}

function num(v: unknown): number | undefined {
  return typeof v === 'number' && Number.isFinite(v) ? v : undefined;
}

function count(v: unknown): number {
  const n = num(v);
  return n !== undefined && n > 0 ? Math.round(n) : 0;
}

function obj(v: unknown): Record<string, unknown> | null {
  return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
}

function list<T>(v: unknown, read: (x: unknown) => T | null): T[] {
  return Array.isArray(v) ? v.map(read).filter((x): x is T => x !== null) : [];
}

function texts(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string' && !!x.trim()) : [];
}

/** `/Users/me/x` → `~/x`. */
function tilde(path: string | undefined): string {
  return (path ?? '').replace(/^\/(Users|home)\/[^/]+/, '~');
}

function storageSet(raw: unknown): StorageSet | null {
  const r = obj(raw);
  const name = str(r?.['name']);
  if (!r || !name) return null;
  const account = str(r['account']);
  const sessions = num(r['sessions']);
  const archived = num(r['archived']);
  return {
    name: name.trim(),
    ...(account ? { account } : {}),
    exists: r['exists'] === true,
    keyMissing: r['keyMissing'] === true,
    ...(sessions !== undefined && sessions >= 0 ? { sessions } : {}),
    ...(archived !== undefined && archived > 0 ? { archived } : {}),
  };
}

function storage(raw: unknown): StorageView | null {
  const r = obj(raw);
  const id = str(r?.['id']);
  const path = str(r?.['path']);
  if (!r || !id || !path) return null;
  const kind = STORAGE_KINDS.find((k) => k === r['kind']) ?? storageKindOf(path);
  const seen = new Set<string>();
  const sets = list(r['sets'], storageSet).filter((s) => !seen.has(s.name) && !!seen.add(s.name));
  return {
    id,
    label: str(r['label'])?.trim() ?? KIND_LABELS[kind],
    path,
    kind,
    primary: r['primary'] === true || id === BACKUP_STORAGE,
    // Absent means there: only a server that looked says otherwise.
    available: r['available'] !== false,
    sets,
  };
}

/**
 * The storages from `GET /backup`'s answer, the backup folder first. A
 * server that sends no `storage` (one from before storages, or one with
 * only the Default account and the backup folder, which answers as before:
 * goal 7) has the backup folder alone, with the sets of the accounts (a
 * token account's is the Default's).
 */
export function normalizeStorages(raw: unknown): StorageView[] {
  const r = obj(raw);
  if (!r) return [];
  if (Array.isArray(r['storage'])) {
    const seen = new Set<string>();
    const out = list(r['storage'], storage).filter((s) => !seen.has(s.id) && !!seen.add(s.id));
    return [...out.filter((s) => s.primary), ...out.filter((s) => !s.primary)];
  }
  const folder = str(obj(r['settings'])?.['folder']);
  if (!folder) return [];
  const sets = (Array.isArray(r['sets']) ? r['sets'] : [])
    .map(obj)
    .filter((s): s is Record<string, unknown> => !!s && !str(s['sharedWith']))
    .map((s) =>
      storageSet({
        name: s['name'] ?? s['account'],
        account: s['account'],
        exists: s['exists'],
        keyMissing: s['keyMissing'],
        sessions: s['sessions'],
      }),
    )
    .filter((s): s is StorageSet => !!s);
  const kind = storageKindOf(folder);
  return [
    {
      id: BACKUP_STORAGE,
      // As the server names it once it lists storages.
      label: BACKUP_LABEL,
      path: folder,
      kind,
      primary: true,
      available: true,
      sets,
    },
  ];
}

/** Why a name can't be a new set's ('' when it can): a folder's name. */
export function setNameProblem(name: string, store: StorageView | undefined): string {
  const n = name.trim();
  if (!n) return 'Name the new set.';
  if ([...n].length > 40) return 'At most 40 characters.';
  if (n.startsWith('.')) return "A name can't start with a dot.";
  // eslint-disable-next-line no-control-regex
  if (/[/\\:*?"<>|\u0000-\u001f\u007f]/.test(n)) return 'A name can\'t contain / \\ : * ? " < > |';
  const lower = n.toLowerCase();
  if (store?.sets.some((s) => s.name.toLowerCase() === lower)) {
    return 'There is a set with this name already: pick it in the list.';
  }
  return '';
}

/** Why a typed folder can't be a storage ('' when it may be one; the Mac
 * checks the rest: not inside an account's directory, not another one). */
export function storagePathProblem(path: string): string {
  const p = path.trim();
  if (!p) return "Enter the folder's path.";
  if (!p.startsWith('/') && !p.startsWith('~')) {
    return "Enter the folder's full path, starting with / (or ~ for your home folder).";
  }
  if (p.split('/').some((part) => part === '.' || part === '..')) {
    return 'Enter the path without . or .. in it.';
  }
  return '';
}

// ──────────────────────────────────────────────────────────── Locations

/** Where data is: an account (its directory, or the shared folder), or one
 * set of a storage (it may not exist yet: a new set as a target). */
export type DataLocation =
  { kind: 'account'; account: string } | { kind: 'storage'; storage: string; set: string };

/** A location on the wire: a bare account id, or `{storage, set}`. */
export type WireLocation = string | { account: string } | { storage: string; set?: string };

export function accountLocation(account: string): DataLocation {
  return { kind: 'account', account };
}

export function setLocation(storage: string, set: string): DataLocation {
  return { kind: 'storage', storage, set };
}

/** What the server is sent: an account as its bare id. */
export function wireLocation(l: DataLocation): WireLocation {
  return l.kind === 'account' ? l.account : { storage: l.storage, set: l.set };
}

/** A location from the wire (any of its three forms), or null. */
export function readLocation(raw: unknown): DataLocation | null {
  if (typeof raw === 'string') return raw.trim() ? accountLocation(raw.trim()) : null;
  const r = obj(raw);
  if (!r) return null;
  const account = str(r['account']);
  if (account) return accountLocation(account);
  const store = str(r['storage']);
  if (!store) return null;
  return setLocation(store, str(r['set'])?.trim() ?? '');
}

export function sameLocation(a: DataLocation | null, b: DataLocation | null): boolean {
  if (!a || !b) return a === b;
  if (a.kind === 'account' && b.kind === 'account') return a.account === b.account;
  if (a.kind === 'storage' && b.kind === 'storage') {
    return a.storage === b.storage && a.set === b.set;
  }
  return false;
}

/** A location as a `<select>` value: `a:<id>`, `s:<storage>:<set>` ('' for
 * none: "where each one is"). */
export function locationKey(l: DataLocation | null): string {
  if (!l) return '';
  return l.kind === 'account' ? `a:${l.account}` : `s:${l.storage}:${l.set}`;
}

/** The value of "New set…" in a storage's group. */
export function newSetKey(storageId: string): string {
  return `n:${storageId}`;
}

/** The storage a "New set…" value names ('' for any other value). */
export function newSetStorage(key: string): string {
  return key.startsWith('n:') ? key.slice(2) : '';
}

export function locationFromKey(key: string): DataLocation | null {
  if (key.startsWith('a:') && key.length > 2) return accountLocation(key.slice(2));
  if (key.startsWith('s:')) {
    const rest = key.slice(2);
    const i = rest.indexOf(':');
    if (i > 0) return setLocation(rest.slice(0, i), rest.slice(i + 1));
  }
  return null;
}

/** The account fields these helpers read. */
export interface AccountLike {
  id: string;
  name: string;
  kind: AccountKind | string;
  isDefault?: boolean;
  dir?: string;
}

function accountOf<T extends AccountLike>(accounts: readonly T[], id: string): T | undefined {
  return accounts.find((a) => a.id === id);
}

function storageOf(storages: readonly StorageView[], id: string): StorageView | undefined {
  return storages.find((s) => s.id === id);
}

/** A storage's name as a person says it: its label ('the backup folder'
 * for the backup folder before the list is read; 'a storage' else). */
export function storageLabel(storages: readonly StorageView[], id: string): string {
  return (
    storageOf(storages, id)?.label ?? (id === BACKUP_STORAGE ? 'the backup folder' : 'a storage')
  );
}

/** "Work", "Google Drive · Shared"; `everywhere` for none. */
export function locationName(
  l: DataLocation | null,
  accounts: readonly AccountLike[],
  storages: readonly StorageView[],
  everywhere = 'where each one is',
): string {
  if (!l) return everywhere;
  if (l.kind === 'account') return accountOf(accounts, l.account)?.name ?? 'a removed account';
  return l.set
    ? `${storageLabel(storages, l.storage)} · ${l.set}`
    : storageLabel(storages, l.storage);
}

/** One line about a location, under its picker. */
export function locationHint(
  l: DataLocation | null,
  accounts: readonly AccountLike[],
  storages: readonly StorageView[],
): string {
  if (!l) return 'Each conversation goes from the account it is in: one job for each.';
  if (l.kind === 'account') {
    const a = accountOf(accounts, l.account);
    if (!a) return 'This account was removed.';
    const shared = accountOf(accounts, DEFAULT_ACCOUNT);
    const sharedDir = tilde(shared?.dir) || 'the Default account’s folder';
    if (a.kind === 'token') {
      return `A token account: its conversations are in the shared folder (${sharedDir}) with ${shared?.name ?? 'Default'}'s.`;
    }
    const sharers = sharersOf(accounts, a.id);
    if (sharers.length) {
      return `The shared folder, ${tilde(a.dir) || sharedDir}: ${andNames([a, ...sharers])} use it.`;
    }
    return a.isDefault ? tilde(a.dir) : `Its own folder, ${tilde(a.dir)}.`;
  }
  const s = storageOf(storages, l.storage);
  if (!s) return 'This storage is no longer there.';
  const set = s.sets.find((x) => x.name === l.set);
  const where = `${tilde(s.path)}/${l.set}`;
  if (!set) return `A new set: ${where}.`;
  const facts = [
    set.sessions !== undefined ? plural(set.sessions, 'conversation') : '',
    set.archived ? `${set.archived} in storage only` : '',
    set.keyMissing ? 'its key is not in the folder on the Mac' : '',
    s.available ? '' : 'not available now',
  ].filter(Boolean);
  return facts.length ? `${where} · ${facts.join(' · ')}` : where;
}

function andNames(items: readonly { name: string }[]): string {
  const names = items.map((a) => a.name);
  if (names.length <= 1) return names[0] ?? '';
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
}

export interface LocationOption {
  key: string;
  label: string;
  disabled: boolean;
}

export interface LocationGroup {
  label: string;
  options: LocationOption[];
}

/**
 * The places a From or To picker offers: the accounts (a directory of
 * their own, or the shared folder with the accounts that use it), then each
 * storage with its sets — on the To side with "New set…". A set can't be
 * read without its key, nor anything in a storage that isn't there now.
 */
export function locationGroups(
  accounts: readonly AccountLike[],
  storages: readonly StorageView[],
  side: 'from' | 'to',
): LocationGroup[] {
  const groups: LocationGroup[] = [];
  if (accounts.length) {
    groups.push({
      label: 'Accounts',
      options: accounts.map((a) => ({
        key: locationKey(accountLocation(a.id)),
        label: accountOptionLabel(a, accounts),
        disabled: false,
      })),
    });
  }
  for (const s of storages) {
    const sets = side === 'from' ? s.sets.filter((x) => x.exists) : s.sets;
    const options: LocationOption[] = sets.map((x) => ({
      key: locationKey(setLocation(s.id, x.name)),
      label: x.keyMissing ? `${x.name} — no key here` : x.name,
      disabled: x.keyMissing || !s.available,
    }));
    if (side === 'to') {
      options.push({ key: newSetKey(s.id), label: 'New set…', disabled: !s.available });
    }
    if (!options.length) continue;
    const tags = [
      s.primary && !namesBackupFolder(s.label) ? 'backup folder' : '',
      s.available ? '' : 'not available',
    ].filter(Boolean);
    groups.push({ label: tags.length ? `${s.label} (${tags.join(', ')})` : s.label, options });
  }
  return groups;
}

function accountOptionLabel(a: AccountLike, accounts: readonly AccountLike[]): string {
  if (a.kind === 'token') return `${a.name} — shared folder`;
  if (sharersOf(accounts, a.id).length) return `${a.name} — shared folder`;
  return a.isDefault || a.id === DEFAULT_ACCOUNT ? a.name : `${a.name} — own folder`;
}

// ─────────────────────────────────────────────────────────── Directions

/**
 * Which way data goes (docs/ACCOUNTS.md §13.2): between accounts in two
 * directories; between accounts that share one (an assign: nothing moves);
 * into a storage, out of one, or between two; from wherever each
 * conversation is (`mixed`); or nowhere (`same`).
 */
export type MoveDirection =
  'between' | 'assign' | 'toStorage' | 'fromStorage' | 'storages' | 'mixed' | 'same';

/** The directory an account id's data is in, as an account id (a token
 * account's is the Default's; an id nobody knows is its own). */
export function accountHome(accounts: readonly AccountLike[], id: string): string {
  const a = accountOf(accounts, id);
  return a ? homeOf(a) : id;
}

export function moveDirection(
  from: DataLocation | null,
  to: DataLocation | null,
  accounts: readonly AccountLike[],
): MoveDirection {
  if (!to) return 'same';
  // Left out: conversations of the accounts, each from where it is.
  if (!from) return to.kind === 'storage' ? 'toStorage' : 'mixed';
  if (sameLocation(from, to)) return 'same';
  if (from.kind === 'account' && to.kind === 'account') {
    return accountHome(accounts, from.account) === accountHome(accounts, to.account)
      ? 'assign'
      : 'between';
  }
  if (from.kind === 'account') return 'toStorage';
  return to.kind === 'account' ? 'fromStorage' : 'storages';
}

/** What a move takes. */
export type MoveWhat = 'all' | 'projects' | 'sessions' | 'customizations';

export type CustomizationItem =
  | 'memory'
  | 'settings'
  | 'skills'
  | 'agents'
  | 'commands'
  | 'outputStyles'
  | 'keybindings'
  | 'mcpServers'
  | 'history';

export interface CustomizationInfo {
  item: CustomizationItem;
  label: string;
  hint: string;
  /** Copying it needs the dangerous right (it can run code or choose how
   * Claude signs in). */
  dangerous: boolean;
}

/** The customizations an account's directory has (§13.1), in the order the
 * checklist shows them. Plugins are never copied. */
export const CUSTOMIZATIONS: readonly CustomizationInfo[] = [
  { item: 'memory', label: 'Memory', hint: 'CLAUDE.md', dangerous: false },
  {
    item: 'settings',
    label: 'Settings',
    hint: 'settings.json: permissions, hooks, the status line, env',
    dangerous: true,
  },
  { item: 'skills', label: 'Skills', hint: 'skills/', dangerous: false },
  { item: 'agents', label: 'Agents', hint: 'agents/', dangerous: false },
  { item: 'commands', label: 'Commands', hint: 'commands/', dangerous: false },
  { item: 'outputStyles', label: 'Output styles', hint: 'output-styles/', dangerous: false },
  { item: 'keybindings', label: 'Keybindings', hint: 'keybindings.json', dangerous: false },
  {
    item: 'mcpServers',
    label: 'MCP servers',
    hint: 'the user-scope servers, merged by name',
    dangerous: true,
  },
  { item: 'history', label: 'Prompt history', hint: 'history.jsonl, merged', dangerous: false },
];

export function customizationLabel(item: string): string {
  return CUSTOMIZATIONS.find((c) => c.item === item)?.label ?? item;
}

/** What a device without the dangerous right is told about the parts that
 * need it (a folder's trust, settings and MCP servers). */
export const DANGEROUS_REASON =
  'Needs dangerous actions for this device, granted on the Mac in Remote › Devices.';

/** Why `what` can't go this way ('' when it can): with `from` left out
 * (`everywhere`) only conversations can. */
export function whatProblem(what: MoveWhat, direction: MoveDirection, everywhere = false): string {
  if (everywhere && what !== 'sessions') return 'Choose where they come from first.';
  if (what === 'customizations' && direction !== 'between') {
    return 'Customizations are copied only between accounts in different folders.';
  }
  return '';
}

/** Why a mode can't be used for these places and items ('' when it can). */
export function modeProblem(mode: TransferMode, direction: MoveDirection, what: MoveWhat): string {
  if (direction === 'same') return 'Pick another place for it to go.';
  if (mode === 'move' && what === 'customizations') return 'Customizations are only ever copied.';
  if (mode === 'copy' && direction === 'assign') {
    return 'Both are in the shared folder: a copy there would be the same conversation.';
  }
  return '';
}

/** What the chosen mode does, in one sentence (`from` and `to` are names). */
export function modeHint(
  mode: TransferMode,
  direction: MoveDirection,
  what: MoveWhat,
  from: string,
  to: string,
  replaceExisting = false,
): string {
  if (what === 'customizations') {
    return replaceExisting
      ? `Adds what ${to} lacks and replaces what it has; its own copy goes into the undo folder first.`
      : `Adds what ${to} lacks; what it has stays as it is.`;
  }
  const move = mode === 'move';
  switch (direction) {
    case 'between':
      return move
        ? `Only ${to} has it afterwards: it leaves ${from} once every copy is verified.`
        : 'Both accounts have it afterwards; each copy goes on by itself.';
    case 'assign':
      return `Nothing moves: it is recorded as ${to}'s, so it shows, counts and continues under ${to}.`;
    case 'toStorage':
      return move
        ? `Archived: it goes into ${to} and leaves ${from} once its copy there is verified. ${to} lists it as in storage only.`
        : `A backup of it goes into ${to}; ${from} keeps it as it is.`;
    case 'fromStorage':
      return move
        ? `Restored into ${to}, then taken out of ${from} (kept seven days for Undo).`
        : `Restored into ${to}; ${from} keeps it.`;
    case 'storages':
      return move
        ? `Copied into ${to}, then taken out of ${from}.`
        : `Copied into ${to}; ${from} keeps it.`;
    case 'mixed':
      return move
        ? `Each goes from the account it is in to ${to}; from an account in the same folder that only records it as ${to}'s.`
        : `Each is copied from the account it is in to ${to}.`;
    default:
      return '';
  }
}

/** The trust option means something: the items go into an account's
 * directory from another one, or from a storage. */
export function trustApplies(direction: MoveDirection, what: MoveWhat): boolean {
  return (
    what !== 'customizations' &&
    (direction === 'between' || direction === 'fromStorage' || direction === 'mixed')
  );
}

/** Where things go when nobody says: another account's directory, else
 * another account in the same one (an assign), else the source's set in
 * the backup folder, else the first set of any storage. */
export function defaultTarget(
  from: DataLocation | null,
  accounts: readonly AccountLike[],
  storages: readonly StorageView[],
): DataLocation | null {
  if (from?.kind === 'storage') {
    const set = storageOf(storages, from.storage)?.sets.find((s) => s.name === from.set);
    const named = set?.account && accountOf(accounts, set.account);
    return accountLocation(named ? named.id : DEFAULT_ACCOUNT);
  }
  const source = from?.account ?? DEFAULT_ACCOUNT;
  const home = accountHome(accounts, source);
  const other = accounts.find((a) => homeOf(a) !== home);
  if (other) return accountLocation(other.id);
  const sharing = accounts.find((a) => a.id !== source);
  if (from && sharing) return accountLocation(sharing.id);
  const owner = accountOf(accounts, home);
  for (const s of storages) {
    if (!s.available) continue;
    const own = s.sets.find((x) => !x.keyMissing && (x.account === home || x.name === owner?.name));
    if (own) return setLocation(s.id, own.name);
  }
  for (const s of storages) {
    const first = s.sets.find((x) => !x.keyMissing);
    if (s.available && first) return setLocation(s.id, first.name);
  }
  return null;
}

/** Somewhere to move things to exists: another account, or a storage. */
export function anyTarget(
  accounts: readonly AccountLike[],
  storages: readonly StorageView[],
): boolean {
  return accounts.length > 1 || storages.some((s) => s.available);
}

// ──────────────────────────────────────────────────────── The request

export interface MoveItems {
  sessions?: string[];
  projects?: string[];
  all?: true;
  customizations?: CustomizationItem[];
}

/** `POST /data/move` with `items` (desktop and paired devices). */
export interface MoveBody {
  /** Left out only for conversations: each goes from where it is. */
  from?: WireLocation;
  to: WireLocation;
  items: MoveItems;
  mode: TransferMode;
  trust: boolean;
  replaceExisting: boolean;
  dryRun?: boolean;
}

/** What the Move data sheet has chosen. */
export interface MoveChoice {
  from: DataLocation | null;
  to: DataLocation;
  what: MoveWhat;
  sessions: readonly string[];
  projects: readonly string[];
  customizations: readonly CustomizationItem[];
  mode: TransferMode;
  trust: boolean;
  replaceExisting: boolean;
}

/** The request for a choice (without `dryRun`). */
export function moveBody(c: MoveChoice): MoveBody {
  const items: MoveItems =
    c.what === 'all'
      ? { all: true }
      : c.what === 'projects'
        ? { projects: [...c.projects] }
        : c.what === 'customizations'
          ? { customizations: [...c.customizations] }
          : { sessions: [...c.sessions] };
  return {
    ...(c.from ? { from: wireLocation(c.from) } : {}),
    to: wireLocation(c.to),
    items,
    mode: c.what === 'customizations' ? 'copy' : c.mode,
    trust: c.what !== 'customizations' && c.trust,
    replaceExisting: c.what === 'customizations' && c.replaceExisting,
  };
}

// ─────────────────────────────────────────────────────────── The plan

export type MoveAction = 'move' | 'copy' | 'assign' | 'archive' | 'restore';
export type CustomizationAction = 'add' | 'replace' | 'skip' | 'merge';

const ACTIONS: readonly MoveAction[] = ['move', 'copy', 'assign', 'archive', 'restore'];
const CUSTOM_ACTIONS: readonly CustomizationAction[] = ['add', 'replace', 'skip', 'merge'];

export interface MovePlanSession {
  id: string;
  cwd?: string;
  title?: string;
  files: number;
  bytes: number;
  /** Where it comes from (with `from` left out, each its own). */
  from?: DataLocation;
  action: MoveAction;
}

/** Something that does not go, and why. */
export interface MoveSkip {
  id?: string;
  cwd?: string;
  /** A customization category. */
  item?: string;
  code: string;
  message: string;
}

export interface CustomizationPlan {
  item: string;
  name: string;
  action: CustomizationAction;
  reason?: string;
}

/** A dry run: everything a move would do, nothing done yet. */
export interface MovePlan {
  from?: DataLocation;
  to?: DataLocation;
  mode: TransferMode;
  /** One job per source location. */
  jobs: number;
  sessions: MovePlanSession[];
  skipped: MoveSkip[];
  projects: string[];
  addsProjectEntries: string[];
  removesProjectEntries: string[];
  historyLines: number;
  customizations: CustomizationPlan[];
  warnings: string[];
  files: number;
  bytes: number;
  writesShared: boolean;
  /** Some list holds only its first 1000 entries (the run takes them all). */
  truncated: boolean;
}

/** A list of files as an older plan sent it, or a count. */
function fileCount(v: unknown): { files: number; bytes: number } {
  if (Array.isArray(v)) {
    return {
      files: v.length,
      bytes: v.reduce((n: number, f) => n + count(obj(f)?.['size']), 0),
    };
  }
  return { files: count(v), bytes: 0 };
}

function planSession(raw: unknown, mode: TransferMode): MovePlanSession | null {
  const r = obj(raw);
  const id = str(r?.['id']);
  if (!r || !id) return null;
  const cwd = str(r['cwd']);
  const title = str(r['title']);
  const from = readLocation(r['from']);
  const files = fileCount(r['files']);
  return {
    id,
    ...(cwd ? { cwd } : {}),
    ...(title ? { title: title.trim() } : {}),
    files: files.files,
    bytes: num(r['bytes']) !== undefined ? count(r['bytes']) : files.bytes,
    ...(from ? { from } : {}),
    action: ACTIONS.find((a) => a === r['action']) ?? mode,
  };
}

function skip(raw: unknown): MoveSkip | null {
  if (typeof raw === 'string') return raw.trim() ? { code: '', message: raw.trim() } : null;
  const r = obj(raw);
  if (!r) return null;
  const id = str(r['id']);
  const cwd = str(r['cwd']);
  const item = str(r['item']);
  const code = str(r['code']) ?? '';
  const message = str(r['message']) ?? str(r['reason']) ?? '';
  if (!id && !cwd && !item && !code && !message) return null;
  return {
    ...(id ? { id } : {}),
    ...(cwd ? { cwd } : {}),
    ...(item ? { item } : {}),
    code,
    message,
  };
}

function customization(raw: unknown): CustomizationPlan | null {
  const r = obj(raw);
  const item = str(r?.['item']);
  const action = CUSTOM_ACTIONS.find((a) => a === r?.['action']);
  if (!r || !item || !action) return null;
  const reason = str(r['reason']);
  return { item, name: str(r['name']) ?? item, action, ...(reason ? { reason } : {}) };
}

function truncatedFlag(v: unknown): boolean {
  if (v === true) return true;
  if (Array.isArray(v)) return v.length > 0;
  const r = obj(v);
  return !!r && Object.values(r).some((x) => x === true);
}

/** A dry run from the wire (`{plan}`'s value), or null when it isn't one. */
export function normalizeMovePlan(raw: unknown): MovePlan | null {
  const r = obj(raw);
  const mode: TransferMode | undefined =
    r?.['mode'] === 'move' || r?.['mode'] === 'copy' ? r['mode'] : undefined;
  if (!r || !mode) return null;
  const from = readLocation(r['from']);
  const to = readLocation(r['to']);
  const sessions = list(r['sessions'], (x) => planSession(x, mode));
  const jobs = num(r['jobs']);
  return {
    ...(from ? { from } : {}),
    ...(to ? { to } : {}),
    mode,
    jobs: jobs !== undefined && jobs >= 0 ? Math.round(jobs) : 1,
    sessions,
    skipped: list(r['skipped'], skip),
    projects: texts(r['projects']),
    addsProjectEntries: texts(r['addsProjectEntries']),
    removesProjectEntries: texts(r['removesProjectEntries']),
    historyLines: count(r['historyLines']),
    customizations: list(r['customizations'], customization),
    warnings: texts(r['warnings']),
    files: count(r['files']),
    bytes: count(r['bytes']),
    writesShared: r['writesShared'] === true,
    truncated:
      truncatedFlag(r['truncated']) ||
      ['sessions', 'skipped', 'customizations'].some((k) => r[`${k}Truncated`] === true),
  };
}

const VERBS: Record<MoveAction, string> = {
  move: 'Moves',
  copy: 'Copies',
  assign: 'Assigns',
  archive: 'Archives',
  restore: 'Restores',
};

const ACTION_LABELS: Record<MoveAction, string> = {
  move: 'Move',
  copy: 'Copy',
  assign: 'Assign',
  archive: 'Archive',
  restore: 'Restore',
};

/** "Archive": what happens to one conversation, as a tag. */
export function actionLabel(action: MoveAction): string {
  return ACTION_LABELS[action];
}

/** One sentence about what a dry run would do with the conversations
 * (`toName`: where they go). */
export function planLine(p: MovePlan, toName: string): string {
  if (!p.sessions.length) {
    return p.customizations.length
      ? `Copies customizations into ${toName}.`
      : 'There is nothing to move or copy here.';
  }
  const byAction = new Map<MoveAction, number>();
  for (const s of p.sessions) byAction.set(s.action, (byAction.get(s.action) ?? 0) + 1);
  const size = p.files || p.bytes ? ` (${plural(p.files, 'file')}, ${formatBytes(p.bytes)})` : '';
  const parts = [...byAction.entries()].map(([a, n], i) => {
    const what = plural(n, 'conversation');
    return `${i === 0 ? VERBS[a] : VERBS[a].toLowerCase()} ${what}`;
  });
  const more = p.truncated ? ' (the first 1000 are listed)' : '';
  const into = byAction.size === 1 && !byAction.has('assign') ? 'into' : 'to';
  return `${parts.join(', ')}${size} ${into} ${toName}${more}.`;
}

/** What a customization line says: "review: added". */
export function customizationText(c: CustomizationPlan): string {
  const verb =
    c.action === 'add'
      ? 'added'
      : c.action === 'replace'
        ? 'replaced (its copy kept for Undo)'
        : c.action === 'merge'
          ? 'merged'
          : 'left as it is';
  return c.reason ? `${verb}: ${c.reason.trim().replace(/\.$/, '')}` : verb;
}

/** What each bulk skip code means, when the server says nothing. */
const SKIP_TEXT: Record<string, string> = {
  same_directory:
    "It is in that account's folder already: a copy there would be the same conversation. Move it instead, which records it as that account's.",
  session_live: 'It is running; stop it first.',
  already_exists: 'The target has it already.',
  not_found: 'It could not be read.',
  session_changed: 'It changed while being checked; try again.',
  archived: 'It is in the storage only.',
};

/** Skip codes said our way, whatever the server's words. */
const OUR_SKIP_WORDS = new Set(['same_directory']);

/** Why something does not go, as a sentence. */
export function skipReason(s: MoveSkip): string {
  if (OUR_SKIP_WORDS.has(s.code)) return SKIP_TEXT[s.code];
  return sentence(s.message) || SKIP_TEXT[s.code] || 'Skipped.';
}

/** The line a skip is listed under: its title, else its folder or id or
 * category (`titleOf` knows the titles of conversations). */
export function skipName(s: MoveSkip, titleOf: (id: string) => string | undefined): string {
  if (s.id) return titleOf(s.id) || s.id.slice(0, 8);
  if (s.item) return customizationLabel(s.item);
  if (s.cwd) return tilde(s.cwd);
  return 'Something';
}

// ───────────────────────────────────────────────────────────── Results

/** A move job's `result`: what it did. */
export interface MoveResult {
  moved: string[];
  copied: string[];
  assigned: string[];
  archived: string[];
  restored: string[];
  skipped: MoveSkip[];
  customizations: CustomizationPlan[];
}

const RESULT_KEYS = [
  'moved',
  'copied',
  'assigned',
  'archived',
  'restored',
  'skipped',
  'customizations',
] as const;

/** A move or copy job's result, or null when it names none of what a move
 * does (an older server's result). */
export function normalizeMoveResult(raw: unknown): MoveResult | null {
  const r = obj(raw);
  if (!r || !RESULT_KEYS.some((k) => k in r)) return null;
  return {
    moved: texts(r['moved']),
    copied: texts(r['copied']),
    assigned: texts(r['assigned']),
    archived: texts(r['archived']),
    restored: texts(r['restored']),
    skipped: list(r['skipped'], skip),
    customizations: list(r['customizations'], customization),
  };
}

/** One line about a finished move or copy ("Archived 3 conversations · 1
 * skipped"), or null for a result that says none of it. */
export function moveResultSummary(job: JobView): string | null {
  const r = normalizeMoveResult(job.result);
  if (!r) return null;
  const done = r.customizations.filter((c) => c.action !== 'skip').length;
  const parts = [
    r.moved.length ? `Moved ${plural(r.moved.length, 'conversation')}` : '',
    r.copied.length ? `Copied ${plural(r.copied.length, 'conversation')}` : '',
    r.assigned.length ? `Assigned ${plural(r.assigned.length, 'conversation')}` : '',
    r.archived.length ? `Archived ${plural(r.archived.length, 'conversation')}` : '',
    r.restored.length ? `Restored ${plural(r.restored.length, 'conversation')}` : '',
    done ? `${plural(done, 'customization')} copied` : '',
    r.skipped.length ? `${r.skipped.length} skipped` : '',
  ].filter(Boolean);
  return parts.join(' · ') || 'Nothing to do: everything was there already.';
}

// ────────────────────────────────────────────────────────────── Errors

/** A refusal of a move, copy or storage change, in words: the backup
 * codes for a storage, the transfer codes, and the rest. */
export function moveErrorText(e: unknown): string {
  if (e instanceof ApiError && e.code === 'no_passphrase') {
    return 'Set a backup passphrase first: every storage opens with it.';
  }
  return backupErrorText(e);
}

// ──────────────────────────────────────────────────────────── Checklists

/** One row of a searchable checklist (a conversation or a project). */
export interface ChecklistRow {
  id: string;
  title: string;
  /** Its id, folder, size: one muted line. */
  sub: string;
  /** "Running", "In storage only": a short tag ('' for none). */
  tag: string;
  /** Lower-case text the search looks in. */
  haystack: string;
}

/** A conversation of an account as a checklist row. */
export function sessionRow(
  s: Pick<SessionSummary, 'id' | 'title' | 'name' | 'cwd' | 'projectName' | 'live'>,
): ChecklistRow {
  const title = s.title?.trim() || s.name?.trim() || s.projectName || s.id.slice(0, 8);
  const sub = [s.id.slice(0, 8), tilde(s.cwd)].filter(Boolean).join(' · ');
  return {
    id: s.id,
    title,
    sub,
    tag: s.live ? 'Running' : '',
    haystack: [title, s.cwd, s.id].join(' ').toLowerCase(),
  };
}

/** A conversation of a set as a checklist row. */
export function indexRow(s: BackupIndexSession): ChecklistRow {
  const title = s.title?.trim() || s.id.slice(0, 8);
  const sub = [s.id.slice(0, 8), tilde(s.cwd), formatBytes(s.remoteBytes)]
    .filter(Boolean)
    .join(' · ');
  return {
    id: s.id,
    title,
    sub,
    tag: s.archived ? 'In storage only' : '',
    haystack: [title, s.cwd ?? '', s.id].join(' ').toLowerCase(),
  };
}

/** The projects of some conversations (one row per folder): the `keep`
 * folders first (listed even when none of the conversations is in them),
 * then the most conversations first. */
export function projectRows(
  sessions: readonly { cwd?: string }[],
  keep: readonly string[] = [],
): ChecklistRow[] {
  const counts = new Map<string, number>();
  for (const s of sessions) {
    if (s.cwd) counts.set(s.cwd, (counts.get(s.cwd) ?? 0) + 1);
  }
  for (const cwd of keep) if (!counts.has(cwd)) counts.set(cwd, 0);
  const first = new Set(keep);
  return [...counts.entries()]
    .sort(
      (a, b) =>
        Number(first.has(b[0])) - Number(first.has(a[0])) ||
        b[1] - a[1] ||
        a[0].localeCompare(b[0]),
    )
    .map(([cwd, n]) => ({
      id: cwd,
      title: tilde(cwd),
      sub: n ? plural(n, 'conversation') : '',
      tag: '',
      haystack: cwd.toLowerCase(),
    }));
}

/** The rows a search shows. */
export function filterRows(rows: readonly ChecklistRow[], query: string): ChecklistRow[] {
  const q = query.trim().toLowerCase();
  return q ? rows.filter((r) => r.haystack.includes(q)) : [...rows];
}
