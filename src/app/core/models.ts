// TypeScript mirrors of the Rust DTOs (src-tauri/src/claude/registry.rs &
// server). Kept in sync by hand; the server emits camelCase.

export type Surface = 'cli' | 'vs-code' | 'jet-brains' | 'desktop' | 'unknown';

export type SessionState =
  | 'working'
  | 'needs-input'
  | 'idle'
  | 'completed'
  | 'failed'
  | 'stopped'
  | 'unknown';

export type PendingKind = 'permission' | 'question';

export interface QuestionOption {
  label: string;
  description?: string;
}

export interface PendingInput {
  kind: PendingKind;
  tool?: string;
  prompt?: string;
  /** Very short topic chip (AskUserQuestion `header`), e.g. "Auth method". */
  header?: string;
  options?: QuestionOption[];
  multiSelect?: boolean;
  /** Salient context: the Bash command / file path / plan text, display-ready. */
  detail?: string;
  requestId?: string;
  answerable: boolean;
  dangerous: boolean;
}

export interface UsageSummary {
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  totalTokens: number;
}

/** A background task/agent/workflow a session launched. */
export interface BackgroundTask {
  id: string;
  kind: 'bash' | 'agent' | 'workflow' | string;
  label: string;
  status: 'running' | 'completed' | 'failed' | 'killed' | string;
  startedAt?: number;
  endedAt?: number;
}

export interface ModelOption {
  value: string;
  label: string;
  description?: string;
}

/** What the spawn/continue forms should offer and pre-select (server-read
 * from the user's own Claude settings). */
export interface LaunchDefaults {
  model?: string;
  effort?: string;
  models: ModelOption[];
  efforts: string[];
}

/** Overrides for launching/continuing a session; unset = the user's own
 * Claude settings (exactly what VS Code / the CLI would use). */
export interface LaunchOverrides {
  model?: string;
  effort?: string;
  thinking?: 'on' | 'off';
}

export interface Session {
  id: string;
  cwd: string;
  projectName: string;
  surface: Surface;
  owned: boolean;
  state: SessionState;
  model?: string;
  title?: string;
  startedAt?: number;
  lastActivity?: number;
  pid?: number;
  kind?: string;
  gitBranch?: string;
  running: boolean;
  messageCount: number;
  usage: UsageSummary;
  pending?: PendingInput;
  tasks?: BackgroundTask[];
  canInject: boolean;
}

export interface ContentBlock {
  type: string;
  text?: string;
  name?: string;
  id?: string;
  tool_use_id?: string;
  input?: unknown;
  content?: unknown;
  is_error?: boolean;
}

export interface TranscriptMessage {
  role?: string;
  model?: string;
  content?: string | ContentBlock[];
  stop_reason?: string;
  usage?: Record<string, unknown>;
}

export interface TranscriptEvent {
  type: string;
  uuid?: string;
  parentUuid?: string;
  timestamp?: string;
  sessionId?: string;
  cwd?: string;
  gitBranch?: string;
  subtype?: string;
  message?: TranscriptMessage;
  content?: unknown;
  [key: string]: unknown;
}

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

export interface Pairing {
  url: string;
  token: string;
  svg: string;
  fingerprint: string;
  addresses: string[];
  port: number;
  tls: boolean;
}

// Adjacently-tagged ServerEvent: { kind, data }.
export type ServerEvent =
  | { kind: 'sessions'; data: Session[] }
  | { kind: 'transcript'; data: { sessionId: string; events: TranscriptEvent[] } }
  | { kind: 'hook'; data: unknown }
  | { kind: 'pending'; data: { sessionId: string; pending?: PendingInput } }
  | { kind: 'notice'; data: string };
