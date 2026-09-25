// TypeScript mirror of the server's API contract (design contract v1, §2.6,
// §2.8, §4, §6). The server emits camelCase JSON; Rust structs live in
// src-tauri/src/claude/model.rs (+ server/pairing.rs, remote.rs, push.rs).
//
// Every field the server may omit is optional here, and consumers must treat
// unknown enum values gracefully — the backend reads undocumented Claude Code
// internals, so values drift.

// ─────────────────────────────────────────────────────────── 4.1 Sessions

export type SessionStatus =
  | 'working'
  | 'waiting'
  | 'idle'
  | 'starting'
  | 'ended'
  | 'failed'
  | 'stopped';

export type Surface =
  | 'vscode'
  | 'terminal'
  | 'tmux'
  | 'desktop'
  | 'jetbrains'
  | 'background'
  | 'mother-claude'
  | 'sdk'
  | 'unknown';

export type ActivityKind =
  | 'thinking'
  | 'tool'
  | 'waiting'
  | 'delegating'
  | 'background'
  | 'compacting'
  | 'retrying'
  | 'idle'
  | 'starting'
  | 'ended';

/** "What is it doing right now" (§3.5). `since` is epoch ms. */
export interface Activity {
  kind: ActivityKind;
  /** Server-built human label, e.g. "Running Bash". */
  label: string;
  tool?: string;
  /** Salient detail, e.g. the command or the file path (≤ 120 chars). */
  detail?: string;
  since?: number;
}

export interface Usage {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  thinking?: number;
}

export interface ContextFill {
  used: number;
  window: number;
  /** 0..100 */
  pct: number;
}

export interface TodoSummary {
  total: number;
  done: number;
  inProgress: number;
  activeForm?: string;
}

export interface AgentCounts {
  running: number;
  total: number;
  workflowsRunning: number;
  backgroundRunning: number;
}

export type ControlChannel = 'owned' | 'hooks' | 'tmux' | 'pty' | 'none';

/** A button the UI can offer next to a limitation (design §10 B2). */
export type LimitAction =
  | 'install-foreign-control'
  | 'reload-plugins'
  | 'continue-owned'
  | 'fork'
  | 'open-terminal'
  | 'open-on-desktop';

/** One thing a session can't do from here, why, and what would fix it. */
export interface CapLimit {
  /** Machine code, e.g. `no_hooks`, `foreign_model`. */
  code: string;
  /** One human sentence. */
  text: string;
  action?: LimitAction;
}

/** What this dashboard can do with a session (§4.1, computed by §5.5; shape
 * per the §10 amendments). New fields are read defensively: an older server
 * may omit them (see shared/caps.ts). */
export interface ControlCaps {
  channel: ControlChannel;
  /** relay = delivered through hooks or tmux, not straight into the CLI. */
  message: 'direct' | 'relay' | 'none';
  approve: boolean;
  answer: boolean;
  interrupt: 'immediate' | 'next-tool' | 'none';
  setModel: boolean;
  setMode: 'any' | 'on-prompt' | 'none';
  setEffort: boolean;
  slashCommands: boolean;
  attachImages: boolean;
  stopTask: boolean;
  fork: boolean;
  continueOwned: boolean;
  lifecycle: { stop: boolean; respawn: boolean; rm: boolean };
  /** Hook events arrived from this session's current process. */
  hooksActive: boolean;
  /** `sync` renames everywhere; `alias` only inside Mother Claude. */
  rename: 'sync' | 'alias' | 'none';
  /** A terminal view (tmux capture / background log) exists. */
  terminal: boolean;
  /** Raw keys can be sent (tmux only). */
  keys: boolean;
  /** `inline` = images as content blocks; `path` = files as @ paths. */
  attach: 'inline' | 'path' | 'none';
  /** How approvals reach the CLI. */
  approveVia: 'control' | 'hook' | 'keys' | 'none';
  /** An idle session can be woken to take a message now (else it waits
   * for its next stopping point). */
  wake: boolean;
  /** What doesn't work, why, and the fix. */
  limits: CapLimit[];
  /** @deprecated pre-amendment free-text notes; read via capLimits(). */
  notes?: string[];
}

export interface ModelRef {
  id: string;
  label: string;
  contextWindow?: number;
}

