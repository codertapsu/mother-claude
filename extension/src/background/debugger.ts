// chrome.debugger (an optional permission): trusted input, screenshots,
// console capture, the network log, cookies, eval and dialogs. Attached per
// tab on first use, kept while
// the tab is in its lease, detached when the lease closes. Chrome shows its
// "started debugging this browser" bar while anything is attached; the side
// panel explains it. If the user cancels that bar, we do not attach again in
// that tab until its lease ends.

import { CommandError, messageOf, permissionNeeded } from '../shared/errors.ts';
import { dialogKind, type Dialogs } from './dialogs.ts';
import { hasPermission } from './env.ts';

export interface ConsoleMessage {
  level: 'debug' | 'log' | 'info' | 'warning' | 'error';
  text: string;
  at: number;
  url?: string;
}

const CONSOLE_LIMIT = 1000;
const TEXT_LIMIT = 2000;

/** Requests kept per tab (the oldest go first). */
export const NETWORK_LIMIT = 500;
/** WebSocket frames kept per socket, and the longest payload kept. */
const FRAME_LIMIT = 50;
const FRAME_TEXT = 2000;
/** The longest request body kept as sent (larger ones are fetched on demand). */
const POST_DATA_LIMIT = 64 * 1024;
/** The longest response body handed on. */
export const BODY_LIMIT = 200_000;

/** One request the tab made, as the debugger reported it. */
export interface NetRecord {
  id: string;
  method: string;
  url: string;
  /** What Chrome calls the resource type, lower case: document, xhr, fetch, … */
  type: string;
  /** Epoch ms when it started. */
  at: number;
  status?: number;
  statusText?: string;
  mimeType?: string;
  /** Bytes over the wire. */
  size?: number;
  /** How long it took, in ms (once it ended). */
  ms?: number;
  /** Why it failed ("net::ERR_…", "canceled"). */
  failed?: string;
  fromCache?: boolean;
  redirects?: number;
  protocol?: string;
  remoteAddress?: string;
  requestHeaders?: Record<string, string>;
  responseHeaders?: Record<string, string>;
  postData?: string;
  hasPostData?: boolean;
  /** WebSocket frames, sent (`>`) and received (`<`). */
  frames?: { dir: '>' | '<'; payload: string; at: number }[];
  /** CDP's monotonic start, for the duration. */
  t0?: number;
}

/** A request in a list: what it is and how it went, no headers or bodies. */
export function netSummary(r: NetRecord): Record<string, unknown> {
  const out: Record<string, unknown> = {
    id: r.id,
    method: r.method,
    url: r.url,
    type: r.type,
    at: r.at,
  };
  for (const k of [
    'status',
    'mimeType',
    'size',
    'ms',
    'failed',
    'fromCache',
    'redirects',
  ] as const) {
    if (r[k] !== undefined && r[k] !== false) out[k] = r[k];
  }
  if (r.frames) out['frames'] = r.frames.length;
  return out;
}

/** Whether a request matches a list's filters. */
export function netMatches(
  r: NetRecord,
  f: { filter?: string; type?: string; failed?: boolean; since?: number },
): boolean {
  if (f.filter && !r.url.toLowerCase().includes(f.filter.toLowerCase())) return false;
  if (f.type) {
    const known = [
      'document',
      'xhr',
      'fetch',
      'script',
      'stylesheet',
      'image',
      'font',
      'media',
      'websocket',
    ];
    if (f.type === 'other' ? known.includes(r.type) : r.type !== f.type) return false;
  }
  if (f.failed && !(r.failed || (r.status !== undefined && r.status >= 400))) return false;
  if (f.since !== undefined && r.at < f.since) return false;
  return true;
}

function headerMap(v: unknown): Record<string, string> | undefined {
  if (typeof v !== 'object' || v === null) return undefined;
  const out: Record<string, string> = {};
  for (const [k, val] of Object.entries(v as Record<string, unknown>)) {
    out[k] = typeof val === 'string' ? val : String(val);
  }
  return out;
}

type Params = Record<string, unknown>;

function consoleLevel(type: string): ConsoleMessage['level'] {
  switch (type) {
    case 'warning':
    case 'warn':
      return 'warning';
    case 'error':
    case 'assert':
      return 'error';
    case 'debug':
    case 'verbose':
      return 'debug';
    case 'info':
      return 'info';
    default:
      return 'log';
  }
}

