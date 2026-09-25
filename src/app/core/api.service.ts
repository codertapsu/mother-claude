import { Injectable, inject, signal } from '@angular/core';

import { AuthService } from './auth.service';
import { ConfigService } from './config.service';
import { activeMock } from './mock/mock-api';
import {
  BridgeDefaults,
  BridgeStatus,
  CommandInfo,
  ContinueBody,
  DeviceInfo,
  DevicesList,
  DeviceUpdate,
  DiffBase,
  Fleet,
  ForeignControlStatus,
  GitOverview,
  Inbox,
  KeyName,
  LaunchDefaults,
  LifecycleAction,
  MeInfo,
  MessageBody,
  MessageResult,
  PairCode,
  PairCodeRequest,
  PairExchangeBody,
  PairExchangeResult,
  PairPendingStatus,
  PermissionMode,
  ProjectInfo,
  PushPrefs,
  PushSubscriptionBody,
  RemoteInfo,
  RemoteUpdate,
  RespondBody,
  ServicesInfo,
  SessionAgents,
  SessionDetail,
  SessionsPage,
  SessionsQuery,
  SpawnBody,
  TranscriptPage,
  TranscriptQuery,
} from './models';

/**
 * ApiService — the one typed REST client for the desktop webview and phone
 * browsers (design contract §4.5, §2.6, §2.8, §6). Every method returns a
 * Promise and rejects with an {@link ApiError}.
 *
 * Transport behaviour (applies to every call):
 * - `Authorization: Bearer <token>` whenever this client has a token.
 * - `ngrok-skip-browser-warning: 1` on every browser request, so a free ngrok
 *   tunnel's interstitial doesn't swallow API calls. (Skipped in the desktop
 *   webview, which always talks to localhost and never through a tunnel.)
 * - An HTML answer to an /api call means a tunnel interstitial is in the way:
 *   the call rejects with `kind: 'interstitial'` and {@link interstitial} flips
 *   to true (the shell tells the user to open the page once and reload).
 * - 401 ⇒ `kind: 'unauthorized'` and {@link AuthService.handleUnauthorized}
 *   (phones become "unpaired" and are routed to /pair).
 * - Error bodies `{"error":{"code","message","field?"}}` become
 *   `ApiError.code/message/field`; 429 carries `retryAfter` (seconds).
 * - In dev mock mode (`?mock=1`) requests go to the fixture backend instead.
 *
 * Methods:
 *   Data      listSessions(q) · getSession(id) · getTranscript(id, q) ·
 *             fetchMedia(ref) · getSessionAgents(id) · getFleet() ·
 *             getTaskOutput(id, taskId) · getDiff(id, base?) ·
 *             getFilePatch(id, path, base?) · getFiles(id, q) ·
 *             getCommands(id) · getTerminal(id) · getProjects() ·
 *             getDefaults() · getInbox() · getServices()
 *   Control   spawnSession(body) · sendMessage(id, body) ·
 *             cancelQueued(id, messageId) · interrupt(id) · setModel(id, m) ·
 *             setMode(id, mode) · setEffort(id, e) · rename(id, name) ·
 *             stopTask(id, taskId) · respond(id, body) ·
 *             continueSession(id, body) · sendKey(id, key) ·
 *             lifecycle(id, action)
 *   Pairing   createPairCode(req) · exchangePairCode(body) ·
 *             pollPairPending(pendingId, secret) · getMe()
 *   Devices   listDevices() · approvePair(id) · denyPair(id) ·
 *             revokeDevice(id) · updateDevice(id, patch)
 *   Remote    getRemote() · updateRemote(patch)
 *   Foreign   getForeignControl() · installForeignControl() ·
 *             uninstallForeignControl()
 *   Push      getVapidKey() · pushSubscribe(sub) · pushUnsubscribe(endpoint) ·
 *             pushTest() · getPushPrefs() · setPushPrefs(prefs)
 *   Bridge    getBridge() · startBridge(defaults) · stopBridge() ·
 *             setBridgeThread(threadId)
 */