export interface PrLink {
  number: number;
  url: string;
  repo?: string;
}

export interface SessionError {
  kind: string;
  text: string;
  at: number;
}

export interface SessionSummary {
  id: string;
  name: string;
  title?: string;
  projectName: string;
  cwd: string;
  surface: Surface;
  kind?: string;
  entrypoint?: string;
  owned: boolean;
  live: boolean;
  status: SessionStatus;
  waitingFor?: string;
  activity: Activity;
  model?: ModelRef;
  effort?: string;
  permissionMode?: string;
  gitBranch?: string;
  prLink?: PrLink;
  startedAt?: number;
  lastActivityAt?: number;
  pid?: number;
  jobId?: string;
  version?: string;
  tmuxPane?: string;
  usage: Usage;
  costUsd?: number;
  /** The cost came from Claude's own final `cost-state` record. */
  costFinal?: boolean;
  /** Priced cost written since local midnight (absent when none). */
  costToday?: number;
  context?: ContextFill;
  messageCount: number;
  /** ≤ 240 chars */
  lastPrompt?: string;
  /** ≤ 240 chars */
  lastMessage?: string;
  todos?: TodoSummary;
  agents: AgentCounts;
  queued: number;
  /** Just enough to render list rows and quick actions; the full items come
   * from `GET /inbox`, `GET /sessions/{id}` (`pendingDetail`) and `pending`
   * frames (see PendingService). */
  pending: PendingPreview[];
  lastError?: SessionError;
  remoteControl?: boolean;
  control: ControlCaps;
}

// ─────────────────────────────────────────── 4.2 Pending items & responding

export interface QuestionOption {
  label: string;
  description?: string;
}

export interface Question {
  question: string;
  header?: string;
  options: QuestionOption[];
  multiSelect: boolean;
}

export interface PermissionSuggestion {
  index: number;
  label: string;
  kind: string;
  destination?: string;
  /** Persists beyond this session (settings file) — dangerous. */
  standing: boolean;
  raw: unknown;
}

export type PendingKind = 'permission' | 'question' | 'plan' | 'elicitation';

/** observed = seen on disk, not answerable from here. */
export type PendingSource = 'owned' | 'hook' | 'bridge' | 'sidecar' | 'observed';

export interface PendingItem {
  id: string;
  sessionId: string;
  kind: PendingKind;
  source: PendingSource;
  toolName?: string;
  toolUseId?: string;
  agentId?: string;
  agentType?: string;
  title: string;
  detail?: string;
  input?: unknown;
  questions?: Question[];
  /** Markdown (ExitPlanMode). */
  plan?: string;
  suggestions: PermissionSuggestion[];
  dangerous: boolean;
  /** Why the server marked it dangerous (a sentence, for the card). */
  dangerousReason?: string;
  answerable: boolean;
  createdAt: number;
  /** `input` was capped (4 KB) in this copy; `GET /sessions/{id}` has it all. */
  inputTruncated?: boolean;
}

/** The light copy of a PendingItem carried by every `sessions` frame. */
export interface PendingPreview {
  id: string;
  kind: PendingKind;
  title: string;
  /** ≤ 240 chars: the command, path, question or plan summary. */
  detail?: string;
  toolName?: string;
  agentId?: string;
  dangerous: boolean;
  answerable: boolean;
  createdAt: number;
  questionCount?: number;
  /** Options of a single single-select question (list quick actions). */
  quick?: { options: string[] };
  /** Tolerated: older servers embed full items (these then carry it). */
  sessionId?: string;
  source?: PendingSource;
}

/** A preview with its session filled in (the realtime store adds it). */
export type PendingRef = PendingPreview & { sessionId: string };

/** Why a pending item went away (`pendingResolved` frame). */
export type PendingOutcome =
  | 'allowed'
  | 'denied'
  | 'answered'
  | 'withdrawn'
  | 'expired'
  | 'cancelled';

export interface PendingResolvedBy {
  kind: 'device' | 'desktop' | 'host' | 'timeout';
  /** Device name, or e.g. "Mac". */
  name?: string;
}

export type PermissionMode =
  | 'default'
  | 'acceptEdits'
  | 'plan'
  | 'auto'
  | 'bypassPermissions'
  | 'dontAsk';

