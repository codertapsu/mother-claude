import {
  AccountAttention,
  AccountDefaults,
  AccountKind,
  AccountView,
  AccountsView,
  CaptureState,
  CaptureView,
  CopyConsent,
  SecretStoreKind,
} from './models';
import { normalizeIdentityWarning, normalizeSignIn } from './sign-in';

/**
 * Pure helpers for the Claude accounts (`GET /accounts`, `accounts` frames;
 * docs/ACCOUNTS.md). Everything read from the wire is checked here before
 * the UI sees it, like the plan usage: the registry is new and will grow.
 */

/** The Default account's id. */
export const DEFAULT_ACCOUNT = 'default';

/** Longest account name, in characters (the server's rule). */
export const ACCOUNT_NAME_MAX = 40;

/** The colours the server hands out (the Default's first), offered when
 * choosing one. */
export const ACCOUNT_COLORS: readonly string[] = [
  '#8b5cf6',
  '#0ea5e9',
  '#f59e0b',
  '#10b981',
  '#ef4444',
  '#ec4899',
  '#14b8a6',
  '#6366f1',
  '#84cc16',
];

const KINDS: readonly AccountKind[] = ['directory', 'token'];
const ATTENTION: readonly AccountAttention[] = [
  'tokenMissing',
  'authFailed',
  'signedOut',
  'checkFailed',
];
const CAPTURE_STATES: readonly CaptureState[] = [
  'running',
  'captured',
  'failed',
  'timedOut',
  'cancelled',
];
const SECRET_STORES: readonly SecretStoreKind[] = ['keychain', 'files', 'memory'];
const HEX_COLOR = /^#[0-9a-f]{6}$/i;

/** A `#rrggbb` colour (the only kind accounts have). */
export function isAccountColor(v: unknown): v is string {
  return typeof v === 'string' && HEX_COLOR.test(v);
}

function str(v: unknown): string | undefined {
  return typeof v === 'string' && v.trim() ? v : undefined;
}

function num(v: unknown): number | undefined {
  return typeof v === 'number' && Number.isFinite(v) ? v : undefined;
}

/** A `setup-token` run from the wire, or undefined when it isn't one. */
export function normalizeCapture(raw: unknown): CaptureView | undefined {
  if (!raw || typeof raw !== 'object') return undefined;
  const r = raw as Record<string, unknown>;
  const state = CAPTURE_STATES.find((s) => s === r['state']);
  if (!state) return undefined;
  const url = str(r['url']);
  const error = str(r['error']);
  const finishedAt = num(r['finishedAt']);
  return {
    state,
    // Only an http(s) link is offered as "open this link".
    ...(url && /^https?:\/\//i.test(url) ? { url } : {}),
    asksForCode: r['asksForCode'] === true,
    ...(error ? { error } : {}),
    startedAt: num(r['startedAt']) ?? 0,
    ...(finishedAt ? { finishedAt } : {}),
  };
}

function normalizeAccount(raw: unknown): AccountView | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  const id = str(r['id']);
  const name = str(r['name']);
  const kind = KINDS.find((k) => k === r['kind']);
  if (!id || !name || !kind) return null;
  const attention = ATTENTION.find((a) => a === r['attention']);
  const capture = normalizeCapture(r['capture']);
  const signIn = normalizeSignIn(r['signIn']);
  const identityWarning = normalizeIdentityWarning(r['identityWarning']);
  const live = num(r['live']);
  const color = r['color'];
  const optional = (key: string) => {
    const v = str(r[key]);
    return v ? { [key]: v } : {};
  };
  return {
    id,
    name: name.trim(),
    kind,
    isDefault: r['isDefault'] === true || id === DEFAULT_ACCOUNT,
    ...optional('dir'),
    color: isAccountColor(color) ? color.toLowerCase() : ACCOUNT_COLORS[0],
    createdAt: num(r['createdAt']) ?? 0,
    signedIn: r['signedIn'] === true,
    ...optional('email'),
    ...optional('subscription'),
    ...optional('authMethod'),
    ...optional('authError'),
    ...(attention ? { attention } : {}),
    checking: r['checking'] === true,
    signingIn: r['signingIn'] === true,
    tokenStored: r['tokenStored'] === true,
    ...optional('hooksPlugin'),
    ...(live !== undefined && live >= 0 ? { live } : {}),
    ...(capture ? { capture } : {}),
    ...(signIn ? { signIn } : {}),
    ...(identityWarning ? { identityWarning } : {}),
  } as AccountView;
}

function normalizeDefaults(raw: unknown): AccountDefaults {
  const r = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>;
  const byProject: Record<string, string> = {};
  const map = r['byProject'];
  if (map && typeof map === 'object' && !Array.isArray(map)) {
    for (const [dir, id] of Object.entries(map as Record<string, unknown>)) {
      if (dir.startsWith('/') && typeof id === 'string' && id) byProject[dir] = id;
    }
  }
  return {
    newSession: str(r['newSession']) ?? DEFAULT_ACCOUNT,
    bridge: str(r['bridge']) ?? DEFAULT_ACCOUNT,
    byProject,
  };
}

function normalizeConsent(raw: unknown): CopyConsent[] {
  if (!Array.isArray(raw)) return [];
  return raw
    .map((c) => (c && typeof c === 'object' ? (c as Record<string, unknown>) : {}))
    .map((c) => ({ from: str(c['from']), to: str(c['to']) }))
    .filter((c): c is CopyConsent => !!c.from && !!c.to);
}

