// The wire protocol between the extension and the desktop, v1
// (docs/BROWSER-PROTOCOL.md). Decoding is tolerant: a field we don't know is
// ignored, a frame type we don't know is answered with `unknown_frame`.

export const PROTOCOL_VERSION = 1;
/** Largest frame either side accepts (a screenshot). */
export const MAX_FRAME_BYTES = 8 * 1024 * 1024;
export const DEFAULT_TIMEOUT_MS = 30_000;
/** The extension answers `timeout` this long before the desktop's deadline. */
export const DEADLINE_MARGIN_MS = 500;

export type SiteMode = 'off' | 'read' | 'act';
export const SITE_MODES: readonly SiteMode[] = ['off', 'read', 'act'];

export const ERROR_CODES = [
  'stale_ref',
  'not_found',
  'timeout',
  'site_off',
  'site_read_only',
  'no_tab',
  'no_lease',
  'stopped',
  'dialog_open',
  'navigation',
  'debugger_unavailable',
  'permission_needed',
  'unsupported',
  'internal',
  // Not a command error in the table, but the protocol's own name for a frame
  // over MAX_FRAME_BYTES; used when a result would be too big to send.
  'frame_too_large',
  // Added by the desktop side: `type`/`fill` with `refuseSecret: true` into a
  // field whose value the extension redacts.
  'secret_field',
] as const;
export type ErrorCode = (typeof ERROR_CODES)[number];

export type TabGroupColor =
  'grey' | 'blue' | 'red' | 'yellow' | 'green' | 'pink' | 'purple' | 'cyan' | 'orange';
export const TAB_GROUP_COLORS: readonly TabGroupColor[] = [
  'grey',
  'blue',
  'red',
  'yellow',
  'green',
  'pink',
  'purple',
  'cyan',
  'orange',
];

export interface BrowserInfo {
  name: string;
  version: string;
}

export interface Settings {
  screenshots?: boolean;
  defaultMode?: SiteMode;
  [key: string]: unknown;
}

export interface SessionRef {
  id: string;
  title: string;
}

/** A lease as the desktop lists it in `state.leases` (tolerant: only `lease` is required). */
export interface LeaseInfo {
  lease: string;
  session?: SessionRef;
  color?: string;
  groupId?: number;
}

export interface PendingItem {
  id: string;
  lease?: string;
  session?: string;
  summary?: string;
  detail?: string;
  dangerous?: boolean;
  createdAt?: number | string;
}

export interface WireError {
  code: string;
  message: string;
  data?: unknown;
}

// ---------------------------------------------------------------- app → ext

export interface PairedFrame {
  type: 'paired';
  id: string;
  token: string;
  browserId: string;
}
export interface WelcomeFrame {
  type: 'welcome';
  id: string;
  browserId?: string;
  protocol?: number;
  appVersion?: string;
  siteModes?: Record<string, SiteMode>;
  settings?: Settings;
}
export interface StateFrame {
  type: 'state';
  siteModes?: Record<string, SiteMode>;
  settings?: Settings;
  leases?: LeaseInfo[];
  pending?: PendingItem[];
}
export interface OkFrame {
  type: 'ok';
  id: string;
  /** What the request asked for (`sessions`, `session.connect`, …): an addition to v1. */
  data?: Record<string, unknown>;
}
export interface PongFrame {
  type: 'pong';
  id?: string;
}
export interface ErrorFrame {
  type: 'error';
  id?: string;
  code: string;
  message?: string;
}
export interface LeaseOpenFrame {
  type: 'lease.open';
  id: string;
  lease: string;
  session: SessionRef;
  color?: string;
}
export interface LeaseCloseFrame {
  type: 'lease.close';
  id: string;
  lease: string;
  closeTabs: boolean;
}
export interface CommandFrame {
  type: 'command';
  id: string;
  lease: string;
  tabId?: number;
  action: string;
  params: Record<string, unknown>;
  timeoutMs: number;
}

export type IncomingFrame =
  | PairedFrame
  | WelcomeFrame
  | StateFrame
  | OkFrame
  | PongFrame
  | ErrorFrame
  | LeaseOpenFrame
  | LeaseCloseFrame
  | CommandFrame;

// ---------------------------------------------------------------- ext → app