/** `POST /api/sessions/{id}/respond` */
export interface RespondBody {
  requestId: string;
  decision: 'allow' | 'deny';
  /** Index into `suggestions` ("always allow"). */
  suggestion?: number;
  /** Deny feedback / plan feedback. */
  message?: string;
  /** AskUserQuestion: question → label (multi: comma-joined) or free text. */
  answers?: Record<string, string>;
  /** Plan approval → switch to this mode. */
  mode?: 'default' | 'acceptEdits' | 'plan' | 'auto' | 'bypassPermissions';
  /** Deny + stop the turn. */
  interrupt?: boolean;
  /** Elicitation: the form content (allow = accept, deny = decline). */
  content?: Record<string, unknown>;
}

// ──────────────────────────────────────────────────────────────── 4.3 Agents

export type AgentKind = 'subagent' | 'workflow-agent' | 'bash' | 'monitor' | 'teammate';
export type AgentStatus = 'running' | 'done' | 'failed' | 'stopped' | 'unknown';

export interface AgentInfo {
  id: string;
  kind: AgentKind;
  sessionId: string;
  parentId?: string;
  depth: number;
  type?: string;
  label: string;
  phase?: string;
  workflowRunId?: string;
  status: AgentStatus;
  background: boolean;
  model?: string;
  startedAt?: number;
  endedAt?: number;
  lastActivityAt?: number;
  activity?: Activity;
  tokens?: number;
  toolUses?: number;
  worktree?: string;
  toolUseId?: string;
  taskId?: string;
  canStop: boolean;
}

export interface WorkflowRun {
  runId: string;
  name: string;
  status: 'running' | 'completed' | 'failed' | 'killed';
  phases: { title: string; detail?: string }[];
  agents: { total: number; running: number; done: number; failed: number };
  startedAt?: number;
  durationMs?: number;
  taskId?: string;
  totalTokens?: number;
}

export interface Todo {
  content: string;
  status: 'pending' | 'in_progress' | 'completed';
  activeForm?: string;
}

export interface BackgroundTask {
  id: string;
  kind: 'bash' | 'agent' | 'workflow' | 'monitor';
  label: string;
  status: string;
  startedAt?: number;
  endedAt?: number;
  outputAvailable: boolean;
}

export interface QueuedMessage {
  id: string;
  text: string;
  createdAt: number;
  via: 'owned' | 'hook-busy' | 'hook-idle' | 'tmux';
  status: 'queued' | 'delivered' | 'cancelled' | 'failed';
  /** Can still be taken back (✕). */
  cancellable?: boolean;
}

export interface ChangedFile {
  path: string;
  added?: number;
  removed?: number;
}

/** `GET /api/sessions/{id}` (Rust `agents_detail`/`todos_detail` → camelCase). */
export interface SessionDetail extends SessionSummary {
  agentsDetail: AgentInfo[];
  workflows: WorkflowRun[];
  tasks: BackgroundTask[];
  todosDetail: Todo[];
  queuedMessages: QueuedMessage[];
  filesChanged: ChangedFile[];
  /** Full pending items (with complete `input`). */
  pendingDetail?: PendingItem[];
}

// ─────────────────────────────────────────────────────────────── 4.4 UiEvent

export type UiEventKind =
  | 'prompt'
  | 'text'
  | 'thinking'
  | 'tool'
  | 'result'
  | 'image'
  | 'notice'
  | 'compact'
  | 'error'
  | 'task'
  | 'relayed'
  | 'todos'
  | 'question'
  | 'plan'
  | 'decision';

export interface UiEvent {
  /** `<record uuid>:<blockIndex>`: stable across REST pages, WS appends and
   * replays, so the client dedupes on it. */
  id: string;
  /** The assistant `message.id` (matches `stream` frames). */
  messageId?: string;
  ts?: number;
  role: 'user' | 'assistant' | 'system';
  kind: UiEventKind;
  text?: string;
  /** Set on events that belong to a subagent's own transcript. */
  agentId?: string;
  tool?: { id: string; name: string; summary: string; input?: unknown };
  result?: {
    toolUseId: string;
    isError: boolean;
    text?: string;
    patch?: { path: string; hunks: unknown[] };
    truncated?: boolean;
  };
  /** `ref` is an /api path; fetch it with ApiService.fetchMedia (needs auth). */
  image?: { mediaType: string; ref: string };
  todos?: Todo[];
  question?: { questions: Question[]; answers?: Record<string, string> };
  plan?: { text: string };
  task?: { taskId: string; status: string; summary?: string };
}