@Injectable({ providedIn: 'root' })
export class ApiService {
  private config = inject(ConfigService);
  private auth = inject(AuthService);

  /** True once an /api call came back as an HTML page (tunnel interstitial). */
  readonly interstitial = signal(false);

  // ───────────────────────────────────────────────────────────────── Data

  listSessions(q: SessionsQuery = {}): Promise<SessionsPage> {
    return this.req<SessionsPage>(`/sessions${query({ ...q })}`);
  }

  getSession(id: string): Promise<SessionDetail> {
    return this.req<SessionDetail>(`/sessions/${enc(id)}`);
  }

  /** Newest-last page of normalised events; page back with `before`. */
  getTranscript(id: string, q: TranscriptQuery = {}): Promise<TranscriptPage> {
    return this.req<TranscriptPage>(`/sessions/${enc(id)}/transcript${query({ ...q })}`);
  }

  /** Fetch an image referenced by a UiEvent (`image.ref`), with auth. Use
   * `URL.createObjectURL` on the result; revoke it when done. */
  async fetchMedia(ref: string): Promise<Blob> {
    const path = ref.startsWith('/api/') ? ref.slice(4) : ref;
    const res = await this.send(path, { method: 'GET' }, { accept: '*/*' });
    return res.blob();
  }

  getSessionAgents(id: string): Promise<SessionAgents> {
    return this.req<SessionAgents>(`/sessions/${enc(id)}/agents`);
  }

  /** Every running agent, grouped by session. */
  getFleet(): Promise<Fleet> {
    return this.req<Fleet>('/agents');
  }

  /** Tail of a background task's output (plain text). */
  async getTaskOutput(id: string, taskId: string): Promise<string> {
    const res = await this.send(`/sessions/${enc(id)}/tasks/${enc(taskId)}/output`, {
      method: 'GET',
    });
    const text = await res.text();
    if ((res.headers.get('content-type') ?? '').includes('json')) {
      try {
        const body = JSON.parse(text) as { text?: unknown; output?: unknown };
        if (typeof body.text === 'string') return body.text;
        if (typeof body.output === 'string') return body.output;
      } catch {
        /* not JSON after all */
      }
    }
    return text;
  }

  getDiff(id: string, base?: DiffBase): Promise<GitOverview> {
    return this.req<GitOverview>(`/sessions/${enc(id)}/diff${query({ base })}`);
  }

  getFilePatch(id: string, path: string, base?: DiffBase): Promise<{ patch: string | null }> {
    return this.req(`/sessions/${enc(id)}/file-patch${query({ path, base })}`);
  }

  /** `@` autocomplete: files in the session's cwd (fuzzy, ≤ 50). */
  getFiles(id: string, q: string): Promise<{ files: string[] }> {
    return this.req(`/sessions/${enc(id)}/files${query({ q })}`);
  }

  /** `/` palette. */
  getCommands(id: string): Promise<{ commands: CommandInfo[] }> {
    return this.req(`/sessions/${enc(id)}/commands`);
  }

  /** tmux pane capture / background job log. */
  getTerminal(id: string): Promise<{ text: string }> {
    return this.req(`/sessions/${enc(id)}/terminal`);
  }

  /** Projects for the New session picker. */
  getProjects(): Promise<{ projects: ProjectInfo[] }> {
    return this.req('/projects');
  }

  /** Launch defaults + available models/efforts (from the user's own settings). */
  getDefaults(): Promise<LaunchDefaults> {
    return this.req<LaunchDefaults>('/defaults');
  }

  /** Every pending item across sessions. */
  getInbox(): Promise<Inbox> {
    return this.req<Inbox>('/inbox');
  }

  /** Daemon + MCP inventory (legacy Services screen). */
  getServices(): Promise<ServicesInfo> {
    return this.req<ServicesInfo>('/services');
  }

  // ────────────────────────────────────────────────────────────── Control

  /** Spawn an owned session (`bypassPermissions` needs a local/dangerous client). */
  spawnSession(body: SpawnBody): Promise<{ id: string }> {
    return this.post(`/sessions`, body);
  }