/** The accounts from the wire made safe to render, or null when it isn't a
 * registry at all. Malformed accounts are dropped (and duplicates of an id
 * already seen); the Default comes first. */
export function normalizeAccounts(raw: unknown): AccountsView | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  if (!Array.isArray(r['accounts'])) return null;
  const seen = new Set<string>();
  const accounts: AccountView[] = [];
  for (const item of r['accounts']) {
    const a = normalizeAccount(item);
    if (!a || seen.has(a.id)) continue;
    seen.add(a.id);
    accounts.push(a);
  }
  accounts.sort((a, b) => Number(b.isDefault) - Number(a.isDefault));
  const secretStore = SECRET_STORES.find((s) => s === r['secretStore']);
  return {
    accounts,
    defaults: normalizeDefaults(r['defaults']),
    copyConsent: normalizeConsent(r['copyConsent']),
    ...(secretStore ? { secretStore } : {}),
    tokenCapture: r['tokenCapture'] === true,
  };
}

/** `/a/b/` → `/a/b` (the root stays `/`). */
function trimDir(dir: string): string {
  const t = dir.trim().replace(/\/+$/, '');
  return t || '/';
}

/** `dir` is `cwd` or one of its ancestors (whole path components). */
export function dirContains(dir: string, cwd: string): boolean {
  const d = trimDir(dir);
  const c = trimDir(cwd);
  return d === '/' || c === d || c.startsWith(`${d}/`);
}

export interface AccountChoice {
  id: string;
  /** `project`: a per-project rule matched (`dir`); `default`: the default
   * for new sessions. */
  via: 'project' | 'default';
  dir?: string;
}

/** The account a session started in `cwd` runs under when nobody picks
 * one, as the server decides it: the longest project directory containing
 * `cwd` whose account still exists, else the default for new sessions. */
export function accountForCwd(view: AccountsView | null, cwd: string): AccountChoice {
  if (!view) return { id: DEFAULT_ACCOUNT, via: 'default' };
  const known = new Set(view.accounts.map((a) => a.id));
  let best: { dir: string; id: string; depth: number } | null = null;
  if (cwd.trim().startsWith('/')) {
    for (const [dir, id] of Object.entries(view.defaults.byProject)) {
      if (!known.has(id) || !dirContains(dir, cwd)) continue;
      const depth = trimDir(dir).split('/').filter(Boolean).length;
      if (!best || depth > best.depth) best = { dir, id, depth };
    }
  }
  if (best) return { id: best.id, via: 'project', dir: best.dir };
  const fallback = known.has(view.defaults.newSession) ? view.defaults.newSession : DEFAULT_ACCOUNT;
  return { id: fallback, via: 'default' };
}

/** The config directory an account's sessions live in, as an account id: a
 * token account (the unregistered-token pseudo account too) uses the
 * Default's. */
export function homeOf(a: { id: string; kind: AccountKind | string }): string {
  return a.kind === 'token' ? DEFAULT_ACCOUNT : a.id;
}

/** Continuing a conversation of `from` on `to` needs it copied into another
 * directory first (docs/ACCOUNTS.md §5.9: "continue on" a directory account
 * copies it there, once the desktop allowed copies between the two). */
export function needsCopy(
  from: { id: string; kind: AccountKind | string },
  to: { id: string; kind: AccountKind | string },
): boolean {
  return homeOf(from) !== homeOf(to);
}

/** Where a conversation or project of `from` can be moved or copied: the
 * directory accounts (the Default among them) whose directory is another
 * one. A token account is no target of its own: it shares the Default's
 * directory, so moving there is moving to the Default. */
export function transferTargets<T extends { id: string; kind: AccountKind | string }>(
  accounts: readonly T[],
  from: { id: string; kind: AccountKind | string },
): T[] {
  return accounts.filter((a) => a.kind === 'directory' && needsCopy(from, a));
}

/** The token accounts that use `id`'s directory (only the Default has any). */
export function sharersOf<T extends { id: string; kind: AccountKind | string; name: string }>(
  accounts: readonly T[],
  id: string,
): T[] {
  return accounts.filter((a) => a.id !== id && a.kind === 'token' && homeOf(a) === id);
}

/** Work can run under this account right now: its token is here and it is
 * not known to be signed out. */
export function accountCanRun(a: Pick<AccountView, 'attention'>): boolean {
  return a.attention !== 'tokenMissing' && a.attention !== 'signedOut';
}

/** Why a name can't be used ('' when it can): the server's rules. */
export function accountNameProblem(
  name: string,
  accounts: readonly Pick<AccountView, 'id' | 'name'>[],
  exceptId?: string,
): string {
  const n = name.trim();
  if (!n) return 'Enter a name.';
  if ([...n].length > ACCOUNT_NAME_MAX) return `At most ${ACCOUNT_NAME_MAX} characters.`;
  if (n.startsWith('.')) return "A name can't start with a dot.";
  // eslint-disable-next-line no-control-regex
  if (/[/\\:*?"<>|\u0000-\u001f\u007f]/.test(n)) {
    return 'A name can\'t contain / \\ : * ? " < > |';
  }
  const lower = n.toLowerCase();
  if (accounts.some((a) => a.id !== exceptId && a.name.trim().toLowerCase() === lower)) {
    return 'Another account already has this name.';
  }
  return '';
}

/** A directory to suggest for a new directory account: `~/.claude-work`. */
export function suggestAccountDir(name: string): string {
  const slug = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return `~/.claude-${slug || 'work'}`;
}