// ─────────────────────────────────────────────── 4.5 Requests and responses

export type SessionScope = 'active' | 'all';

export interface SessionsQuery {
  scope?: SessionScope;
  limit?: number;
  /** Opaque cursor from a previous page's `nextBefore`. */
  before?: string;
}

export interface SessionsPage {
  sessions: SessionSummary[];
  nextBefore?: string;
}

export interface TranscriptQuery {
  /** Opaque cursor (a byte offset) from a previous page's `before`. */
  before?: string;
  /** Only events after this `tail` cursor (the polling fallback). */
  after?: string;
  /** ≤ 300 */
  limit?: number;
  agent?: string;
}

/** Newest-last. `before` pages further back; `tail` is the live cursor for `sub{since}`. */
export interface TranscriptPage {
  events: UiEvent[];
  before?: string;
  tail: string;
  /** `after=` was too far behind for a bounded read: refetch the last page. */
  reset?: boolean;
}

export interface SessionAgents {
  agents: AgentInfo[];
  workflows: WorkflowRun[];
}

export interface FleetSession {
  sessionId: string;
  name: string;
  /** Running agents only, background shells and monitors included (kind
   * `bash`/`monitor`: they have output, not a transcript). */
  agents: AgentInfo[];
  workflows?: WorkflowRun[];
}

/** "3 working · 7 agents running · 2 need you · $4.12 today". */
export interface FleetSummary {
  /** Live sessions currently working. */
  working: number;
  /** Running subagents and workflow agents. */
  agentsRunning: number;
  workflowsRunning?: number;
  /** Running background shells and monitors. */
  backgroundRunning?: number;
  /** Sessions with a request answerable from here. */
  needYou: number;
  /** Priced cost written since local midnight, across sessions (the sum of
   * their `costToday`, not of their lifetime `costUsd`). */
  costToday?: number;
}

export interface Fleet {
  sessions: FleetSession[];
  summary?: FleetSummary;
}

/** `GET /sessions/{id}/mcp` (owned sessions). */
export interface McpServerStatus {
  name: string;
  status: string;
  scope?: string;
  error?: string;
}

/** `POST /sessions/{id}/uploads` answer: where the file was stored. */
export interface UploadResult {
  /** Absolute path on the Mac, referenced as `@<path>` in the message. */
  path: string;
  name?: string;
  size?: number;
}

export type DiffBase = 'working' | 'branch';

export interface CommandInfo {
  name: string;
  description: string;
  argumentHint?: string;
}

export interface ProjectInfo {
  path: string;
  name: string;
  lastActivityAt?: number;
}

export interface Inbox {
  items: PendingItem[];
}

/** Base64 (no data: prefix). png/jpeg/gif/webp, ≤ 5 MB each, ≤ 10 per message. */
export interface ImageAttachment {
  mediaType: string;
  data: string;
}

export interface SpawnBody {
  cwd: string;
  prompt?: string;
  model?: string;
  effort?: string;
  permissionMode?: PermissionMode;
  images?: ImageAttachment[];
}

export interface MessageBody {
  text: string;
  images?: ImageAttachment[];
}

export interface MessageResult {
  delivery: 'sent' | 'queued' | 'relayed';
  messageId: string;
}

export interface ContinueBody {
  fork?: boolean;
  prompt?: string;
  model?: string;
  effort?: string;
  permissionMode?: PermissionMode;
}

export type KeyName =
  | 'Escape'
  | 'Enter'
  | 'Up'
  | 'Down'
  | 'Tab'
  | '1'
  | '2'
  | '3'
  | '4'
  | '5'
  | '6'
  | '7'
  | '8'
  | '9';

export type LifecycleAction = 'stop' | 'respawn' | 'rm';

/** `GET /sessions/{id}/terminal`: the screen as plain text (and its lines). */
export interface TerminalText {
  text: string;
  rows?: string[];
}