export interface PairRequest {
  type: 'pair';
  id: string;
  code: string;
  name: string;
  browser: BrowserInfo;
  extensionVersion: string;
}
export interface HelloRequest {
  type: 'hello';
  id: string;
  token: string;
  extensionVersion: string;
  browser: BrowserInfo;
  capabilities: Record<string, boolean>;
  /** Optional addition: the optional permissions and origins granted now. */
  granted?: string[];
}
export interface ResultFrame {
  type: 'result';
  id: string;
  ok: boolean;
  data?: unknown;
  error?: WireError;
}

export type Decoded =
  { ok: true; frame: IncomingFrame } | { ok: false; reply: ErrorFrame | null; reason: string };

const isObject = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);
const str = (v: unknown): string | undefined => (typeof v === 'string' ? v : undefined);
const int = (v: unknown): number | undefined =>
  typeof v === 'number' && Number.isInteger(v) ? v : undefined;

export function isSiteMode(v: unknown): v is SiteMode {
  return v === 'off' || v === 'read' || v === 'act';
}

function siteModes(v: unknown): Record<string, SiteMode> | undefined {
  if (!isObject(v)) return undefined;
  const out: Record<string, SiteMode> = {};
  for (const [k, m] of Object.entries(v)) if (isSiteMode(m)) out[k] = m;
  return out;
}

function settings(v: unknown): Settings | undefined {
  if (!isObject(v)) return undefined;
  const out: Settings = { ...v };
  if (typeof v['screenshots'] !== 'boolean') delete out.screenshots;
  if (!isSiteMode(v['defaultMode'])) delete out.defaultMode;
  return out;
}

function session(v: unknown): SessionRef | undefined {
  if (!isObject(v)) return undefined;
  const id = str(v['id']) ?? '';
  const title = str(v['title']) ?? '';
  return { id, title };
}

function leases(v: unknown): LeaseInfo[] | undefined {
  if (!Array.isArray(v)) return undefined;
  const out: LeaseInfo[] = [];
  for (const item of v) {
    if (!isObject(item)) continue;
    const lease = str(item['lease']) ?? str(item['id']);
    if (!lease) continue;
    const info: LeaseInfo = { lease };
    const s = session(item['session']);
    if (s) info.session = s;
    const color = str(item['color']);
    if (color) info.color = color;
    const groupId = int(item['groupId']);
    if (groupId !== undefined) info.groupId = groupId;
    out.push(info);
  }
  return out;
}

function pending(v: unknown): PendingItem[] | undefined {
  if (!Array.isArray(v)) return undefined;
  const out: PendingItem[] = [];
  for (const item of v) {
    if (!isObject(item)) continue;
    const id = str(item['id']);
    if (!id) continue;
    const p: PendingItem = { id };
    const lease = str(item['lease']);
    if (lease) p.lease = lease;
    const sess = item['session'];
    if (typeof sess === 'string') p.session = sess;
    else if (isObject(sess) && typeof sess['title'] === 'string') p.session = sess['title'];
    const summary = str(item['summary']);
    if (summary) p.summary = summary;
    const detail = str(item['detail']);
    if (detail) p.detail = detail;
    if (typeof item['dangerous'] === 'boolean') p.dangerous = item['dangerous'];
    const created = item['createdAt'];
    if (typeof created === 'number' || typeof created === 'string') p.createdAt = created;
    out.push(p);
  }
  return out;
}

function bad(id: string | undefined, reason: string): Decoded {
  const reply: ErrorFrame = { type: 'error', code: 'bad_frame', message: reason };
  if (id !== undefined) reply.id = id;
  return { ok: false, reply, reason };
}