  sendMessage(id: string, body: MessageBody): Promise<MessageResult> {
    return this.post(`/sessions/${enc(id)}/message`, body);
  }

  cancelQueued(id: string, messageId: string): Promise<void> {
    return this.req(`/sessions/${enc(id)}/queue/${enc(messageId)}`, { method: 'DELETE' });
  }

  interrupt(id: string): Promise<void> {
    return this.post(`/sessions/${enc(id)}/interrupt`, {});
  }

  setModel(id: string, model: string): Promise<void> {
    return this.post(`/sessions/${enc(id)}/model`, { model });
  }

  setMode(id: string, mode: PermissionMode): Promise<void> {
    return this.post(`/sessions/${enc(id)}/mode`, { mode });
  }

  setEffort(id: string, effort: string): Promise<void> {
    return this.post(`/sessions/${enc(id)}/effort`, { effort });
  }

  rename(id: string, name: string): Promise<void> {
    return this.post(`/sessions/${enc(id)}/rename`, { name });
  }

  stopTask(id: string, taskId: string): Promise<void> {
    return this.post(`/sessions/${enc(id)}/tasks/${enc(taskId)}/stop`, {});
  }

  /** Answer a PendingItem (permission / question / plan / elicitation).
   * A late answer rejects with status 409, code `already_decided`. */
  respond(id: string, body: RespondBody): Promise<void> {
    return this.post(`/sessions/${enc(id)}/respond`, body);
  }

  /** Take over / fork a session into an owned one. Returns the id to open
   * (a live foreign session is always forked into a new id). */
  continueSession(id: string, body: ContinueBody = {}): Promise<{ id: string }> {
    return this.post(`/sessions/${enc(id)}/continue`, body);
  }

  /** tmux sessions only (local / allow-dangerous clients). */
  sendKey(id: string, key: KeyName): Promise<void> {
    return this.post(`/sessions/${enc(id)}/keys`, { key });
  }

  lifecycle(id: string, action: LifecycleAction): Promise<void> {
    return this.post(`/sessions/${enc(id)}/${action}`, {});
  }

  // ────────────────────────────────────────────────────────────── Pairing

  /** Desktop: mint a single-use pairing code + QR for LAN or internet. */
  createPairCode(req: PairCodeRequest): Promise<PairCode> {
    return this.post('/pair/codes', req);
  }

  /** Phone: trade a pairing code for a device token (no auth needed).
   * 201 ⇒ `{status:'paired'}`; 202 ⇒ `{status:'pending'}` (poll it). */
  async exchangePairCode(body: PairExchangeBody): Promise<PairExchangeResult> {
    const res = await this.send(
      '/pair/exchange',
      { method: 'POST', body: JSON.stringify(body) },
      { auth: false },
    );
    const data = (await readJson(res)) as Record<string, unknown> | null;
    if (res.status === 202 || (data && typeof data['pendingId'] === 'string')) {
      return {
        status: 'pending',
        pendingId: String(data?.['pendingId'] ?? ''),
        pollSecret: String(data?.['pollSecret'] ?? ''),
        fingerprint: String(data?.['fingerprint'] ?? ''),
      };
    }
    return {
      status: 'paired',
      deviceId: String(data?.['deviceId'] ?? ''),
      token: String(data?.['token'] ?? ''),
      name: String(data?.['name'] ?? body.deviceName),
    };
  }

  /** Phone: poll a pending internet pairing (the token is returned once). */
  pollPairPending(pendingId: string, secret: string): Promise<PairPendingStatus> {
    return this.req(`/pair/pending/${enc(pendingId)}`, {
      method: 'GET',
      headers: { 'X-Pair-Secret': secret },
      auth: false,
    });
  }

  getMe(): Promise<MeInfo> {
    return this.req<MeInfo>('/me');
  }

  // ────────────────────────────────────────────────────────────── Devices

  listDevices(): Promise<DevicesList> {
    return this.req<DevicesList>('/devices');
  }