/** `GET /api/control/foreign` (desktop only). */
export interface ForeignControlStatus {
  installed: boolean;
  pluginVersion?: string;
  cliAvailable: boolean;
  lastEventAt?: number;
  sessionsWithHooks: number;
  notes: string[];
  /** Installed but switched off (the plugin's scripts exit at once). */
  enabled?: boolean;
  /** Live sessions started before the install (need /reload-plugins). */
  sessionsNeedingReload?: number;
  pluginDir?: string;
  lastError?: string;
}

// ─────────────────────────────────────────────────── 2.6 Pairing & devices

export type Ingress = 'loopback' | 'lan' | 'tunnel' | 'bridge-loopback';

/** `self` = a paired device's code for its own second app (the iOS Home
 * Screen app keeps storage separate from Safari). */
export type PairTarget = 'lan' | 'internet' | 'self';

export interface PairCodeRequest {
  target: PairTarget;
  baseUrl?: string;
}

/** `POST /api/pair/codes`: a single-use, 5-minute pairing code + its QR. */
export interface PairCode {
  code: string;
  /** The code grouped for reading aloud, e.g. `ABCD-EFGH-JKMN`. */
  display?: string;
  /** `{baseUrl}/#/pair?c={code}` (relative for `self` codes). */
  url: string;
  /** Server-rendered QR code (SVG markup). */
  svg: string;
  expiresAt: number;
  ttlSec?: number;
  target: PairTarget;
  baseUrl?: string;
}

export interface PairExchangeBody {
  code: string;
  deviceName: string;
  platform: string;
}

/** `201` — paired straight away. */
export interface PairExchangePaired {
  status: 'paired';
  deviceId: string;
  token: string;
  name: string;
}

/** `202` — an internet pairing waiting for approval on the desktop. */
export interface PairExchangePending {
  status: 'pending';
  pendingId: string;
  pollSecret: string;
  /** 4 digits shown on the phone; the user types them on the desktop. */
  fingerprint: string;
  expiresAt?: number;
}

export type PairExchangeResult = PairExchangePaired | PairExchangePending;

/** `GET /api/pair/pending/{id}` (header `X-Pair-Secret`). The token is returned once. */
/** `contested`: the code was presented twice, so this request can never be
 * approved (`message` says what to do). */
export interface PairPendingStatus {
  status: 'pending' | 'approved' | 'denied' | 'expired' | 'contested';
  token?: string;
  deviceId?: string;
  name?: string;
  message?: string;
}

/** One recent action of a device (Remote › Devices). */
export interface DeviceAction {
  at: number;
  /** e.g. `POST /api/sessions/{id}/respond`. */
  action: string;
  status?: number;
  ip?: string;
  ingress?: string;
  /** Passed a dangerous-action gate. */
  dangerous?: boolean;
}

export interface DeviceInfo {
  id: string;
  name: string;
  platform?: string;
  createdAt?: number;
  lastSeenAt?: number;
  lastIp?: string;
  ingress?: Ingress;
  /** How it paired: `lan` | `internet` | `self`. */
  pairedVia?: string;
  userAgent?: string;
  /** Dangerous actions while on the LAN. */
  allowDangerous: boolean;
  /** Dangerous actions through a tunnel. */
  allowDangerousInternet?: boolean;
  revoked?: boolean;
  revokedAt?: number;
  revokeReason?: string;
  /** Unpaired automatically after this (idle expiry). */
  expiresAt?: number;
  /** Tokens it holds (the Home Screen app adds one). */
  tokenCount?: number;
  /** Open live connections. */
  sockets?: number;
  recent?: DeviceAction[];
}

/** A pairing request waiting for desktop approval. The desktop never sees
 * the 4 digits: the user reads them on the device and types them. */
export interface PendingPair {
  id: string;
  deviceName?: string;
  /** Tolerated alias of `deviceName`. */
  name?: string;
  platform?: string;
  userAgent?: string;
  /** Client IP (tunnel-aware). */
  clientIp?: string;
  /** Tolerated alias of `clientIp`. */
  ip?: string;
  ingress?: Ingress;
  /** The tunnel it came through (`ngrok`, `cloudflared`, `tailscale`, …). */
  provider?: string;
  codeTarget?: PairTarget;
  /** `pending` | `approved` | `denied` | `expired`. */
  status?: string;
  /** The code was presented twice: this request cannot be approved. */
  contested?: boolean;
  fingerprintAttemptsLeft?: number;
  createdAt?: number;
  expiresAt?: number;
}