/** Parse one text frame from the desktop. */
export function decodeIncoming(text: string): Decoded {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return bad(undefined, 'not JSON');
  }
  if (!isObject(raw)) return bad(undefined, 'not a JSON object');
  const type = str(raw['type']);
  const id = str(raw['id']);
  if (!type) return bad(id, 'no "type"');

  switch (type) {
    case 'paired': {
      const token = str(raw['token']);
      const browserId = str(raw['browserId']) ?? '';
      if (!id || !token) return bad(id, 'paired needs id and token');
      return { ok: true, frame: { type, id, token, browserId } };
    }
    case 'welcome': {
      if (!id) return bad(id, 'welcome needs id');
      const frame: WelcomeFrame = { type, id };
      const browserId = str(raw['browserId']);
      if (browserId) frame.browserId = browserId;
      const protocol = int(raw['protocol']);
      if (protocol !== undefined) frame.protocol = protocol;
      const appVersion = str(raw['appVersion']);
      if (appVersion) frame.appVersion = appVersion;
      const modes = siteModes(raw['siteModes']);
      if (modes) frame.siteModes = modes;
      const s = settings(raw['settings']);
      if (s) frame.settings = s;
      return { ok: true, frame };
    }
    case 'state': {
      const frame: StateFrame = { type };
      const modes = siteModes(raw['siteModes']);
      if (modes) frame.siteModes = modes;
      const s = settings(raw['settings']);
      if (s) frame.settings = s;
      const l = leases(raw['leases']);
      if (l) frame.leases = l;
      const p = pending(raw['pending']);
      if (p) frame.pending = p;
      return { ok: true, frame };
    }
    case 'ok': {
      if (!id) return bad(id, 'ok needs id');
      const frame: OkFrame = { type, id };
      if (isObject(raw['data'])) frame.data = raw['data'];
      return { ok: true, frame };
    }
    case 'pong':
      return { ok: true, frame: id ? { type, id } : { type } };
    case 'error': {
      const frame: ErrorFrame = { type, code: str(raw['code']) ?? 'unknown' };
      if (id) frame.id = id;
      const message = str(raw['message']);
      if (message) frame.message = message;
      return { ok: true, frame };
    }
    case 'lease.open': {
      const lease = str(raw['lease']);
      if (!id || !lease) return bad(id, 'lease.open needs id and lease');
      const frame: LeaseOpenFrame = {
        type,
        id,
        lease,
        session: session(raw['session']) ?? { id: '', title: '' },
      };
      const color = str(raw['color']);
      if (color) frame.color = color;
      return { ok: true, frame };
    }
    case 'lease.close': {
      const lease = str(raw['lease']);
      if (!id || !lease) return bad(id, 'lease.close needs id and lease');
      return { ok: true, frame: { type, id, lease, closeTabs: raw['closeTabs'] === true } };
    }
    case 'command': {
      const lease = str(raw['lease']);
      const action = str(raw['action']);
      if (!id || !lease || !action) return bad(id, 'command needs id, lease and action');
      const params = raw['params'] === undefined ? {} : raw['params'];
      if (!isObject(params)) return bad(id, 'command params must be an object');
      const t = raw['timeoutMs'];
      const timeoutMs =
        typeof t === 'number' && Number.isFinite(t) && t > 0
          ? Math.min(t, 10 * 60_000)
          : DEFAULT_TIMEOUT_MS;
      const frame: CommandFrame = { type, id, lease, action, params, timeoutMs };
      const tabId = int(raw['tabId']);
      if (tabId !== undefined) frame.tabId = tabId;
      else if (raw['tabId'] !== undefined && raw['tabId'] !== null)
        return bad(id, 'tabId must be an integer');
      return { ok: true, frame };
    }
    default: {
      const reply: ErrorFrame = {
        type: 'error',
        code: 'unknown_frame',
        message: `unknown frame type "${type}"`,
      };
      if (id) reply.id = id;
      return { ok: false, reply, reason: reply.message ?? '' };
    }
  }
}

/** UTF-8 byte length of a string, counted without encoding it. */
export function utf8Length(s: string): number {
  let n = 0;
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (c < 0x80) n += 1;
    else if (c < 0x800) n += 2;
    else if (c >= 0xd800 && c <= 0xdbff && i + 1 < s.length) {
      n += 4; // a surrogate pair is one 4-byte code point
      i++;
    } else n += 3;
  }
  return n;
}

export type Encoded = { ok: true; text: string } | { ok: false; bytes: number };

/** Serialize an outgoing frame, refusing anything over MAX_FRAME_BYTES. */
export function encodeFrame(frame: object): Encoded {
  const text = JSON.stringify(frame);
  // Every UTF-16 unit is at most 3 bytes: short frames need no counting.
  if (text.length * 3 <= MAX_FRAME_BYTES) return { ok: true, text };
  const bytes = utf8Length(text);
  return bytes <= MAX_FRAME_BYTES ? { ok: true, text } : { ok: false, bytes };
}

/** A result frame for a failed command. */
export function errorResult(id: string, error: WireError): ResultFrame {
  const frame: ResultFrame = { type: 'result', id, ok: false, error };
  // `permission_needed` names the permission in `data` as well as in
  // `error.data`, so a desktop reading either finds it.
  if (error.code === 'permission_needed' && error.data !== undefined) frame.data = error.data;
  return frame;
}

export function okResult(id: string, data: unknown): ResultFrame {
  return { type: 'result', id, ok: true, data: data ?? {} };
}
