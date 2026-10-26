import { Injectable, inject, signal } from '@angular/core';

import type { NewServer, ServerDefaults, ServerPatch } from './api-servers';
import { AuthService } from './auth.service';
import { ConfigService } from './config.service';
import { activeMock } from './mock/mock-api';
import type { DeleteBody } from './delete-data';
import type { MoveBody } from './move-data';
import {
  AccountDefaults,
  AccountPatch,
  AccountView,
  AccountsView,
  BackupIndex,
  BackupSettings,
  BackupView,
  BridgeDefaults,
  BridgeStatus,
  CaptureView,
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
  JobView,
  KeyName,
  LaunchDefaults,
  LifecycleAction,
  McpServerStatus,
  MeInfo,
  MessageBody,
  MessageResult,
  NewAccount,
  PairCode,
  PairCodeRequest,
  PairExchangeBody,
  PairExchangeResult,
  PairPendingStatus,
  PassphraseChange,
  PermissionMode,
  PlanUsage,
  PlanUsageReply,
  ProjectInfo,
  PullSkip,
  PushPrefs,
  PushPrefsUpdate,
  PushSubscriptionBody,
  RemoteInfo,
  RemoteUpdate,
  RespondBody,
  ServicesInfo,
  SessionAgents,
  SessionDetail,
  SessionPushPref,
  SessionsPage,
  SessionsQuery,
  SpawnBody,
  TerminalText,
  TokenStatus,
  TranscriptPage,
  TranscriptQuery,
  UploadResult,
  VapidInfo,
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
 *   Data      listSessions(q) · pollSessions(etag) · getSession(id) ·
 *             getTranscript(id, q) · fetchMedia(ref) · getSessionAgents(id) ·
 *             getFleet() · getTaskOutput(id, taskId) · getDiff(id, base?) ·
 *             getFilePatch(id, path, base?) · getFiles(id, q) ·
 *             getCommands(id) · getTerminal(id) · getMcp(id) · getProjects() ·
 *             getDefaults() · getInbox() · getServices()
 *   Control   spawnSession(body) · sendMessage(id, body) · uploadFile(id, f) ·
 *             cancelQueued(id, messageId) · interrupt(id) · setModel(id, m) ·
 *             setMode(id, mode) · setEffort(id, e) · setUltracode(id, on) ·
 *             setThinking(id, on) · rename(id, name) ·
 *             stopTask(id, taskId) · respond(id, body) ·
 *             continueSession(id, body) · sendKey(id, key) ·
 *             lifecycle(id, action)
 *   Usage     getPlanUsage() · refreshPlanUsage(account?) ·
 *             setPlanUsageSettings(intervalSecs)
 *   Accounts  getAccounts() · addAccount(body) · updateAccount(id, patch) ·
 *             removeAccount(id) · setAccountDefaults(d) ·
 *             setCopyConsent(from, to, granted) · recheckAccount(id) ·
 *             loginAccount(id, consoleAccount?) · getAccountToken(id) ·
 *             startTokenCapture(id) · sendTokenCode(id, code) ·
 *             pasteAccountToken(id, token) · cancelTokenCapture(id) ·
 *             installAccountHooks(id) · uninstallAccountHooks(id)
 *   Data      planMove(body) · startMove(body) · listDataJobs() ·
 *             getDataJob(id) · undoDataJob(id) · getDataProjects() ·
 *             planDelete(body) · startDelete(body) · revealSession(id) ·
 *             revealProject(account, cwd)
 *   Backup    getBackup() · setBackupSettings(patch) ·
 *             generateBackupPassphrase() · setBackupPassphrase(p) ·
 *             pushBackup(account?) · getBackupIndex(account, into?) ·
 *             restoreBackup(body) · restoreBackupSettings(account) ·
 *             pullBackup(body) · addStorage(body) ·
 *             renameStorage(id, label) · removeStorage(id) ·
 *             getStorageIndex(storage, set)
 *   Pairing   createPairCode(req) · exchangePairCode(body) ·
 *             pollPairPending(pendingId, secret) · getMe() · unpairSelf()
 *   Devices   listDevices() · approvePair(id, fingerprint) · denyPair(id) ·
 *             revokeDevice(id) · updateDevice(id, patch)
 *   Remote    getRemote() · updateRemote(patch)
 *   Foreign   getForeignControl() · installForeignControl() ·
 *             uninstallForeignControl()
 *   Push      getVapidKey() · pushSubscribe(sub) · pushUnsubscribe(endpoint) ·
 *             pushTest() · getPushPrefs() · setPushPrefs(prefs) ·
 *             setSessionPush(id, pref)
 *
 * `describeError(e)` turns any failure into one user-facing sentence (plus
 * a retry hint), so every page reports errors the same way.
 *   Bridge    getBridge() · startBridge(defaults) · stopBridge() ·
 *             setBridgeThread(threadId)
 *   Servers   addBridgeServer(body) · updateBridgeServer(id, patch) ·
 *             removeBridgeServer(id) · startBridgeServer(id, defaults) ·
 *             stopBridgeServer(id) · setBridgeServerThread(id, threadId)
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

  /** Polling fallback: the active list with a conditional GET. Resolves to
   * null when the server answered 304 (nothing changed). */
  async pollSessions(etag?: string): Promise<{ page: SessionsPage; etag?: string } | null> {
    const res = await this.send(
      '/sessions?scope=active',
      { method: 'GET', headers: etag ? { 'If-None-Match': etag } : {} },
      { allowNotModified: true },
    );
    if (res.status === 304) return null;
    const page = ((await readJson(res)) as SessionsPage | null) ?? { sessions: [] };
    return { page, etag: res.headers.get('etag') ?? undefined };
  }

  getSession(id: string): Promise<SessionDetail> {
    return this.req<SessionDetail>(`/sessions/${enc(id)}`);
  }

  /** Newest-last page of normalised events; page back with `before`. */
  getTranscript(id: string, q: TranscriptQuery = {}): Promise<TranscriptPage> {
    return this.req<TranscriptPage>(`/sessions/${enc(id)}/transcript${query({ ...q })}`);
  }

  /** Fetch an image referenced by a UiEvent (`image.ref`), with auth. Use
   * `URL.createObjectURL` on the result; revoke it when done. `signal`
   * cancels the download (e.g. the row went away). */
  async fetchMedia(ref: string, signal?: AbortSignal): Promise<Blob> {
    const path = ref.startsWith('/api/') ? ref.slice(4) : ref;
    const res = await this.send(path, { method: 'GET', signal }, { accept: '*/*' });
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
  async getTerminal(id: string): Promise<TerminalText> {
    const res = await this.send(`/sessions/${enc(id)}/terminal`, { method: 'GET' });
    const text = await res.text();
    if ((res.headers.get('content-type') ?? '').includes('json')) {
      try {
        const body = JSON.parse(text) as Partial<TerminalText>;
        return { text: typeof body.text === 'string' ? body.text : '', rows: body.rows };
      } catch {
        /* plain text after all */
      }
    }
    return { text };
  }

  /** MCP servers of an owned session. */
  getMcp(id: string): Promise<{ servers: McpServerStatus[] }> {
    return this.req(`/sessions/${enc(id)}/mcp`);
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

  /** Store a file next to the session (`<app data>/uploads/<session>/`) so
   * it can be referenced as `@<path>` — attachments for sessions that only
   * take text (relay, tmux) and for non-image files. */
  uploadFile(
    id: string,
    file: { name: string; mediaType: string; data: string },
  ): Promise<UploadResult> {
    return this.post(`/sessions/${enc(id)}/uploads`, file);
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

  /** Owned sessions: ultracode on or off for the rest of the session (409
   * `not_supported` elsewhere; 422 with the CLI's refusal). */
  setUltracode(id: string, enabled: boolean): Promise<void> {
    return this.post(`/sessions/${enc(id)}/ultracode`, { enabled });
  }

  /** Owned sessions: thinking on or off (409 `not_supported` elsewhere). */
  setThinking(id: string, enabled: boolean): Promise<void> {
    return this.post(`/sessions/${enc(id)}/thinking`, { enabled });
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

  // ───────────────────────────────────────────────────────── Plan usage

  /** The server's last known plan usage (never triggers a read): the
   * Default account's, plus every other account's under `accounts`. */
  getPlanUsage(): Promise<PlanUsageReply> {
    return this.req<PlanUsageReply>('/usage');
  }

  /** Ask for a fresh reading now (the Default account's unless `account`
   * names another). 202 with the current state (`refreshing`); the result
   * arrives as a `planUsage` (or `accountUsage`) frame. Within a minute of
   * the last read: 429 with `retryAfter`. */
  refreshPlanUsage(account?: string): Promise<PlanUsage | null> {
    return this.post('/usage/refresh', account && account !== 'default' ? { account } : {});
  }

  /** Desktop only: seconds between reads, 0 (manual only), 180, 300 or 600. */
  setPlanUsageSettings(intervalSecs: number): Promise<PlanUsage | null> {
    return this.req('/usage/settings', { method: 'PUT', body: JSON.stringify({ intervalSecs }) });
  }

  // ───────────────────────────────────────────────────────────── Accounts

  /** Every Claude account, the defaults, copy consent and where secrets live
   * (any signed-in client; the desktop also gets a running sign-in's URL). */
  getAccounts(): Promise<AccountsView> {
    return this.req<AccountsView>('/accounts');
  }

  /** Desktop only: add an account (201). A pasted token travels once, in
   * this body, and goes straight to the Mac's secret store. */
  addAccount(body: NewAccount): Promise<AccountView> {
    return this.post<AccountView>('/accounts', body);
  }

  /** Desktop only: rename or recolour. */
  updateAccount(id: string, patch: AccountPatch): Promise<AccountView> {
    return this.req<AccountView>(`/accounts/${enc(id)}`, {
      method: 'PATCH',
      body: JSON.stringify(patch),
    });
  }

  /** Desktop only: forget an account (never its directory or its Anthropic
   * sign-in). 409 `account_in_use` while sessions Mother Claude started run
   * under it; 422 `default_account_fixed` for the Default. */
  removeAccount(id: string): Promise<void> {
    return this.req(`/accounts/${enc(id)}`, { method: 'DELETE' });
  }

  /** Desktop only: which account new sessions, the API and projects use. */
  setAccountDefaults(defaults: AccountDefaults): Promise<AccountDefaults> {
    return this.req<AccountDefaults>('/accounts/defaults', {
      method: 'PUT',
      body: JSON.stringify(defaults),
    });
  }

  /** Desktop only: remember (or forget) "copy from → to without asking". */
  setCopyConsent(from: string, to: string, granted: boolean): Promise<AccountsView> {
    return this.req<AccountsView>('/accounts/copy-consent', {
      method: 'PUT',
      body: JSON.stringify({ from, to, granted }),
    });
  }

  /** Check an account's sign-in again. */
  recheckAccount(id: string): Promise<AccountView> {
    return this.post<AccountView>(`/accounts/${enc(id)}/refresh`, {});
  }

  /** Desktop only: a browser sign-in for a directory account (or the
   * Default), opened on the Mac (202; `signingIn` until it lands).
   * `consoleAccount`: an Anthropic Console (API billing) sign-in. */
  loginAccount(id: string, consoleAccount = false): Promise<AccountView> {
    return this.post<AccountView>(
      `/accounts/${enc(id)}/login`,
      consoleAccount ? { console: true } : {},
    );
  }

  /** Desktop only: a token account's `setup-token` run (with its URL). */
  getAccountToken(id: string): Promise<TokenStatus> {
    return this.req<TokenStatus>(`/accounts/${enc(id)}/token`);
  }

  /** Desktop only: start `claude setup-token` (the browser opens on the Mac). */
  startTokenCapture(id: string): Promise<{ capture?: CaptureView }> {
    return this.post(`/accounts/${enc(id)}/token`, {});
  }

  /** Desktop only: type the code the browser shows into a running capture. */
  sendTokenCode(id: string, code: string): Promise<{ capture?: CaptureView }> {
    return this.post(`/accounts/${enc(id)}/token`, { code });
  }

  /** Desktop only: store a pasted token (sent once, in the body). */
  pasteAccountToken(id: string, token: string): Promise<AccountView> {
    return this.post<AccountView>(`/accounts/${enc(id)}/token`, { token });
  }

  /** Desktop only: stop a running capture. */
  cancelTokenCapture(id: string): Promise<{ capture?: CaptureView }> {
    return this.req(`/accounts/${enc(id)}/token`, { method: 'DELETE' });
  }

  /** Desktop only: the hooks plugin in a directory account's directory (422
   * `use_default_install` for the Default, `not_a_directory_account` for a
   * token account). */
  installAccountHooks(id: string): Promise<AccountView> {
    return this.post<AccountView>(`/accounts/${enc(id)}/hooks/install`, {});
  }

  uninstallAccountHooks(id: string): Promise<AccountView> {
    return this.post<AccountView>(`/accounts/${enc(id)}/hooks/uninstall`, {});
  }

  // ─────────────────────────────────────────── Moving and copying data

  /** What a move or copy would do, nothing done yet (`{plan}`;
   * docs/ACCOUNTS.md §13.4). A device needs the dangerous right for
   * `trust` and the settings or MCP customizations (403
   * `dangerous_blocked`). */
  planMove(body: MoveBody): Promise<{ plan: unknown }> {
    return this.post('/data/move', { ...body, dryRun: true });
  }

  /** Run it (202): one job per source location (`job` is the first). */
  startMove(body: MoveBody): Promise<{ jobs?: unknown[]; job?: unknown }> {
    return this.post('/data/move', { ...body, dryRun: false });
  }

  /** Running and recent data jobs (moves, copies, backups, restores, pulls). */
  listDataJobs(): Promise<{ jobs: JobView[] }> {
    return this.req('/data/jobs');
  }

  getDataJob(id: string): Promise<JobView> {
    return this.req<JobView>(`/data/jobs/${enc(id)}`);
  }

  /** Desktop only: put things back as they were (409 `undo_conflict` when
   * something it created was changed since). */
  undoDataJob(id: string): Promise<JobView> {
    return this.post<JobView>(`/data/jobs/${enc(id)}/undo`, {});
  }

  // ─────────────────────── Conversations on disk (docs/ACCOUNTS.md §14)

  /** Every project folder that holds conversations, in every account's
   * directory (`{projects}`). */
  getDataProjects(): Promise<unknown> {
    return this.req('/data/projects');
  }

  /** What a delete would take, nothing taken yet (`{plan}`). Deleting is a
   * dangerous action: 403 `dangerous_blocked` without the right. */
  planDelete(body: DeleteBody): Promise<{ plan: unknown }> {
    return this.post('/data/delete', { ...body, dryRun: true });
  }

  /** Run it (202): one job per directory (`job` is the first). */
  startDelete(body: DeleteBody): Promise<{ jobs?: unknown[]; job?: unknown }> {
    return this.post('/data/delete', { ...body, dryRun: false });
  }

  /** Desktop only: show the conversation's transcript in Finder, File
   * Explorer or the file manager (204). */
  revealSession(id: string): Promise<void> {
    return this.post(`/sessions/${enc(id)}/reveal`, {});
  }

  /** Desktop only: open a project's folder in an account's directory. */
  revealProject(account: string, cwd: string): Promise<void> {
    return this.post('/data/projects/reveal', { account, cwd });
  }

  // ─────────────────────────────────────────────────── Backup and sync

  getBackup(): Promise<BackupView> {
    return this.req<BackupView>('/backup');
  }

  /** Desktop only: send only what changes. */
  setBackupSettings(patch: Partial<BackupSettings>): Promise<BackupView> {
    return this.req<BackupView>('/backup/settings', {
      method: 'PUT',
      body: JSON.stringify(patch),
    });
  }

  /** Desktop only: a generated passphrase, answered this once and then only
   * in the Mac's Keychain. The caller shows it and forgets it. Every set in
   * the folder is re-keyed (`rekeyed`; `left`: sets of no account here it
   * could not open). A failure after some set has the new one carries
   * `error.applied: true` and, for a generated one, `error.passphrase`
   * ({@link ApiError.extra}). */
  generateBackupPassphrase(): Promise<PassphraseChange> {
    return this.post('/backup/passphrase', { generate: true });
  }

  /** Desktop only: the user's own passphrase (one line, at least 12
   * characters); answered and refused like {@link generateBackupPassphrase}. */
  setBackupPassphrase(passphrase: string): Promise<PassphraseChange> {
    return this.post('/backup/passphrase', { passphrase });
  }

  /** Desktop only: back up one set, or every set (202 with the job ids; a
   * token account's set is the Default's, and a set being backed up
   * already answers the running job's id). */
  pushBackup(account?: string): Promise<{ jobs: string[] }> {
    return this.post('/backup/push', account ? { account } : {});
  }

  /** A set's sessions compared with an account's directory (`into`,
   * default the set's own account). */
  getBackupIndex(account: string, into?: string): Promise<BackupIndex> {
    return this.req<BackupIndex>(`/backup/index${query({ account, into })}`);
  }

  /** Desktop only: restore sessions of a set (202 with the job, or its id). */
  restoreBackup(body: {
    account: string;
    into?: string;
    ids: string[];
    trust?: boolean;
  }): Promise<{ job: JobView | string }> {
    return this.post('/backup/restore', body);
  }

  /** Desktop only: unpack the newest settings bundle for review. */
  restoreBackupSettings(account: string): Promise<{ folder: string }> {
    return this.post('/backup/restore', { account, config: true });
  }

  /** Desktop only: pull one account's set, or every account's (202). */
  pullBackup(body: {
    account?: string;
    ids?: string[];
    trust?: boolean;
  }): Promise<{ jobs: (JobView | string)[]; skipped?: (PullSkip | string)[] }> {
    return this.post('/backup/pull', body);
  }

  /** Add a folder that holds sets (201: the storage). */
  addStorage(body: { path: string; label?: string }): Promise<unknown> {
    return this.post('/backup/storage', body);
  }

  renameStorage(id: string, label: string): Promise<unknown> {
    return this.req(`/backup/storage/${enc(id)}`, {
      method: 'PATCH',
      body: JSON.stringify({ label }),
    });
  }

  /** Forget a storage: its files stay where they are. */
  removeStorage(id: string): Promise<void> {
    return this.req(`/backup/storage/${enc(id)}`, { method: 'DELETE' });
  }

  /** One set of a storage, with what each entry is (`archived`: in the
   * storage only). */
  getStorageIndex(storage: string, set: string): Promise<BackupIndex> {
    return this.req<BackupIndex>(`/backup/index${query({ storage, set })}`);
  }

  // ────────────────────────────────────────────────────────────── Pairing

  /** Desktop: mint a single-use pairing code + QR for LAN or internet. A
   * paired device may mint a `self` code for its own Home Screen app. */
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

  /** A device unpairs itself (the server forgets its token). */
  unpairSelf(): Promise<void> {
    return this.req('/me', { method: 'DELETE' });
  }

  // ────────────────────────────────────────────────────────────── Devices

  listDevices(): Promise<DevicesList> {
    return this.req<DevicesList>('/devices');
  }

  /** Approve a pending pairing; `fingerprint` is the 4 digits the user
   * typed after reading them on the device. */
  approvePair(pendingId: string, fingerprint: string): Promise<void> {
    return this.post(`/devices/pending/${enc(pendingId)}/approve`, { fingerprint });
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

  getVapidKey(): Promise<VapidInfo> {
    return this.req<VapidInfo>('/push/vapid');
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

  /** Send only what changed: the server merges it. */
  setPushPrefs(prefs: PushPrefsUpdate): Promise<PushPrefs | null> {
    return this.req('/push/prefs', { method: 'PUT', body: JSON.stringify(prefs) });
  }

  /** Follow / mute one session's notifications (`null` = back to default). */
  setSessionPush(id: string, pref: SessionPushPref | null): Promise<PushPrefs | null> {
    return this.setPushPrefs({ sessions: { [id]: pref } });
  }

  /** Arm (or disarm) a one-shot "notify me when this turn finishes". */
  setTurnFinishedPush(id: string, on: boolean): Promise<PushPrefs | null> {
    return this.setPushPrefs({ turnFinished: { [id]: on } });
  }

  // ─────────────────────────────────────── The Claude HTTP bridge (/bridge)

  getBridge(): Promise<BridgeStatus> {
    return this.req<BridgeStatus>('/bridge');
  }

  /** Start the API with these defaults. Returns the new status. */
  /**
   * Start (or restart) the API. `openToTunnels`: true opens the port to
   * callers without a token, tunnels included (desktop app only); false closes
   * that opening; undefined keeps what `MOTHER_CLAUDE_BRIDGE_REQUIRE_TOKEN` says.
   */
  startBridge(defaults: BridgeDefaults, openToTunnels?: boolean): Promise<BridgeStatus> {
    return this.post<BridgeStatus>('/bridge/start', { defaults, openToTunnels });
  }

  stopBridge(): Promise<BridgeStatus> {
    return this.post<BridgeStatus>('/bridge/stop', {});
  }

  /** Change which conversation unaddressed messages join. */
  setBridgeThread(threadId: string | null): Promise<BridgeStatus> {
    return this.post<BridgeStatus>('/bridge/active-thread', { threadId });
  }

  // ─────────── API servers besides the main one (docs/ACCOUNTS.md §13.5)
  // Each answers that server's status (`normalizeServer` reads it); the
  // main server keeps the routes above.

  /** Add a server (not started); `port` unset: the first free from 5613. */
  addBridgeServer(body: NewServer): Promise<unknown> {
    return this.post<unknown>('/bridge/servers', body);
  }

  /** Rename it, move it to another port, or change its defaults (a running
   * server restarts with them). */
  updateBridgeServer(id: string, patch: ServerPatch): Promise<unknown> {
    return this.req<unknown>(`/bridge/servers/${enc(id)}`, {
      method: 'PATCH',
      body: JSON.stringify(patch),
    });
  }

  /** Remove it (the server stops it first). */
  async removeBridgeServer(id: string): Promise<void> {
    await this.req<unknown>(`/bridge/servers/${enc(id)}`, { method: 'DELETE' });
  }

  /** Start (or restart) it with these defaults; `openToTunnels` as for
   * {@link startBridge} (desktop app only to open it). */
  startBridgeServer(
    id: string,
    defaults: ServerDefaults,
    openToTunnels?: boolean,
  ): Promise<unknown> {
    return this.post<unknown>(`/bridge/servers/${enc(id)}/start`, { defaults, openToTunnels });
  }

  stopBridgeServer(id: string): Promise<unknown> {
    return this.post<unknown>(`/bridge/servers/${enc(id)}/stop`, {});
  }

  /** Change which conversation its unaddressed messages join. */
  setBridgeServerThread(id: string, threadId: string | null): Promise<unknown> {
    return this.post<unknown>(`/bridge/servers/${enc(id)}/active-thread`, { threadId });
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
    opts: { auth?: boolean; accept?: string; allowNotModified?: boolean } = {},
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

    if (res.status === 304 && opts.allowNotModified) return res;
    if (res.status === 401) {
      // Only the credential in use now counts: an answer to a request sent
      // with a token that was replaced meanwhile (re-pairing) says nothing.
      if (opts.auth !== false && cfg.token === this.config.token()) {
        this.auth.handleUnauthorized();
      }
      const env = parseEnvelope(await res.text().catch(() => ''));
      throw new ApiError(env.message || 'Not paired', 401, 'unauthorized', env.code);
    }
    if (!res.ok) {
      const env = parseEnvelope(await res.text().catch(() => ''));
      // The header, else the body's copy (a proxy may drop the header).
      const header = Number(res.headers.get('retry-after'));
      const retry = Number.isFinite(header) && header > 0 ? header : env.retryAfter;
      throw new ApiError(
        env.message || `${res.status} ${res.statusText}`.trim(),
        res.status,
        'http',
        env.code,
        retry,
        env.field,
        env.extra,
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
    /** The error object's other fields (e.g. `copy_consent_required`'s
     * `from` and `to`). */
    readonly extra: Readonly<Record<string, unknown>> = {},
  ) {
    super(message);
    this.name = 'ApiError';
  }

  /** A string field of the error object ('' when absent). */
  detail(key: string): string {
    const v = this.extra[key];
    return typeof v === 'string' ? v : '';
  }
}

/** A short, human message for any thrown value. */
export function errorMessage(e: unknown): string {
  if (e instanceof ApiError) return e.field ? `${e.message} (${e.field})` : e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}

/** What the UI should say (and offer) for a failure. */
export interface DescribedError {
  text: string;
  /** Offer a Retry button. */
  retry?: boolean;
  /** Seconds before retrying makes sense (429). */
  retryAfter?: number;
  /** The session can't be reached this way: show its caps.limits. */
  noChannel?: boolean;
  /** Someone else already answered (409 already_decided). */
  alreadyDecided?: boolean;
  /** A dangerous action this client may not take. */
  dangerous?: boolean;
  /** Link to the API page (bridge not started). */
  bridge?: boolean;
}

/**
 * The one error → sentence mapper every page uses (design review,
 * "loading/empty/error states"). CLI refusals (422) are shown verbatim.
 */
export function describeError(e: unknown): DescribedError {
  if (!(e instanceof ApiError)) return { text: errorMessage(e) };
  switch (e.kind) {
    case 'network':
      return {
        text:
          navigator.onLine === false
            ? "You're offline."
            : "Can't reach your Mac (asleep, offline or the tunnel is down).",
        retry: true,
      };
    case 'interstitial':
      return {
        text: "The tunnel's warning page is in the way: open this page once in the browser, accept it, then reload.",
      };
    case 'unauthorized':
      return { text: 'This device is no longer paired.' };
    case 'parse':
      return { text: e.message, retry: true };
    default:
      break;
  }
  const code = e.code ?? '';
  if (e.status === 403 && (code === 'dangerous_blocked' || /dangerous/i.test(e.message))) {
    return {
      text: 'Only allowed from the Mac, or allow dangerous actions for this device in Remote › Devices.',
      dangerous: true,
    };
  }
  if (e.status === 403 && code === 'desktop_only') {
    return { text: 'Only the desktop app can do this.' };
  }
  if (e.status === 403 && code === 'untrusted_cwd') {
    return {
      text:
        e.message ||
        'That folder has never been opened with Claude Code on the Mac. Open it there once, or start the session from the desktop.',
    };
  }
  if (e.status === 400 && code === 'https_required') {
    return { text: e.message || 'Open Mother Claude through the https:// address of your tunnel.' };
  }
  if ((e.status === 409 || e.status === 410) && code === 'already_decided') {
    return { text: e.message || 'Already answered elsewhere.', alreadyDecided: true };
  }
  if (e.status === 409 && code === 'no_channel') {
    return { text: e.message || "This session can't take that from here.", noChannel: true };
  }
  if (e.status === 422) return { text: e.message };
  if (e.status === 429) {
    const secs = e.retryAfter ?? 30;
    return { text: `Too many requests. Try again in ${secs}s.`, retry: true, retryAfter: secs };
  }
  if (e.status === 503 && code === 'bridge_not_started') {
    return { text: 'The API is not started. Start it on the API page.', bridge: true };
  }
  if (e.status === 501) return { text: 'This server does not support that yet.' };
  if (e.status >= 500) return { text: `The server failed: ${e.message}`, retry: true };
  return { text: errorMessage(e) };
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

/** `{"error":{"code","message","field", …}}`, `{"error":"…"}`, `{"message":"…"}` or text. */
function parseEnvelope(text: string): {
  code?: string;
  message?: string;
  field?: string;
  retryAfter?: number;
  extra?: Record<string, unknown>;
} {
  const trimmed = text.trim();
  if (!trimmed) return {};
  try {
    const body = JSON.parse(trimmed) as {
      error?: string | { code?: string; message?: string; field?: string; retryAfter?: unknown };
      message?: string;
      code?: string;
    };
    if (typeof body.error === 'string') return { message: body.error, code: body.code };
    if (body.error && typeof body.error === 'object') {
      const {
        code,
        message,
        field,
        retryAfter: retry,
        ...extra
      } = body.error as Record<string, unknown>;
      return {
        code: typeof code === 'string' ? code : undefined,
        message: typeof message === 'string' ? message : undefined,
        field: typeof field === 'string' ? field : undefined,
        ...(typeof retry === 'number' && Number.isFinite(retry) && retry > 0
          ? { retryAfter: retry }
          : {}),
        extra,
      };
    }
    if (typeof body.message === 'string') return { message: body.message, code: body.code };
  } catch {
    /* plain text */
  }
  return { message: trimmed.slice(0, 300) };
}