  approvePair(pendingId: string): Promise<void> {
    return this.post(`/devices/pending/${enc(pendingId)}/approve`, {});
  }

  denyPair(pendingId: string): Promise<void> {
    return this.post(`/devices/pending/${enc(pendingId)}/deny`, {});
  }

  /** Revoking closes the device's sockets and drops its push subscriptions. */
  revokeDevice(id: string): Promise<void> {
    return this.req(`/devices/${enc(id)}`, { method: 'DELETE' });
  }

  updateDevice(id: string, patch: DeviceUpdate): Promise<DeviceInfo | null> {
    return this.req(`/devices/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(patch) });
  }

  // ─────────────────────────────────────────────────────────────── Remote

  getRemote(): Promise<RemoteInfo> {
    return this.req<RemoteInfo>('/remote');
  }

  /** Resolves to the updated info when the server returns it, else null. */
  updateRemote(patch: RemoteUpdate): Promise<RemoteInfo | null> {
    return this.req(`/remote`, { method: 'PUT', body: JSON.stringify(patch) });
  }

  // ───────────────────────────────────────────────────── Foreign control

  getForeignControl(): Promise<ForeignControlStatus> {
    return this.req<ForeignControlStatus>('/control/foreign');
  }

  installForeignControl(): Promise<ForeignControlStatus | null> {
    return this.post('/control/foreign/install', {});
  }

  uninstallForeignControl(): Promise<ForeignControlStatus | null> {
    return this.post('/control/foreign/uninstall', {});
  }

  // ───────────────────────────────────────────────────────────────── Push

  getVapidKey(): Promise<{ publicKey: string }> {
    return this.req('/push/vapid');
  }

  pushSubscribe(sub: PushSubscriptionBody): Promise<void> {
    return this.post('/push/subscribe', sub);
  }

  pushUnsubscribe(endpoint: string): Promise<void> {
    return this.req('/push/subscribe', { method: 'DELETE', body: JSON.stringify({ endpoint }) });
  }

  pushTest(): Promise<void> {
    return this.post('/push/test', {});
  }

  getPushPrefs(): Promise<PushPrefs> {
    return this.req<PushPrefs>('/push/prefs');
  }

  setPushPrefs(prefs: PushPrefs): Promise<PushPrefs | null> {
    return this.req('/push/prefs', { method: 'PUT', body: JSON.stringify(prefs) });
  }

  // ─────────────────────────────────────── The Claude HTTP bridge (/bridge)

  getBridge(): Promise<BridgeStatus> {
    return this.req<BridgeStatus>('/bridge');
  }

  /** Start the API with these defaults. Returns the new status. */
  startBridge(defaults: BridgeDefaults): Promise<BridgeStatus> {
    return this.post<BridgeStatus>('/bridge/start', { defaults });
  }

  stopBridge(): Promise<BridgeStatus> {
    return this.post<BridgeStatus>('/bridge/stop', {});
  }

  /** Change which conversation unaddressed messages join. */
  setBridgeThread(threadId: string | null): Promise<BridgeStatus> {
    return this.post<BridgeStatus>('/bridge/active-thread', { threadId });
  }

  // ──────────────────────────────────────────────────────────── Transport

  private post<T>(path: string, body: unknown): Promise<T> {
    return this.req<T>(path, { method: 'POST', body: JSON.stringify(body) });
  }

  private async req<T>(
    path: string,
    init: RequestInit & { auth?: boolean; headers?: Record<string, string> } = {},
  ): Promise<T> {
    const { auth, ...rest } = init;
    const res = await this.send(path, rest, { auth: auth !== false });
    return (await readJson(res)) as T;
  }

  /** Perform a request and turn every failure mode into an ApiError. */
  private async send(
    path: string,
    init: RequestInit & { headers?: Record<string, string> },
    opts: { auth?: boolean; accept?: string } = {},
  ): Promise<Response> {
    const cfg = await this.config.ensure();
    const headers = new Headers(init.headers);
    headers.set('Accept', opts.accept ?? 'application/json');
    if (!cfg.desktop) headers.set('ngrok-skip-browser-warning', '1');
    if (opts.auth !== false && cfg.token) headers.set('Authorization', `Bearer ${cfg.token}`);
    if (typeof init.body === 'string') headers.set('Content-Type', 'application/json');

    const url = `${cfg.baseUrl}/api${path}`;
    const mock = activeMock();
    let res: Response;
    try {
      res = mock
        ? await mock.fetch(url, { ...init, headers })
        : await fetch(url, { ...init, headers, cache: 'no-store' });
    } catch (e) {
      throw new ApiError(
        navigator.onLine === false
          ? 'You are offline.'
          : `Can't reach Mother Claude (${e instanceof Error ? e.message : 'network error'}).`,
        0,
        'network',
      );
    }