export interface DevicesList {
  devices: DeviceInfo[];
  pending: PendingPair[];
  /** Devices unused this many days are unpaired automatically. */
  idleDays?: number;
}

export interface DeviceUpdate {
  name?: string;
  allowDangerous?: boolean;
  allowDangerousInternet?: boolean;
}

/** `local-script` = the on-disk token of scripts on the Mac. */
export type PrincipalKind = 'desktop' | 'device' | 'local-script';

/** `GET /api/me` (also delivered in the WS `hello` frame). */
export interface MeInfo {
  principal: PrincipalKind;
  deviceId?: string;
  name?: string;
  ingress: Ingress;
  /** The network the request really came from (a proxied request is `tunnel`). */
  effectiveIngress?: Ingress;
  local: boolean;
  /** May take dangerous actions right now (from where it connects). */
  allowDangerous: boolean;
  allowDangerousLan?: boolean;
  allowDangerousInternet?: boolean;
  platform?: string;
  pairedAt?: number;
  clientIp?: string;
}

// ───────────────────────────────────────────────── 2.8 Remote access/tunnels

export type TunnelProvider = 'ngrok' | 'cloudflared' | 'tailscale';

export interface DetectedTunnel {
  provider: TunnelProvider | string;
  url: string;
  upstream?: string;
  upstreamPort?: number;
  /** ngrok's traffic inspector is on (it records request bodies). */
  inspect?: boolean;
  /** `http` | `tcp`. */
  kind?: string;
  /** Which of our ports it reaches: `tunnel` | `main` | `bridge` | … */
  target?: string;
}

/** `GET /api/remote` (desktop only). */
export interface RemoteInfo {
  lan: {
    enabled?: boolean;
    addresses: { ip: string; interface?: string; url: string }[];
    /** Addresses deliberately not bound (public IPs, …) and why. */
    skipped?: { ip: string; interface?: string; reason: string }[];
    port: number;
    /** Leaf TLS certificate fingerprint. */
    fingerprint: string;
    /** The local CA the phone can trust. */
    caFingerprint?: string;
    caUrl?: string;
    mobileconfigUrl?: string;
    bindTailscale?: boolean;
    error?: string;
    note?: string;
  };
  internet: {
    enabled: boolean;
    /** The tunnel listener is bound right now. */
    listening?: boolean;
    port: number;
    publicUrl?: string;
    manualUrl?: string;
    /** Always true: internet pairings wait for approval on the Mac. */
    requireApproval: boolean;
    detected: DetectedTunnel[];
    checkedAt?: number;
    installed?: { ngrok?: boolean; cloudflared?: boolean; tailscale?: boolean };
    commands: { ngrok: string; cloudflared: string; tailscale: string; tailscaleServe?: string };
    recommended?: string;
    warnings: string[];
  };
  bridge: { port: number; running: boolean; exposed?: boolean };
  devices?: { idleDays?: number; active?: number; pending?: number };
}

/** `PUT /api/remote` (unknown fields are refused). */
export interface RemoteUpdate {
  internetEnabled?: boolean;
  /** `null` or `''` clears it. */
  manualUrl?: string | null;
  bindTailscale?: boolean;
  deviceIdleDays?: number;
}

// ──────────────────────────────────────────────────────────────── 6 Push

export type SessionPushPref = 'follow' | 'mute';

/** `GET /api/push/prefs` (per device). */
export interface PushPrefs {
  permission: boolean;
  question: boolean;
  plan: boolean;
  finished: boolean;
  failed: boolean;
  /** Claude asked to notify you (PushNotification tool, Notification hooks). */
  claude?: boolean;
  /** A session has been waiting idle for your next prompt. */
  idle?: boolean;
  /** Include the command / question text in notifications (off: only the
   * session name and the kind of event). */
  showDetails?: boolean;
  /** Per-session follow/mute; absent = the event prefs above. */
  sessions?: Record<string, SessionPushPref>;
  /** Sessions with a one-shot "notify me when this turn finishes". */
  turnFinished?: string[];
  /** Subscriptions this device has stored. */
  subscriptions?: number;
  /** This caller can subscribe at all (paired devices only). */
  available?: boolean;
}