interface RemoteObject {
  type?: string;
  value?: unknown;
  description?: string;
  unserializableValue?: string;
}

function remoteText(o: RemoteObject): string {
  if (o.type === 'string') return String(o.value);
  if (o.value !== undefined) {
    try {
      return JSON.stringify(o.value);
    } catch {
      return String(o.value);
    }
  }
  return o.unserializableValue ?? o.description ?? o.type ?? '';
}

export class DebuggerManager {
  private readonly attached = new Set<number>();
  private readonly cancelled = new Set<number>();
  private readonly consoleBuffers = new Map<number, ConsoleMessage[]>();
  private readonly networks = new Map<
    number,
    { inflight: Map<string, { at: number; url: string }>; last: number; mainFrame: string }
  >();
  /** Each tab's requests, oldest first, and since when it records them. */
  private readonly logs = new Map<number, { records: Map<string, NetRecord>; since: number }>();
  private listening = false;
  private readonly dialogs: Dialogs;
  /** Called when the user cancels the debugging bar (the side panel says why). */
  onCancelled: () => void = () => undefined;

  constructor(dialogs: Dialogs) {
    this.dialogs = dialogs;
  }

  isAttached(tabId: number): boolean {
    return this.attached.has(tabId);
  }

  get attachedCount(): number {
    return this.attached.size;
  }

  async available(): Promise<boolean> {
    return (await hasPermission('debugger')) && typeof chrome.debugger !== 'undefined';
  }

  /** Attach to the tab (once), or explain why it cannot be done. */
  async ensure(tabId: number, why: string): Promise<void> {
    if (!(await hasPermission('debugger'))) {
      throw permissionNeeded(
        'debugger',
        `${why} needs the "debugger" permission: grant it in the Mother Claude side panel`,
      );
    }
    if (typeof chrome.debugger === 'undefined') {
      throw new CommandError('debugger_unavailable', 'this browser has no debugger API');
    }
    this.listen();
    if (this.attached.has(tabId)) return;
    if (this.cancelled.has(tabId)) {
      throw new CommandError(
        'debugger_unavailable',
        'the user cancelled debugging in this tab: it stays off until the session’s lease ends',
      );
    }
    try {
      await chrome.debugger.attach({ tabId }, '1.3');
    } catch (e) {
      const msg = messageOf(e);
      if (/already attached/i.test(msg)) {
        // Ours from before a worker restart? Detaching succeeds only then.
        try {
          await chrome.debugger.detach({ tabId });
          await chrome.debugger.attach({ tabId }, '1.3');
        } catch {
          throw new CommandError(
            'debugger_unavailable',
            'another debugger (DevTools or an extension) is attached to this tab',
          );
        }
      } else {
        throw new CommandError('debugger_unavailable', `cannot debug this tab: ${msg}`);
      }
    }
    this.attached.add(tabId);
    this.networks.set(tabId, { inflight: new Map(), last: Date.now(), mainFrame: '' });
    this.startLog(tabId);
    await Promise.all([
      this.send(tabId, 'Page.enable'),
      this.send(tabId, 'Runtime.enable'),
      this.send(tabId, 'Log.enable'),
      // Requests are counted (network idle) and recorded (browser_network);
      // the browser keeps recent bodies so one can be read on demand.
      this.send(tabId, 'Network.enable', {
        maxTotalBufferSize: 20 * 1024 * 1024,
        maxResourceBufferSize: 5 * 1024 * 1024,
        maxPostDataSize: POST_DATA_LIMIT,
      }).catch(() => this.send(tabId, 'Network.enable')),
    ]).catch(() => undefined);
    const tree = await this.send<{ frameTree?: { frame?: { id?: string } } }>(
      tabId,
      'Page.getFrameTree',
    ).catch(() => ({}) as { frameTree?: { frame?: { id?: string } } });
    const net = this.networks.get(tabId);
    if (net) net.mainFrame = tree.frameTree?.frame?.id ?? '';
  }

  /**
   * Requests open in the tab and when the last one started or ended, or null
   * when the debugger is not attached. A request open for more than 10 s (a
   * long poll, a stream) does not count as keeping the network busy.
   */
  network(tabId: number): { inflight: number; last: number } | null {
    const net = this.networks.get(tabId);
    if (!net || !this.attached.has(tabId)) return null;
    const now = Date.now();
    let inflight = 0;
    for (const r of net.inflight.values()) if (now - r.at < 10_000) inflight++;
    return { inflight, last: net.last };
  }