    const type = res.headers.get('content-type') ?? '';
    if (type.includes('text/html')) {
      // A JSON API never answers with a page: a tunnel (e.g. ngrok's free
      // plan) put its browser warning in front of us.
      this.interstitial.set(true);
      throw new ApiError(
        'The tunnel is showing its browser warning page. Open this page once in the browser to accept it, then reload.',
        res.status,
        'interstitial',
      );
    }
    this.interstitial.set(false);

    if (res.status === 401) {
      if (opts.auth !== false) this.auth.handleUnauthorized();
      const env = parseEnvelope(await res.text().catch(() => ''));
      throw new ApiError(env.message || 'Not paired', 401, 'unauthorized', env.code);
    }
    if (!res.ok) {
      const env = parseEnvelope(await res.text().catch(() => ''));
      const retry = Number(res.headers.get('retry-after'));
      throw new ApiError(
        env.message || `${res.status} ${res.statusText}`.trim(),
        res.status,
        'http',
        env.code,
        Number.isFinite(retry) && retry > 0 ? retry : undefined,
        env.field,
      );
    }
    return res;
  }
}

export type ApiErrorKind = 'http' | 'network' | 'unauthorized' | 'interstitial' | 'parse';

export class ApiError extends Error {
  constructor(
    message: string,
    /** HTTP status; 0 for network failures. */
    readonly status: number,
    readonly kind: ApiErrorKind = 'http',
    /** Machine code from the error envelope, e.g. `already_decided`. */
    readonly code?: string,
    /** Seconds, from a 429's Retry-After. */
    readonly retryAfter?: number,
    /** Offending request field, when the server names one. */
    readonly field?: string,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

/** A short, human message for any thrown value. */
export function errorMessage(e: unknown): string {
  if (e instanceof ApiError) return e.field ? `${e.message} (${e.field})` : e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}

function enc(v: string): string {
  return encodeURIComponent(v);
}

/** `?a=1&b=2` from the defined, non-empty values. */
function query(params: Record<string, string | number | boolean | undefined | null>): string {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== null && v !== '') q.set(k, String(v));
  }
  const s = q.toString();
  return s ? `?${s}` : '';
}

async function readJson(res: Response): Promise<unknown> {
  if (res.status === 204) return null;
  const text = await res.text();
  if (!text.trim()) return null;
  try {
    return JSON.parse(text);
  } catch {
    throw new ApiError('The server sent a response this app could not read.', res.status, 'parse');
  }
}

/** `{"error":{"code","message","field"}}`, `{"error":"…"}`, `{"message":"…"}` or text. */
function parseEnvelope(text: string): { code?: string; message?: string; field?: string } {
  const trimmed = text.trim();
  if (!trimmed) return {};
  try {
    const body = JSON.parse(trimmed) as {
      error?: string | { code?: string; message?: string; field?: string };
      message?: string;
      code?: string;
    };
    if (typeof body.error === 'string') return { message: body.error, code: body.code };
    if (body.error && typeof body.error === 'object') {
      return { code: body.error.code, message: body.error.message, field: body.error.field };
    }
    if (typeof body.message === 'string') return { message: body.message, code: body.code };
  } catch {
    /* plain text */
  }
  return { message: trimmed.slice(0, 300) };
}
