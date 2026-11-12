import { Injectable, inject, signal } from '@angular/core';

import type { NewServer, ServerDefaults, ServerPatch } from './api-servers';
import type {
  CapacityView,
  Classification,
  HostView,
  PolicyView,
  RoutingPolicy,
  RoutingReport,
  SettingsPatch,
} from './capacity';
import type {
  BrowserActivity,
  BrowserOverview,
  BrowserPairCode,
  BrowserSettingsPatch,
  GrantAction,
  SiteMode,
} from './browser';
import { AuthService } from './auth.service';
import { ConfigService } from './config.service';
import { computerNoun, tr } from './i18n/i18n';
import { activeMock } from './mock/mock-api';
import type { DeleteBody } from './delete-data';
import type {
  AccountsTable,
  AskBody,
  FoldersView,
  Grants,
  MachineAsk,
  MachinesSettingsPatch,
  MachinesSettingsView,
  MachinesView,
  NearbyMachine,
  SyncPair,
  SyncView,
  Readiness,
  RemoteCheckView,
  RootsChange,
  RootsView,
  SetupView,
} from './machines';
import type { CopiesView, SessionCopiesView, TakeOverFromCopy } from './copies';
import type { MoveBody } from './move-data';
import type { MachineRun, RunGrantsView, RunsView } from './runs';
import type { HostedRolesView, MachineTask, TaskRemoved, TasksView } from './tasks';
import {
  MovePreflight,
  MoveRecord,
  SessionMoveBody,
  TakeOverReply,
  normalizeMove,
  normalizeMoves,
  normalizePreflight,
} from './moves';
import type {
  ObservedEvent,
  RoleStartResult,
  StartTeamBody,
  TaskDraft,
  TaskPatch,
  TeamTask,
  TeamTemplate,
  TeamView,
  TemplateEntry,
  TemplateReport,
} from './teams';
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
  McpToolsStatus,
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
  SendNowResult,
  ServicesInfo,
  SessionAgents,
  SessionDetail,
  SessionPushPref,
  SessionsPage,
  SessionsQuery,
  SignInBrowser,
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
 * - Another computer's client (`MachineApiService`, `machine-scope.ts`)
 *   sends the same paths through `/api/machines/<id>/proxy` and words that
 *   computer's refusals with its name ({@link MachineApiError}).
 *
 * Methods:
 *   Data      listSessions(q) · pollSessions(etag) · getSession(id) ·
 *             getTranscript(id, q) · fetchMedia(ref) · getSessionAgents(id) ·
 *             getFleet() · getTaskOutput(id, taskId) · getDiff(id, base?) ·
 *             getFilePatch(id, path, base?) · getFiles(id, q) ·
 *             getCommands(id) · getTerminal(id) · getMcp(id) · getProjects() ·
 *             getDefaults() · getInbox() · getServices()
 *   Control   spawnSession(body) · sendMessage(id, body) · uploadFile(id, f) ·
 *             cancelQueued(id, messageId) · sendNow(id, messageId) ·
 *             interrupt(id) · setModel(id, m) ·
 *             setMode(id, mode) · setEffort(id, e) · setUltracode(id, on) ·
 *             setThinking(id, on) · rename(id, name) ·
 *             stopTask(id, taskId) · respond(id, body) ·
 *             continueSession(id, body) · switchSessionAccount(id, body) ·
 *             cancelSessionSwitch(id) · sendKey(id, key) ·
 *             lifecycle(id, action)
 *   Usage     getPlanUsage() · refreshPlanUsage(account?) ·
 *             setPlanUsageSettings(intervalSecs)
 *   Accounts  getAccounts() · addAccount(body) · updateAccount(id, patch) ·
 *             removeAccount(id) · setAccountDefaults(d) ·
 *             setCopyConsent(from, to, granted) · recheckAccount(id) ·
 *             loginAccount(id, consoleAccount?) · getAccount(id) ·
 *             getSignInBrowsers(id) · openSignIn(id, browser) ·
 *             sendSignInCode(id, code) · cancelSignIn(id) ·
 *             acceptIdentity(id) · getAccountToken(id) ·
 *             startTokenCapture(id) · sendTokenCode(id, code) ·
 *             pasteAccountToken(id, token) · cancelTokenCapture(id) ·
 *             installAccountHooks(id) · uninstallAccountHooks(id)
 *   Data      planMove(body) · startMove(body) · listDataJobs() ·
 *             getDataJob(id) · undoDataJob(id) · getDataProjects() ·
 *             planDelete(body) · startDelete(body) · revealSession(id) ·
 *             revealProject(account, cwd)
 *   Backup    getBackup() · setBackupSettings(patch) ·
 *             generateBackupPassphrase() · setBackupPassphrase(p) ·
 *             pushBackup(account?, force?) · getBackupIndex(account, into?) ·
 *             restoreBackup(body) · restoreBackupSettings(account) ·
 *             pullBackup(body) · addStorage(body) ·
 *             renameStorage(id, label) · removeStorage(id) ·
 *             getStorageIndex(storage, set) · getFolderPairs(refresh?) ·
 *             addFolderPair(body) · removeFolderPair(id)
 *   Pairing   createPairCode(req) · exchangePairCode(body) ·
 *             pollPairPending(pendingId, secret) · getMe() · unpairSelf()
 *   Devices   listDevices() · approvePair(id, fingerprint) · denyPair(id) ·
 *             revokeDevice(id) · updateDevice(id, patch)
 *   Remote    getRemote() · updateRemote(patch)
 *   Foreign   getForeignControl() · installForeignControl() ·
 *             uninstallForeignControl()
 *   Tools     getMcpTools(withPlugin?) · setMcpToolsEnabled(enabled)
 *   Push      getVapidKey() · pushSubscribe(sub) · pushUnsubscribe(endpoint) ·
 *             pushTest() · getPushPrefs() · setPushPrefs(prefs) ·
 *             setSessionPush(id, pref)
 *   Browser   getBrowser() · getBrowserActivity(session?) ·
 *             createBrowserPairCode() · revokeBrowser(id) ·
 *             setBrowserSite(origin, mode) · setBrowserSettings(patch) ·
 *             addBrowserGrant(action, origin) · removeBrowserGrant(id) ·
 *             stopBrowserSession(id) · resumeBrowserSession(id)
 *
 * `describeError(e)` turns any failure into one user-facing sentence (plus
 * a retry hint), so every page reports errors the same way.
 *   Bridge    getBridge() · startBridge(defaults) · stopBridge() ·
 *             setBridgeThread(threadId)
 *   Servers   addBridgeServer(body) · updateBridgeServer(id, patch) ·
 *             removeBridgeServer(id) · startBridgeServer(id, defaults) ·
 *             stopBridgeServer(id) · setBridgeServerThread(id, threadId)
 *   Machines  getMachines() · renameThisMachine(name) · startPairing() ·
 *             cancelPairing() · joinMachine(code, name?) ·
 *             confirmPairing(grants, digits?) · nearbyMachines() ·
 *             getMachinesSetup(fresh?) · recheckMachines() ·
 *             fixMachineCheck(check) · askMachineFix(machine, check) ·
 *             saveSetupProgress(step) · testMachines(machine?) ·
 *             getMachinesSettings() · saveMachinesSettings(patch) ·
 *             answerSettingsOffer(accept) · setMachinesEnabled(enabled) ·
 *             updateMachine(id, patch) · revokeMachine(id) ·
 *             askMachine(machine, body) · answerMachineAsk(id, answer) ·
 *             getSync() · setSync(project, machine, on) · syncNow(id) ·
 *             resolveSync(id, keep) · getRuns() · getRun(id) ·
 *             cancelRun(id) · getRunGrants() · revokeRunGrant(id) ·
 *             getTasks() · getTask(id) · cancelTask(id) · removeTask(id) ·
 *             getHostedRoles() · stopHostedRole(binding) ·
 *             removeHostedRole(binding) ·
 *             getMachineAccounts() · getMachineFolders(fresh?) ·
 *             cloneHere(body) · checkProjectRemote(project) ·
 *             getRoots() · saveRoots(change)
 *   Moves     moveSessionPreflight(id, body) · moveSession(id, body) ·
 *             takeOverSession(id, self, opts?) · listMoves() · getMove(id) ·
 *             cancelMove(id) · undoMove(id) · retryMove(id)
 *   Copies    getSessionCopies(id) · setSessionCopies(id, machines) ·
 *             getCopies() · removeCopy(id)
 */