  async send<T = Params>(tabId: number, method: string, params?: Params): Promise<T> {
    try {
      return (await chrome.debugger.sendCommand({ tabId }, method, params)) as T;
    } catch (e) {
      const msg = messageOf(e);
      if (/not attached|detached/i.test(msg)) this.attached.delete(tabId);
      throw new CommandError('debugger_unavailable', `${method}: ${msg}`);
    }
  }

  /** Start recording the tab's requests (once: a log already there stays). */
  startLog(tabId: number): void {
    if (!this.logs.has(tabId)) this.logs.set(tabId, { records: new Map(), since: Date.now() });
  }

  /** The tab's recorded requests (oldest first) and since when it records. */
  networkLog(tabId: number): { records: NetRecord[]; since: number } | null {
    const log = this.logs.get(tabId);
    return log ? { records: [...log.records.values()], since: log.since } : null;
  }

  request(tabId: number, id: string): NetRecord | null {
    return this.logs.get(tabId)?.records.get(id) ?? null;
  }

  /** Forget what the tab recorded; it goes on recording. */
  clearNetwork(tabId: number): void {
    const log = this.logs.get(tabId);
    if (log) {
      log.records.clear();
      log.since = Date.now();
    }
  }

  /** A response's body: text, or how big a binary one is. */
  async responseBody(
    tabId: number,
    id: string,
  ): Promise<{
    text?: string;
    base64?: boolean;
    size?: number;
    truncated?: boolean;
    unavailable?: string;
  }> {
    try {
      const r = await this.send<{ body?: string; base64Encoded?: boolean }>(
        tabId,
        'Network.getResponseBody',
        { requestId: id },
      );
      const body = r.body ?? '';
      if (r.base64Encoded) return { base64: true, size: Math.floor((body.length * 3) / 4) };
      return body.length > BODY_LIMIT
        ? { text: body.slice(0, BODY_LIMIT), truncated: true, size: body.length }
        : { text: body, size: body.length };
    } catch (e) {
      const msg = messageOf(e).replace(/^Network\.getResponseBody:\s*/, '');
      return {
        unavailable: /No resource|No data found/i.test(msg)
          ? 'the browser no longer has it: load the page again and ask right after'
          : msg,
      };
    }
  }

  /** A request body the event did not carry (a large one). */
  async requestBody(tabId: number, id: string): Promise<string | null> {
    try {
      const r = await this.send<{ postData?: string }>(tabId, 'Network.getRequestPostData', {
        requestId: id,
      });
      return r.postData ?? null;
    } catch {
      return null;
    }
  }

  async detach(tabId: number): Promise<void> {
    this.cancelled.delete(tabId);
    this.consoleBuffers.delete(tabId);
    this.networks.delete(tabId);
    this.logs.delete(tabId);
    if (!this.attached.delete(tabId)) return;
    try {
      await chrome.debugger.detach({ tabId });
    } catch {
      // already gone
    }
  }

  consoleMessages(tabId: number): ConsoleMessage[] {
    return this.consoleBuffers.get(tabId) ?? [];
  }

  private push(tabId: number, m: ConsoleMessage): void {
    const buf = this.consoleBuffers.get(tabId) ?? [];
    buf.push({
      ...m,
      text: m.text.length > TEXT_LIMIT ? `${m.text.slice(0, TEXT_LIMIT)}…` : m.text,
    });
    if (buf.length > CONSOLE_LIMIT) buf.splice(0, buf.length - CONSOLE_LIMIT);
    this.consoleBuffers.set(tabId, buf);
  }

  /** Listeners exist only once the permission does (it is optional). */
  listen(): void {
    if (this.listening || typeof chrome.debugger === 'undefined') return;
    this.listening = true;
    chrome.debugger.onEvent.addListener((source, method, params) => {
      const tabId = source.tabId;
      if (tabId === undefined) return;
      this.onEvent(tabId, method, (params ?? {}) as Params);
    });
    chrome.debugger.onDetach.addListener((source, reason) => {
      const tabId = source.tabId;
      if (tabId === undefined) return;
      this.attached.delete(tabId);
      if (reason === 'canceled_by_user') {
        this.cancelled.add(tabId);
        this.onCancelled();
      }
    });
  }