/** `PUT /api/push/prefs`: any subset; maps are merged (`null` / `false`
 * removes an entry). */
export interface PushPrefsUpdate {
  permission?: boolean;
  question?: boolean;
  plan?: boolean;
  finished?: boolean;
  failed?: boolean;
  claude?: boolean;
  idle?: boolean;
  showDetails?: boolean;
  sessions?: Record<string, SessionPushPref | 'default' | null>;
  turnFinished?: Record<string, boolean>;
}

/** `GET /api/push/vapid`. */
export interface VapidInfo {
  /** base64url, uncompressed P-256. */
  publicKey: string;
  available?: boolean;
  secureContextHint?: {
    /** `desktop` | `lan_certificate` | `ok`. */
    code: string;
    text?: string;
    publicUrl?: string;
    /** Guidance text: on iPhone/iPad push works only in the Home Screen app. */
    iosHomeScreen?: string;
    requiresUserGesture?: boolean;
  };
}

export interface PushSubscriptionBody {
  endpoint: string;
  keys: { p256dh: string; auth: string };
  /** The key this subscription was made with (base64url); lets the server
   * spot a subscription made with an older key. */
  applicationServerKey?: string;
}

// ─────────────────────────────────────────────────────── 4.6 WebSocket v2

export type StreamKind = 'text' | 'thinking' | 'tool' | 'status';

export interface HelloFrame {
  t: 'hello';
  v: number;
  me: MeInfo;
  serverTime: number;
}

export interface SessionsFullFrame {
  t: 'sessions';
  full: true;
  sessions: SessionSummary[];
  seq: number;
}

export interface SessionsDiffFrame {
  t: 'sessions';
  full: false;
  upsert?: SessionSummary[];
  remove?: string[];
  seq: number;
}

export type SessionsFrame = SessionsFullFrame | SessionsDiffFrame;

/** Appends for a subscribed session (or one of its agents). Clients dedupe
 * by event id. */
export interface TranscriptFrame {
  t: 'transcript';
  session: string;
  agent?: string;
  events: UiEvent[];
  tail: string;
}

/** The replay from `since` was too big: refetch the transcript over REST. */
export interface TranscriptResetFrame {
  t: 'transcriptReset';
  session: string;
  agent?: string;
}

/** Owned sessions' live token-by-token deltas, keyed by the assistant
 * message id and content-block index. A UiEvent with the same `messageId`
 * replaces the bubble. */
export interface StreamFrame {
  t: 'stream';
  session: string;
  messageId?: string;
  index?: number;
  kind: StreamKind;
  delta?: string;
  tool?: { id?: string; name?: string; summary?: string; input?: unknown; phase?: string };
  status?: string;
}

/** A full pending item (for session and inbox subscribers). */
export interface PendingFrame {
  t: 'pending';
  item: PendingItem;
}

/** A pending item went away, and who or what settled it. */
export interface PendingResolvedFrame {
  t: 'pendingResolved';
  id: string;
  session: string;
  outcome: PendingOutcome;
  by?: PendingResolvedBy;
  at: number;
  /** One human line, e.g. "Allowed by Mac · Always allow Bash(npm test:*) in this session". */
  text?: string;
}

export interface AgentsFrame {
  t: 'agents';
  session: string;
  agents: AgentInfo[];
  workflows: WorkflowRun[];
}

export interface FleetFrame {
  t: 'fleet';
  sessions: FleetSession[];
  summary?: FleetSummary;
}

export interface NoticeFrame {
  t: 'notice';
  level: 'info' | 'success' | 'warning' | 'error' | string;
  text: string;
  session?: string;
}

/** Desktop only: devices or pending pairings changed — refetch. */
export interface DevicesFrame {
  t: 'devices';
}

/** Desktop only: a new pairing request. */
export interface PairRequestFrame {
  t: 'pairRequest';
  pending: PendingPair;
}