@Injectable({ providedIn: 'root' })
export class ApiService {
  private config = inject(ConfigService);
  private auth = inject(AuthService);

  /** True once an /api call came back as an HTML page (tunnel interstitial). */
  readonly interstitial = signal(false);

  /** Put between `/api` and every path: '' here; another computer's client
   * (`machine-scope.ts`) sends through its proxy, `/machines/<id>/proxy`. */
  protected readonly scope: string = '';

  // ───────────────────────────────────────────────────────────────── Data

  listSessions(q: SessionsQuery = {}): Promise<SessionsPage> {
    return this.req<SessionsPage>(`/sessions${query({ ...q })}`);
  }

  /** Every session the filters match, by id, unpaged (*Select all*). */
  async listSessionIds(q: Omit<SessionsQuery, 'limit' | 'before'> = {}): Promise<string[]> {
    const v = await this.req<{ ids?: unknown }>(`/sessions${query({ ...q, view: 'ids' })}`);
    return Array.isArray(v?.ids) ? v.ids.filter((i): i is string => typeof i === 'string') : [];
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

  /** Send now on a message waiting behind the running turn: the turn makes
   * way for it at once (sessions Mother Claude runs). */
  sendNow(id: string, messageId: string): Promise<SendNowResult> {
    return this.post(`/sessions/${enc(id)}/queue/${enc(messageId)}/send-now`, {});
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

  /** Switch a session's account (docs/ACCOUNTS.md §15.1): `200 {id,
   * account, moved}`, or `202 {pending: true}` while its turn runs. */
  switchSessionAccount(
    id: string,
    body: { account: string; when?: 'idle' | 'now' },
  ): Promise<unknown> {
    return this.post(`/sessions/${enc(id)}/account`, body);
  }

  /** Cancel a switch waiting for the end of a turn. */
  cancelSessionSwitch(id: string): Promise<void> {
    return this.req(`/sessions/${enc(id)}/account`, { method: 'DELETE' });
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

  // ─────────────────────────── Capacity, routing, budgets (docs/ROUTING.md)

  /** Memory, load and each live session's memory, with the estimate. */
  getHost(): Promise<HostView> {
    return this.req<HostView>('/host');
  }

  /** Caps and signals, who runs and who waits (and why), budgets, recent
   * routing decisions and the settings. */
  getCapacity(): Promise<CapacityView> {
    return this.req<CapacityView>('/capacity');
  }

  /** The desktop or a paired device: change some settings (`null` turns an
   * optional cap off; maps are replaced whole). 422 names the `field`. */
  setCapacitySettings(patch: SettingsPatch): Promise<CapacityView> {
    return this.req<CapacityView>('/capacity/settings', {
      method: 'PUT',
      body: JSON.stringify(patch),
    });
  }

  getRoutingPolicy(): Promise<PolicyView> {
    return this.req<PolicyView>('/routing/policy');
  }

  /** Desktop only: replace the routing policy (validated; 422 names the field). */
  setRoutingPolicy(policy: RoutingPolicy): Promise<PolicyView> {
    return this.req<PolicyView>('/routing/policy', {
      method: 'PUT',
      body: JSON.stringify(policy),
    });
  }

  /** Desktop only: back to the built-in policy. */
  resetRoutingPolicy(): Promise<PolicyView> {
    return this.req<PolicyView>('/routing/policy', { method: 'DELETE' });
  }

  /** Cost and outcomes of tasks over the last `days` (estimates). */
  getRoutingReport(days: number, project?: string): Promise<RoutingReport> {
    return this.req<RoutingReport>(`/routing/report${query({ days, project })}`);
  }

  /** The report as CSV text: by class × model × effort, or every task end. */
  async getRoutingReportCsv(
    days: number,
    rows: 'groups' | 'outcomes' = 'groups',
    project?: string,
  ): Promise<string> {
    const res = await this.send(
      `/routing/report.csv${query({ days, rows, project })}`,
      {},
      {
        accept: 'text/csv',
      },
    );
    return res.text();
  }

  /** The class a task would get (keyword rules; `haiku` asks Claude Haiku
   * when it is on, spending tokens). */
  classifyTask(title: string, goal: string, haiku = false): Promise<Classification> {
    return this.post<Classification>('/routing/classify', { title, goal, haiku });
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

  /** One account, with its sign-in's link in the desktop's own read. */
  getAccount(id: string): Promise<AccountView> {
    return this.req<AccountView>(`/accounts/${enc(id)}`);
  }

  /** Desktop only: the browsers a sign-in link may open in (§15.2). */
  getSignInBrowsers(id: string): Promise<unknown> {
    return this.req(`/accounts/${enc(id)}/sign-in/open`);
  }

  /** Desktop only: open the running sign-in's link in a browser (answers
   * `{signIn}`). */
  openSignIn(id: string, browser: SignInBrowser): Promise<unknown> {
    return this.post(`/accounts/${enc(id)}/sign-in/open`, { browser });
  }

  /** Desktop only: type the code the browser shows into the sign-in. */
  sendSignInCode(id: string, code: string): Promise<unknown> {
    return this.post(`/accounts/${enc(id)}/sign-in/code`, { code });
  }

  /** Desktop only: stop the account's sign-in (answers `{signIn}`). */
  cancelSignIn(id: string): Promise<unknown> {
    return this.req(`/accounts/${enc(id)}/sign-in`, { method: 'DELETE' });
  }

  /** Keep who the account signed in as (its identity warning goes). */
  acceptIdentity(id: string): Promise<AccountView> {
    return this.post<AccountView>(`/accounts/${enc(id)}/identity/accept`, {});
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

  /** Back up one set, or every set (202 with the job ids; a token account's
   * set is the Default's, and a set being backed up already answers the
   * running job's id). `force`: a force backup — this Mac's copy replaces
   * the backup's wherever they differ (a device needs the dangerous right). */
  pushBackup(account?: string, force = false): Promise<{ jobs: string[] }> {
    return this.post('/backup/push', {
      ...(account ? { account } : {}),
      ...(force ? { force: true } : {}),
    });
  }

  /** A set's sessions compared with an account's directory (`into`,
   * default the set's own account). */
  getBackupIndex(account: string, into?: string): Promise<BackupIndex> {
    return this.req<BackupIndex>(`/backup/index${query({ account, into })}`);
  }

  /** Restore sessions of a set (202 with the job, or its id). `force`: one
   * the account has is replaced by the backup's copy (kept for undo). */
  restoreBackup(body: {
    account: string;
    into?: string;
    ids: string[];
    trust?: boolean;
    force?: boolean;
  }): Promise<{ job: JobView | string }> {
    return this.post('/backup/restore', body);
  }

  /** Desktop only: unpack the newest settings bundle for review. */
  restoreBackupSettings(account: string): Promise<{ folder: string }> {
    return this.post('/backup/restore', { account, config: true });
  }

  /** Pull one account's set, or every account's (202). `force`: the
   * backup's copy replaces this Mac's wherever they differ (kept for undo). */
  pullBackup(body: {
    account?: string;
    ids?: string[];
    trust?: boolean;
    force?: boolean;
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

  /** The folder pairs as the backups last said (`refresh`: read them again,
   * which opens every set). */
  getFolderPairs(refresh = false): Promise<unknown> {
    return this.req(`/backup/pairs${refresh ? '?refresh=true' : ''}`);
  }

  /** Pair this machine's folder with another machine's (201: the pairs). */
  addFolderPair(body: { folder: string; machine: string; otherFolder: string }): Promise<unknown> {
    return this.post('/backup/pairs', body);
  }

  /** Remove a folder pair from every backup (the pairs left). */
  removeFolderPair(id: string): Promise<unknown> {
    return this.req(`/backup/pairs/${enc(id)}`, { method: 'DELETE' });
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

  // ─────────────────────────────────────────── Claude Code tools (MCP)

  /** Mother Claude's MCP server: on/off, its tools, the last calls; on the
   * desktop also the installed plugin, unless `withPlugin` is false (a poll:
   * the plugin's state may ask the CLI). */
  getMcpTools(withPlugin = true): Promise<McpToolsStatus> {
    return this.req<McpToolsStatus>(withPlugin ? '/mcp' : '/mcp?plugin=false');
  }

  /** Turn the tools on or off for every session (desktop only). */
  setMcpToolsEnabled(enabled: boolean): Promise<McpToolsStatus> {
    return this.req<McpToolsStatus>('/mcp/settings', {
      method: 'PUT',
      body: JSON.stringify({ enabled }),
    });
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

  // ─────────────────────────── The browser extension (docs/BROWSER.md)

  /** Paired browsers, site modes, settings, grants, leases, approvals. */
  getBrowser(): Promise<BrowserOverview> {
    return this.req<BrowserOverview>('/browser');
  }

  /** Recent browser actions (no values), of one session or all. */
  getBrowserActivity(session?: string, limit = 100): Promise<{ entries: BrowserActivity[] }> {
    const q = new URLSearchParams({ limit: String(limit) });
    if (session) q.set('session', session);
    return this.req<{ entries: BrowserActivity[] }>(`/browser/activity?${q}`);
  }

  /** A pairing code for the extension (the desktop, or a granted device). */
  createBrowserPairCode(): Promise<BrowserPairCode> {
    return this.post<BrowserPairCode>('/browser/pair', {});
  }

  async revokeBrowser(id: string): Promise<void> {
    await this.req<unknown>(`/browser/browsers/${enc(id)}`, { method: 'DELETE' });
  }

  /** Set a site's mode; `null` removes its own (it follows the default). */
  setBrowserSite(origin: string, mode: SiteMode | null): Promise<BrowserOverview> {
    return this.req<BrowserOverview>('/browser/sites', {
      method: 'PUT',
      body: JSON.stringify({ origin, mode }),
    });
  }

  setBrowserSettings(patch: BrowserSettingsPatch): Promise<BrowserOverview> {
    return this.req<BrowserOverview>('/browser/settings', {
      method: 'PUT',
      body: JSON.stringify(patch),
    });
  }

  addBrowserGrant(action: GrantAction, origin: string): Promise<{ id: string }> {
    return this.post<{ id: string }>('/browser/grants', { action, origin });
  }

  async removeBrowserGrant(id: string): Promise<void> {
    await this.req<unknown>(`/browser/grants/${enc(id)}`, { method: 'DELETE' });
  }

  /** Stop a session's browser use now (refused for a minute). */
  stopBrowserSession(id: string): Promise<unknown> {
    return this.post<unknown>(`/browser/sessions/${enc(id)}/stop`, {});
  }

  async resumeBrowserSession(id: string): Promise<void> {
    await this.req<unknown>(`/browser/sessions/${enc(id)}/resume`, { method: 'POST' });
  }

  // ─────────── Teams of sessions (docs/TEAMS.md): templates, teams, roles,
  // the board. Live changes come as `teams` frames (RealtimeService.teams$).

  getTeamTemplates(): Promise<{ templates: TemplateEntry[] }> {
    return this.req<{ templates: TemplateEntry[] }>('/team-templates');
  }

  /** What saving a template would say, without saving it. */
  validateTeamTemplate(template: unknown): Promise<{ ok: boolean } & TemplateReport> {
    return this.post<{ ok: boolean } & TemplateReport>('/team-templates/validate', template);
  }

  createTeamTemplate(template: unknown): Promise<TeamTemplate> {
    return this.post<TeamTemplate>('/team-templates', template);
  }

  saveTeamTemplate(id: string, template: unknown): Promise<TeamTemplate> {
    return this.req<TeamTemplate>(`/team-templates/${enc(id)}`, {
      method: 'PUT',
      body: JSON.stringify(template),
    });
  }

  async deleteTeamTemplate(id: string): Promise<void> {
    await this.req<unknown>(`/team-templates/${enc(id)}`, { method: 'DELETE' });
  }

  duplicateTeamTemplate(id: string, name?: string): Promise<TeamTemplate> {
    return this.post<TeamTemplate>(`/team-templates/${enc(id)}/duplicate`, { name });
  }

  getTeams(): Promise<{ teams: TeamView[] }> {
    return this.req<{ teams: TeamView[] }>('/teams');
  }

  getTeam(id: string): Promise<TeamView> {
    return this.req<TeamView>(`/teams/${enc(id)}`);
  }

  startTeam(body: StartTeamBody): Promise<TeamView> {
    return this.post<TeamView>('/teams', body);
  }

  stopTeam(id: string): Promise<TeamView> {
    return this.post<TeamView>(`/teams/${enc(id)}/stop`, {});
  }

  resumeTeam(id: string): Promise<TeamView> {
    return this.post<TeamView>(`/teams/${enc(id)}/resume`, {});
  }

  async removeTeam(id: string): Promise<void> {
    await this.req<unknown>(`/teams/${enc(id)}`, { method: 'DELETE' });
  }

  startTeamRole(id: string, role: string): Promise<RoleStartResult> {
    return this.post<RoleStartResult>(`/teams/${enc(id)}/roles/${enc(role)}/start`, {});
  }

  stopTeamRole(id: string, role: string): Promise<TeamView> {
    return this.post<TeamView>(`/teams/${enc(id)}/roles/${enc(role)}/stop`, {});
  }

  /** A foreign live session takes the role. */
  takeTeamRole(id: string, role: string, sessionId: string): Promise<TeamView> {
    return this.post<TeamView>(`/teams/${enc(id)}/roles/${enc(role)}/take`, { sessionId });
  }

  releaseTeamRole(id: string, role: string): Promise<TeamView> {
    return this.req<TeamView>(`/teams/${enc(id)}/roles/${enc(role)}/take`, { method: 'DELETE' });
  }

  getTeamTasks(id: string): Promise<{ tasks: TeamTask[] }> {
    return this.req<{ tasks: TeamTask[] }>(`/teams/${enc(id)}/tasks`);
  }

  createTeamTask(id: string, draft: TaskDraft): Promise<TeamTask> {
    return this.post<TeamTask>(`/teams/${enc(id)}/tasks`, draft);
  }

  updateTeamTask(id: string, task: number, patch: TaskPatch): Promise<TeamTask> {
    return this.req<TeamTask>(`/teams/${enc(id)}/tasks/${task}`, {
      method: 'PATCH',
      body: JSON.stringify(patch),
    });
  }

  /** Agent-team events interactive sessions reported (observe only). */
  getTeamEvents(): Promise<{ events: ObservedEvent[] }> {
    return this.req<{ events: ObservedEvent[] }>('/teams/events');
  }

  // ─── Machines: your other computers (docs/PLAN-MACHINES.md) ─────────────

  getMachines(): Promise<MachinesView> {
    return this.req<MachinesView>('/machines');
  }

  /** Desktop: this computer's name as the others see it. */
  renameThisMachine(name: string): Promise<MachinesView> {
    return this.req<MachinesView>('/machines/self', {
      method: 'PATCH',
      body: JSON.stringify({ name }),
    });
  }

  /** Desktop: a new pairing code (replacing any pairing in progress). */
  startPairing(): Promise<MachinesView> {
    return this.post<MachinesView>('/machines/pair', {});
  }

  /** Desktop: cancel the pairing in progress (the other side is told). */
  cancelPairing(): Promise<MachinesView> {
    return this.req<MachinesView>('/machines/pair', { method: 'DELETE' });
  }

  /** Desktop: join the computer showing `code`; `name` names this one. */
  joinMachine(code: string, name?: string): Promise<MachinesView> {
    return this.post<MachinesView>('/machines/join', name ? { code, name } : { code });
  }

  /** Desktop: the numbers match — with what the other may do here (and the
   * typed digits when it joined over the internet). */
  confirmPairing(grants: Grants, digits?: string): Promise<MachinesView> {
    return this.post<MachinesView>('/machines/confirm', digits ? { grants, digits } : { grants });
  }

  /** Desktop: computers on this network showing a pairing code. */
  nearbyMachines(): Promise<{ machines: NearbyMachine[] }> {
    return this.req<{ machines: NearbyMachine[] }>('/machines/nearby');
  }

  /** Every computer's checks; `fresh` runs them again first. */
  getMachinesSetup(fresh = false): Promise<SetupView> {
    return this.req<SetupView>(`/machines/setup${query({ fresh: fresh || undefined })}`);
  }

  /** Desktop: run every check again, here and on the paired computers. */
  recheckMachines(): Promise<SetupView> {
    return this.post<SetupView>('/machines/setup/check', {});
  }

  /** Desktop: run a check's fix on this computer. */
  fixMachineCheck(check: string): Promise<SetupView> {
    return this.post<SetupView>('/machines/setup/fix', { check });
  }

  /** Desktop: ask another computer to show one of its fixes to whoever is
   * at it. */
  askMachineFix(machine: string, check: string): Promise<{ asked: boolean }> {
    return this.post<{ asked: boolean }>('/machines/setup/ask', { machine, check });
  }

  /** Desktop: the setup step left at (null when it is done). */
  saveSetupProgress(step: number | null): Promise<MachinesView> {
    return this.req<MachinesView>('/machines/setup/progress', {
      method: 'PUT',
      body: JSON.stringify({ step }),
    });
  }

  /** Desktop: the readiness test with one computer, or every one. */
  testMachines(machine?: string): Promise<{ results: Readiness[] }> {
    return this.post<{ results: Readiness[] }>('/machines/test', machine ? { machine } : {});
  }

  /** The options with every project's own. */
  getMachinesSettings(): Promise<MachinesSettingsView> {
    return this.req<MachinesSettingsView>('/machines/settings');
  }

  /** Desktop: the relay, keeping awake, the code options, and projects'
   * own (a file to carry needs the dangerous right). */
  saveMachinesSettings(patch: MachinesSettingsPatch): Promise<MachinesSettingsView> {
    return this.req<MachinesSettingsView>('/machines/settings', {
      method: 'PUT',
      body: JSON.stringify(patch),
    });
  }

  /** Desktop: ask another computer for a fix or an asked action (*Sign
   * in…*, *Add the account*, *Clone it*): shown there, done only when
   * someone there confirms. */
  askMachine(machine: string, body: AskBody): Promise<{ asked: boolean; ask?: MachineAsk | null }> {
    return this.post<{ asked: boolean; ask?: MachineAsk | null }>('/machines/setup/ask', {
      machine,
      ...body,
    });
  }

  /** Desktop: confirm (run) or decline what another computer asked. */
  answerMachineAsk(
    id: string,
    answer: 'confirm' | 'decline',
  ): Promise<{ ask: MachineAsk | null; next: string | null }> {
    return this.post<{ ask: MachineAsk | null; next: string | null }>(`/machines/asks/${enc(id)}`, {
      answer,
    });
  }

  /** Every computer's accounts, matched by name. */
  /** `GET /api/machines/sync`: the projects kept in sync with another
   * computer, and how each stands. */
  getSync(): Promise<SyncView> {
    return this.req<SyncView>('/machines/sync');
  }

  /** `PUT /api/machines/sync`: on asks the other computer (its desktop
   * confirms there); off is at once here, and the other is told. */
  setSync(project: string, machine: string, on: boolean): Promise<SyncView> {
    return this.req<SyncView>('/machines/sync', {
      method: 'PUT',
      body: JSON.stringify({ project, machine, on }),
    });
  }

  /** `POST /api/machines/sync/{id}/now`: a step now. */
  syncNow(id: string): Promise<SyncPair> {
    return this.post<SyncPair>(`/machines/sync/${enc(id)}/now`, {});
  }

  /** `POST /api/machines/sync/{id}/resolve`: the other side's state is kept
   * under a ref and put back, then the kept side's lands. */
  resolveSync(id: string, keep: 'here' | 'there'): Promise<SyncPair> {
    return this.post<SyncPair>(`/machines/sync/${enc(id)}/resolve`, { keep });
  }

  /** `GET /api/sessions/{id}/copies`: the computers it keeps a copy on,
   * and how each copy stands (docs/PLAN-MACHINES.md §5.9). */
  getSessionCopies(id: string): Promise<SessionCopiesView> {
    return this.req<SessionCopiesView>(`/sessions/${enc(id)}/copies`);
  }

  /** `PUT /api/sessions/{id}/copies`: its own choice of computers, or
   * `null` to follow its project's. A computer left out drops its copy. */
  setSessionCopies(id: string, machines: string[] | null): Promise<SessionCopiesView> {
    return this.req<SessionCopiesView>(`/sessions/${enc(id)}/copies`, {
      method: 'PUT',
      body: JSON.stringify({ machines }),
    });
  }

  /** `GET /api/machines/copies`: the copies held here, and this
   * computer's kept on others. */
  getCopies(): Promise<CopiesView> {
    return this.req<CopiesView>('/machines/copies');
  }

  /** `DELETE /api/machines/copies/{id}` (`held.…` here, `kept.…` there):
   * answers the list. */
  removeCopy(id: string): Promise<CopiesView> {
    return this.req<CopiesView>(`/machines/copies/${enc(id)}`, { method: 'DELETE' });
  }

  /** `GET /api/machines/runs`: runs on other computers, both ways. */
  getRuns(): Promise<RunsView> {
    return this.req<RunsView>('/machines/runs');
  }

  /** `GET /api/machines/runs/{id}`: one run whole (its output's last 64 KiB). */
  getRun(id: string): Promise<MachineRun> {
    return this.req<MachineRun>(`/machines/runs/${enc(id)}`);
  }

  /** `POST /api/machines/runs/{id}/cancel`: stops it, there or here. */
  cancelRun(id: string): Promise<MachineRun> {
    return this.post<MachineRun>(`/machines/runs/${enc(id)}/cancel`, {});
  }

  /** `GET /api/machines/run-grants`: the commands a project may run on
   * another computer without asking. */
  getRunGrants(): Promise<RunGrantsView> {
    return this.req<RunGrantsView>('/machines/run-grants');
  }

  /** Desktop: `DELETE /api/machines/run-grants/{id}`; answers the list. */
  revokeRunGrant(id: string): Promise<RunGrantsView> {
    return this.req<RunGrantsView>(`/machines/run-grants/${enc(id)}`, { method: 'DELETE' });
  }

  /** `GET /api/machines/tasks`: tasks handed to other computers, and
   * taken from them. */
  getTasks(): Promise<TasksView> {
    return this.req<TasksView>('/machines/tasks');
  }

  /** `GET /api/machines/tasks/{id}`: one task whole (its text, summary,
   * branch). */
  getTask(id: string): Promise<MachineTask> {
    return this.req<MachineTask>(`/machines/tasks/${enc(id)}`);
  }

  /** `POST /api/machines/tasks/{id}/cancel`: stops its session, there or
   * here. */
  cancelTask(id: string): Promise<MachineTask> {
    return this.post<MachineTask>(`/machines/tasks/${enc(id)}/cancel`, {});
  }

  /** `DELETE /api/machines/tasks/{id}`: removes an ended task (its
   * worktree there; never its branch here). */
  removeTask(id: string): Promise<TaskRemoved> {
    return this.req<TaskRemoved>(`/machines/tasks/${enc(id)}`, { method: 'DELETE' });
  }

  /** `GET /api/machines/roles`: the team roles this computer runs for
   * other computers' leads. */
  getHostedRoles(): Promise<HostedRolesView> {
    return this.req<HostedRolesView>('/machines/roles');
  }

  /** `POST /api/machines/roles/{binding}/stop`: stops one's session here
   * (its lead starts it again when it has work). */
  stopHostedRole(binding: string): Promise<unknown> {
    return this.post<unknown>(`/machines/roles/${enc(binding)}/stop`, {});
  }

  /** `DELETE /api/machines/roles/{binding}`: removes one here (its
   * session, worktree and record). */
  removeHostedRole(binding: string): Promise<{ removed: boolean }> {
    return this.req<{ removed: boolean }>(`/machines/roles/${enc(binding)}`, {
      method: 'DELETE',
    });
  }

  getMachineAccounts(): Promise<AccountsTable> {
    return this.req<AccountsTable>('/machines/accounts');
  }

  /** Desktop: repositories matched by origin with every computer's. */
  getMachineFolders(fresh = false): Promise<FoldersView> {
    return this.req<FoldersView>(`/machines/folders${query({ fresh: fresh || undefined })}`);
  }

  /** Desktop: `git clone` a repository into a folder of a root here. */
  cloneHere(body: {
    origin: string;
    root: string;
    rel: string;
  }): Promise<{ cloned: boolean; folder: string }> {
    return this.post<{ cloned: boolean; folder: string }>('/machines/clone', body);
  }

  /** Desktop: whether a project's remote answers without a prompt, here
   * and where a root maps it. */
  checkProjectRemote(project: string): Promise<RemoteCheckView> {
    return this.post<RemoteCheckView>('/machines/setup/remote', { project });
  }

  /** The roots: one folder per computer. */
  getRoots(): Promise<RootsView> {
    return this.req<RootsView>('/roots');
  }

  /** Add, change and remove roots, all or nothing. */
  saveRoots(change: RootsChange): Promise<RootsView> {
    return this.req<RootsView>('/roots', { method: 'PUT', body: JSON.stringify(change) });
  }

  /** Desktop: take or leave another computer's suggested settings. */
  answerSettingsOffer(accept: boolean): Promise<MachinesView> {
    return this.post<MachinesView>('/machines/settings/offer', { accept });
  }

  /** Desktop: turn Machines on (refused while a required check fails:
   * `409 machines_blocked` with `blockers`) or off. */
  setMachinesEnabled(enabled: boolean): Promise<MachinesView> {
    return this.post<MachinesView>('/machines/enable', { enabled });
  }

  /** Desktop: rename a paired computer here (`''` gives back its own name),
   * or change what it may do here. */
  updateMachine(id: string, patch: { name?: string; grants?: Grants }): Promise<MachinesView> {
    return this.req<MachinesView>(`/machines/${enc(id)}`, {
      method: 'PATCH',
      body: JSON.stringify(patch),
    });
  }

  /** Desktop: forget a paired computer and cut its connections. */
  revokeMachine(id: string): Promise<MachinesView> {
    return this.req<MachinesView>(`/machines/${enc(id)}`, { method: 'DELETE' });
  }

  // ─── Moving a session to another computer (docs/PLAN-MACHINES.md §5.8) ──
  // On the computer the session lives on: another computer's client sends
  // them through its proxy (the `move` grant there).

  /** The move sheet's checks, the other computer asked too; nothing is
   * written anywhere. */
  async moveSessionPreflight(id: string, body: SessionMoveBody): Promise<MovePreflight> {
    const p = normalizePreflight(
      await this.post<unknown>(`/sessions/${enc(id)}/move/preflight`, body),
    );
    if (!p) throw unreadable();
    return p;
  }

  /** Starts a move: its record (`202`), then `move` frames. A blocker found
   * again is `409 move_blocked` with the `preflight`. */
  async moveSession(id: string, body: SessionMoveBody): Promise<MoveRecord> {
    return moveOf(await this.post<unknown>(`/sessions/${enc(id)}/move`, body));
  }

  /** The session's computer is offline: it lives on this one (`self`, this
   * computer's id) from now on, from its copy here. */
  takeOverSession(
    id: string,
    self: string,
    opts: { fromCopy?: boolean; folder?: string } = {},
  ): Promise<TakeOverReply & TakeOverFromCopy> {
    return this.post<TakeOverReply & TakeOverFromCopy>(`/sessions/${enc(id)}/move`, {
      to: self,
      takeOver: true,
      ...opts,
    });
  }

  /** Every move this computer keeps (sent and received), newest first. */
  async listMoves(): Promise<MoveRecord[]> {
    return normalizeMoves(await this.req<unknown>('/moves'));
  }

  async getMove(id: string): Promise<MoveRecord> {
    return moveOf(await this.req<unknown>(`/moves/${enc(id)}`));
  }

  /** Until the other computer starts placing it (`409 move_too_late`). */
  async cancelMove(id: string): Promise<MoveRecord> {
    return moveOf(await this.post<unknown>(`/moves/${enc(id)}/cancel`, {}));
  }

  /** Undone on the computer it went to; it lives where it was again. */
  async undoMove(id: string): Promise<MoveRecord> {
    return moveOf(await this.post<unknown>(`/moves/${enc(id)}/undo`, {}));
  }

  /** A move that landed but did not start there starts again. */
  async retryMove(id: string): Promise<MoveRecord> {
    return moveOf(await this.post<unknown>(`/moves/${enc(id)}/retry`, {}));
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

    const url = `${cfg.baseUrl}/api${this.scope}${path}`;
    const mock = activeMock();
    let res: Response;
    try {
      res = mock
        ? await mock.fetch(url, { ...init, headers })
        : await fetch(url, { ...init, headers, cache: 'no-store' });
    } catch (e) {
      throw new ApiError(
        navigator.onLine === false
          ? tr('You are offline.')
          : tr("Can't reach Mother Claude ({error}).", {
              error: e instanceof Error ? e.message : tr('network error'),
            }),
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
        tr(
          'The tunnel is showing its browser warning page. Open this page once in the browser to accept it, then reload.',
        ),
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
        this.unauthorized();
      }
      const env = parseEnvelope(await res.text().catch(() => ''));
      throw this.failure(
        new ApiError(env.message || tr('Not paired'), 401, 'unauthorized', env.code),
      );
    }
    if (!res.ok) {
      const env = parseEnvelope(await res.text().catch(() => ''));
      // The header, else the body's copy (a proxy may drop the header).
      const header = Number(res.headers.get('retry-after'));
      const retry = Number.isFinite(header) && header > 0 ? header : env.retryAfter;
      throw this.failure(
        new ApiError(
          env.message || `${res.status} ${res.statusText}`.trim(),
          res.status,
          'http',
          env.code,
          retry,
          env.field,
          env.extra,
        ),
      );
    }
    return res;
  }

  /** This client's credential was refused: the device may be unpaired. */
  protected unauthorized(): void {
    this.auth.handleUnauthorized();
  }

  /** The error a refused request rejects with: as the server said it here;
   * another computer's client words some in that computer's name. */
  protected failure(e: ApiError): ApiError {
    return e;
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

/** A refusal from (or on the way to) another computer, already worded for
 * the person with that computer's name: `describeError` shows it as it is
 * (docs/PLAN-MACHINES.md §6.2). */
export class MachineApiError extends ApiError {
  constructor(
    cause: ApiError,
    message: string,
    /** The other computer's id. */
    readonly machine: string,
  ) {
    super(
      message,
      cause.status,
      cause.kind,
      cause.code,
      cause.retryAfter,
      cause.field,
      cause.extra,
    );
    this.name = 'MachineApiError';
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
  if (e instanceof MachineApiError) {
    return {
      text: e.message,
      ...(e.code === 'machine_unreachable' ? { retry: true } : {}),
      ...(e.code === 'dangerous_blocked' ? { dangerous: true } : {}),
    };
  }
  switch (e.kind) {
    case 'network':
      return {
        text:
          navigator.onLine === false
            ? tr("You're offline.")
            : tr("Can't reach your {computer} (asleep, offline or the tunnel is down).", {
                computer: computerNoun(),
              }),
        retry: true,
      };
    case 'interstitial':
      return {
        text: tr(
          "The tunnel's warning page is in the way: open this page once in the browser, accept it, then reload.",
        ),
      };
    case 'unauthorized':
      return { text: tr('This device is no longer paired.') };
    case 'parse':
      return { text: e.message, retry: true };
    default:
      break;
  }
  const code = e.code ?? '';
  if (e.status === 403 && (code === 'dangerous_blocked' || /dangerous/i.test(e.message))) {
    return {
      text: tr(
        'Only allowed from the {computer}, or allow dangerous actions for this device in Remote › Devices.',
        { computer: computerNoun() },
      ),
      dangerous: true,
    };
  }
  if (e.status === 403 && code === 'desktop_only') {
    return { text: tr('Only the desktop app can do this.') };
  }
  if (e.status === 403 && code === 'untrusted_cwd') {
    return {
      text:
        e.message ||
        tr(
          'That folder has never been opened with Claude Code on the {computer}. Open it there once, or start the session from the desktop.',
          { computer: computerNoun() },
        ),
    };
  }
  if (e.status === 400 && code === 'https_required') {
    return {
      text: e.message || tr('Open Mother Claude through the https:// address of your tunnel.'),
    };
  }
  if (e.status === 409 && code === 'switching_account') {
    // Its account changes right now (docs/ACCOUNTS.md §15.1).
    return { text: tr('Switching accounts — try again in a moment.'), retry: true };
  }
  if ((e.status === 409 || e.status === 410) && code === 'already_decided') {
    return { text: e.message || tr('Already answered elsewhere.'), alreadyDecided: true };
  }
  if (e.status === 409 && code === 'no_channel') {
    return { text: e.message || tr("This session can't take that from here."), noChannel: true };
  }
  if (e.status === 422) return { text: e.message };
  if (e.status === 429) {
    const secs = e.retryAfter ?? 30;
    return {
      text: tr('Too many requests. Try again in {n}s.', { n: secs }),
      retry: true,
      retryAfter: secs,
    };
  }
  if (e.status === 503 && code === 'bridge_not_started') {
    return { text: tr('The API is not started. Start it on the API page.'), bridge: true };
  }
  if (e.status === 501) return { text: tr('This server does not support that yet.') };
  if (e.status >= 500) {
    return { text: tr('The server failed: {error}', { error: e.message }), retry: true };
  }
  return { text: errorMessage(e) };
}

function enc(v: string): string {
  return encodeURIComponent(v);
}

/** An answer that should have been JSON of a known shape, and was not. */
function unreadable(): ApiError {
  return new ApiError(tr('The server sent a response this app could not read.'), 200, 'parse');
}

/** A move's record, or the reason it is none. */
function moveOf(v: unknown): MoveRecord {
  const m = normalizeMove(v);
  if (!m) throw unreadable();
  return m;
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
    throw new ApiError(
      tr('The server sent a response this app could not read.'),
      res.status,
      'parse',
    );
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