  /** Keep what a Network event says about its request. */
  record(tabId: number, method: string, p: Params): void {
    const log = this.logs.get(tabId);
    const id = String(p['requestId'] ?? '');
    if (!log || !id) return;
    const records = log.records;
    const get = (): NetRecord | undefined => records.get(id);
    const add = (r: NetRecord): void => {
      records.delete(id);
      records.set(id, r);
      while (records.size > NETWORK_LIMIT) {
        const oldest = records.keys().next().value;
        if (oldest === undefined) break;
        records.delete(oldest);
      }
    };
    const num = (v: unknown): number | undefined =>
      typeof v === 'number' && Number.isFinite(v) ? v : undefined;
    switch (method) {
      case 'Network.requestWillBeSent': {
        const req = (p['request'] ?? {}) as Record<string, unknown>;
        const prev = get();
        // A redirect reuses the request id: the same request, at its new URL.
        if (prev && p['redirectResponse']) {
          prev.url = String(req['url'] ?? prev.url);
          prev.redirects = (prev.redirects ?? 0) + 1;
          prev.requestHeaders = headerMap(req['headers']) ?? prev.requestHeaders;
          return;
        }
        const wall = num(p['wallTime']);
        const post = typeof req['postData'] === 'string' ? (req['postData'] as string) : undefined;
        const r: NetRecord = {
          id,
          method: String(req['method'] ?? 'GET'),
          url: String(req['url'] ?? ''),
          type: String(p['type'] ?? 'Other').toLowerCase(),
          at: wall !== undefined ? Math.round(wall * 1000) : Date.now(),
        };
        const t0 = num(p['timestamp']);
        if (t0 !== undefined) r.t0 = t0;
        const headers = headerMap(req['headers']);
        if (headers) r.requestHeaders = headers;
        if (post !== undefined) r.postData = post.slice(0, POST_DATA_LIMIT);
        if (req['hasPostData'] === true) r.hasPostData = true;
        add(r);
        return;
      }
      case 'Network.requestWillBeSentExtraInfo': {
        // The headers as sent, cookies included (redacted on the desktop).
        const r = get();
        const headers = headerMap(p['headers']);
        if (r && headers) r.requestHeaders = { ...r.requestHeaders, ...headers };
        return;
      }
      case 'Network.responseReceived': {
        const r = get();
        const res = (p['response'] ?? {}) as Record<string, unknown>;
        if (!r) return;
        const status = num(res['status']);
        if (status !== undefined) r.status = status;
        if (typeof res['statusText'] === 'string' && res['statusText'])
          r.statusText = res['statusText'];
        if (typeof res['mimeType'] === 'string') r.mimeType = res['mimeType'];
        if (typeof res['protocol'] === 'string') r.protocol = res['protocol'];
        const ip = res['remoteIPAddress'];
        if (typeof ip === 'string' && ip)
          r.remoteAddress = `${ip}:${String(res['remotePort'] ?? '')}`;
        if (res['fromDiskCache'] === true || res['fromServiceWorker'] === true) r.fromCache = true;
        r.responseHeaders = headerMap(res['headers']) ?? r.responseHeaders;
        if (typeof p['type'] === 'string') r.type = p['type'].toLowerCase();
        return;
      }
      case 'Network.responseReceivedExtraInfo': {
        const r = get();
        const headers = headerMap(p['headers']);
        if (r && headers) r.responseHeaders = { ...r.responseHeaders, ...headers };
        return;
      }
      case 'Network.loadingFinished':
      case 'Network.loadingFailed': {
        const r = get();
        if (!r) return;
        const t = num(p['timestamp']);
        if (t !== undefined && r.t0 !== undefined)
          r.ms = Math.max(0, Math.round((t - r.t0) * 1000));
        const size = num(p['encodedDataLength']);
        if (size !== undefined) r.size = size;
        if (method === 'Network.loadingFailed') {
          r.failed = p['canceled'] === true ? 'canceled' : String(p['errorText'] ?? 'failed');
        }
        return;
      }
      case 'Network.webSocketCreated':
        add({
          id,
          method: 'GET',
          url: String(p['url'] ?? ''),
          type: 'websocket',
          at: Date.now(),
          frames: [],
        });
        return;
      case 'Network.webSocketFrameSent':
      case 'Network.webSocketFrameReceived': {
        const r = get();
        if (!r) return;
        const frame = (p['response'] ?? {}) as Record<string, unknown>;
        const opcode = num(frame['opcode']);
        const payload =
          opcode === 1
            ? String(frame['payloadData'] ?? '')
            : `(binary frame, opcode ${opcode ?? '?'})`;
        r.frames ??= [];
        r.frames.push({
          dir: method === 'Network.webSocketFrameSent' ? '>' : '<',
          payload: payload.length > FRAME_TEXT ? `${payload.slice(0, FRAME_TEXT)}…` : payload,
          at: Date.now(),
        });
        if (r.frames.length > FRAME_LIMIT) r.frames.splice(0, r.frames.length - FRAME_LIMIT);
        return;
      }
      case 'Network.webSocketClosed': {
        const r = get();
        if (r && r.failed === undefined) r.ms = Date.now() - r.at;
        return;
      }
      default:
        return;
    }
  }