/** The client lagged and frames were dropped — refetch and resubscribe. */
export interface ResyncFrame {
  t: 'resync';
}

export interface PingFrame {
  t: 'ping';
}

export interface PongFrame {
  t: 'pong';
}

export type ServerFrame =
  | HelloFrame
  | SessionsFrame
  | TranscriptFrame
  | TranscriptResetFrame
  | StreamFrame
  | PendingFrame
  | PendingResolvedFrame
  | AgentsFrame
  | FleetFrame
  | NoticeFrame
  | DevicesFrame
  | PairRequestFrame
  | ResyncFrame
  | PingFrame
  | PongFrame;

export type ClientFrame =
  | { t: 'auth'; token: string }
  | { t: 'sub'; session: string; agent?: string; since?: string }
  | { t: 'unsub'; session: string; agent?: string }
  | { t: 'subFleet' }
  | { t: 'unsubFleet' }
  | { t: 'subInbox' }
  | { t: 'unsubInbox' }
  | { t: 'ping' }
  | { t: 'pong' }
  | { t: 'visible'; v: boolean };

// ─────────────────────────────────── Launch defaults (`GET /api/defaults`)

export interface ModelOption {
  value: string;
  label: string;
  description?: string;
}

/** What the spawn/continue forms offer and pre-select (read by the server
 * from the user's own Claude settings). */
export interface LaunchDefaults {
  model?: string;
  effort?: string;
  models: ModelOption[];
  efforts?: string[];
}

/** Launch/continue overrides; unset = the user's own Claude settings. */
export interface LaunchOverrides {
  model?: string;
  effort?: string;
  permissionMode?: PermissionMode;
}

// ──────────────────────────────────────── The Claude HTTP bridge (/bridge/*)

/** Defaults every conversation the bridge creates inherits, chosen when the
 * API is started. Every one is overridable per request. */
export interface BridgeDefaults {
  cwd?: string;
  model?: string;
  effort?: string;
  thinking?: 'on' | 'off';
  permissionMode?: string;
  settingSources?: string[];
  systemPromptAppend?: string;
  /** The conversation unaddressed messages join. */
  defaultThread?: string;
}

/** A conversation the bridge is running. */
export interface BridgeThread {
  threadId: string;
  cwd: string;
  createdAt: number;
  model?: string;
  effort?: string;
  permissionMode: string;
  title?: string;
  persisted: boolean;
  activeTurn?: string;
}

/** Everything the API screen needs: `GET /api/bridge`. */
export interface BridgeStatus {
  enabled: boolean;
  running: boolean;
  port?: number;
  url?: string;
  startedAt?: number;
  requireToken: boolean;
  hostBuilt: boolean;
  hostRunning: boolean;
  messagesApi: boolean;
  claudeAuthenticated: boolean;
  claudeAccount: string;
  activeOperations: number;
  defaults: BridgeDefaults;
  activeThread?: string;
  threads: BridgeThread[];
  docsUrl?: string;
  consoleUrl?: string;
}

// ──────────────────────────────────────────── Git (`/diff`, `/file-patch`)

export interface FileChange {
  path: string;
  status: string;
  additions: number;
  deletions: number;
}

export interface CommitInfo {
  id: string;
  summary: string;
  author: string;
  time: number;
}

export interface WorktreeInfo {
  name: string;
  path: string;
  locked: boolean;
}

export interface GitOverview {
  isRepo: boolean;
  repoRoot?: string;
  branch?: string;
  head?: string;
  files: FileChange[];
  additions: number;
  deletions: number;
  commits: CommitInfo[];
  worktrees: WorktreeInfo[];
}

// ─────────────────────────────────────────── Desktop OS permissions (invoke)

export interface PermissionInfo {
  id: string;
  label: string;
  description: string;
  /** true / false when detectable; null when it can't be checked. */
  granted: boolean | null;
  required: boolean;
  settingsUrl?: string;
  steps: string[];
}

// ───────────────────────────────────────────── Legacy (`GET /api/services`)

export interface ServicesInfo {
  mcpServers?: Record<string, unknown>;
  daemon?: { reachable?: boolean; raw?: string; error?: string };
  backgroundJobs?: { id: string; cwd: string; state: string }[];
}