  private onEvent(tabId: number, method: string, p: Params): void {
    if (method.startsWith('Network.')) this.record(tabId, method, p);
    switch (method) {
      case 'Page.javascriptDialogOpening':
        this.dialogs.opened(tabId, dialogKind(p['type']), String(p['message'] ?? ''));
        break;
      case 'Page.javascriptDialogClosed':
        this.dialogs.closed(tabId);
        break;
      case 'Network.requestWillBeSent':
      case 'Network.responseReceived':
      case 'Network.loadingFinished':
      case 'Network.loadingFailed': {
        const net = this.networks.get(tabId);
        const id = String(p['requestId'] ?? '');
        if (!net || !id) break;
        if (method === 'Network.requestWillBeSent') {
          // A new document in the main frame: the old page's requests are gone.
          if (p['type'] === 'Document' && net.mainFrame && p['frameId'] === net.mainFrame) {
            net.inflight.clear();
          }
          const url = String((p['request'] as { url?: unknown } | undefined)?.url ?? '');
          // A redirect reuses the request id: still the same open request.
          if (!net.inflight.has(id)) net.inflight.set(id, { at: Date.now(), url });
        } else if (method === 'Network.responseReceived') {
          // A response that becomes a download never reports finishing to the page.
          const headers = ((p['response'] as { headers?: Record<string, string> } | undefined)
            ?.headers ?? {}) as Record<string, string>;
          const disposition = Object.entries(headers).find(
            ([k]) => k.toLowerCase() === 'content-disposition',
          )?.[1];
          if (disposition && /attachment/i.test(disposition)) net.inflight.delete(id);
          break;
        } else net.inflight.delete(id);
        net.last = Date.now();
        break;
      }
      case 'Page.downloadWillBegin': {
        const net = this.networks.get(tabId);
        const url = String(p['url'] ?? '');
        if (net) for (const [id, r] of net.inflight) if (r.url === url) net.inflight.delete(id);
        break;
      }
      case 'Runtime.consoleAPICalled': {
        const args = Array.isArray(p['args']) ? (p['args'] as RemoteObject[]) : [];
        this.push(tabId, {
          level: consoleLevel(String(p['type'] ?? 'log')),
          text: args.map(remoteText).join(' '),
          at: typeof p['timestamp'] === 'number' ? Math.round(p['timestamp']) : Date.now(),
        });
        break;
      }
      case 'Runtime.exceptionThrown': {
        const d = (p['exceptionDetails'] ?? {}) as {
          text?: string;
          exception?: RemoteObject;
          url?: string;
        };
        this.push(tabId, {
          level: 'error',
          text: d.exception?.description ?? d.text ?? 'Uncaught exception',
          at: Date.now(),
          ...(d.url ? { url: d.url } : {}),
        });
        break;
      }
      case 'Log.entryAdded': {
        const e = (p['entry'] ?? {}) as {
          level?: string;
          text?: string;
          url?: string;
          timestamp?: number;
        };
        this.push(tabId, {
          level: consoleLevel(e.level ?? 'info'),
          text: e.text ?? '',
          at: typeof e.timestamp === 'number' ? Math.round(e.timestamp) : Date.now(),
          ...(e.url ? { url: e.url } : {}),
        });
        break;
      }
      default:
        break;
    }
  }
}
