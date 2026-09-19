/**
 * Mother Claude — Claude HTTP bridge runtime host.
 *
 * One long-lived Node process that owns N Claude Agent SDK conversations and
 * speaks newline-delimited JSON with the Rust core over stdin/stdout. The Rust
 * side owns HTTP, auth, validation, the operation registry and the SSE logs;
 * this process owns nothing but the SDK.
 *
 * Why a host rather than one process per session (as `agent-bridge.ts` does for
 * the dashboard's Path A): the bridge has to answer `GET /models`,
 * `GET /threads` and `POST /threads` before any conversation exists, and a
 * process-per-session design cannot. Multiplexing also keeps the ~216 MB
 * bundled CLI resident once instead of once per conversation.
 *
 * ## Protocol
 *
 * In (one JSON object per line):
 *   {"id":"<rpc id>","op":"<name>", ...params}
 *
 * Out (one JSON object per line), tagged by `t`:
 *   {"t":"ready","pid":<n>}                                 once, at startup
 *   {"t":"reply","id":"…","ok":true,"result":…}             RPC result
 *   {"t":"reply","id":"…","ok":false,"error":{code,message}}
 *   {"t":"event","sessionId":"…","turnId":"…"|null,"message":<SDKMessage>}
 *   {"t":"turn","sessionId":"…","turnId":"…","status":"completed"|"failed"
 *                |"interrupted","result":…,"error":…}
 *   {"t":"request","requestId":"…","sessionId":"…","turnId":…,"kind":…,…}
 *   {"t":"session","sessionId":"…","state":"closed","reason":"…"}
 *   {"t":"log","level":"…","message":"…"}
 *
 * A `request` blocks the SDK until the Rust side sends
 * `{"op":"request.respond","requestId":…,"result":…}` — that is the
 * human-in-the-loop hop, and it is the same channel for `canUseTool` tool
 * approvals and for `ask_user` questions.
 */
import {
  query,
  createSdkMcpServer,
  tool,
  listSessions,
  getSessionInfo,
  getSessionMessages,
  listSubagents,
  getSubagentMessages,
  forkSession,
  renameSession,
  tagSession,
  deleteSession,
  type Query,
  type Options,
  type SDKMessage,
  type SDKUserMessage,
  type PermissionMode,
  type EffortLevel,
  type PermissionResult,
  type PermissionUpdate,
} from '@anthropic-ai/claude-agent-sdk';
import { z } from 'zod';
import * as readline from 'node:readline';
import { randomUUID } from 'node:crypto';

// --------------------------------------------------------------------------
// Output framing
// --------------------------------------------------------------------------

/** Serialize one frame. Never throws: a frame that cannot be serialized is
 *  replaced by a log line, because losing stdout desynchronizes the Rust core. */
function send(frame: Record<string, unknown>): void {
  let line: string;
  try {
    line = JSON.stringify(frame);
  } catch (err) {
    line = JSON.stringify({
      t: 'log',
      level: 'error',
      message: `unserializable frame: ${String(err)}`,
    });
  }
  process.stdout.write(line + '\n');
}

function log(level: 'debug' | 'info' | 'warn' | 'error', message: string): void {
  send({ t: 'log', level, message });
}

function errorShape(err: unknown): { code: string; message: string } {
  if (err && typeof err === 'object' && 'code' in err && 'message' in err) {
    const e = err as { code: unknown; message: unknown };
    if (typeof e.code === 'string' && typeof e.message === 'string') {
      return { code: e.code, message: e.message };
    }
  }
  const message = err instanceof Error ? err.message : String(err);
  return { code: 'claude_error', message };
}

class HostError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

// --------------------------------------------------------------------------
// Streaming input
// --------------------------------------------------------------------------

/**
 * A push-driven async iterable. `query()` consumes it exactly once for the
 * lifetime of the conversation, so every turn after the first is a `push()`
 * rather than a new query.
 */
class InputQueue implements AsyncIterable<SDKUserMessage> {
  private readonly buffered: SDKUserMessage[] = [];
  private readonly waiting: ((value: SDKUserMessage | null) => void)[] = [];
  private closed = false;

  push(message: SDKUserMessage): void {
    if (this.closed) return;
    const waiter = this.waiting.shift();
    if (waiter) waiter(message);
    else this.buffered.push(message);
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    while (this.waiting.length) this.waiting.shift()!(null);
  }

  async *[Symbol.asyncIterator](): AsyncGenerator<SDKUserMessage> {
    for (;;) {
      const buffered = this.buffered.shift();
      if (buffered !== undefined) {
        yield buffered;
        continue;
      }
      if (this.closed) return;
      const next = await new Promise<SDKUserMessage | null>((resolve) => {
        this.waiting.push(resolve);
      });
      if (next === null) return;
      yield next;
    }
  }
}

// --------------------------------------------------------------------------
// Pending human-in-the-loop requests
// --------------------------------------------------------------------------

interface Pending {
  resolve: (result: unknown) => void;
  reject: (err: unknown) => void;
}

const pendingRequests = new Map<string, Pending>();

/** Raise a request to the Rust core and block until it answers. */
function askHuman(
  kind: 'permission' | 'question' | 'elicitation',
  sessionId: string,
  turnId: string | null,
  payload: Record<string, unknown>,
  signal?: AbortSignal,
): Promise<unknown> {
  const requestId = randomUUID();
  return new Promise((resolve, reject) => {
    pendingRequests.set(requestId, { resolve, reject });
    const abort = () => {
      if (pendingRequests.delete(requestId)) {
        reject(new HostError('interrupted', 'The request was aborted before it was answered.'));
      }
    };
    signal?.addEventListener('abort', abort, { once: true });
    send({ t: 'request', requestId, sessionId, turnId, kind, ...payload });
  });
}

// --------------------------------------------------------------------------
// Sessions
// --------------------------------------------------------------------------

interface SessionOptions {
  cwd?: string;
  model?: string;
  effort?: string;
  thinking?: string;
  permissionMode?: string;
  resume?: string;
  forkSession?: boolean;
  systemPromptAppend?: string;
  allowedTools?: string[];
  disallowedTools?: string[];
  additionalDirectories?: string[];
  settingSources?: string[];
  mcpServers?: Record<string, unknown>;
  outputFormat?: Record<string, unknown>;
  maxTurns?: number;
  maxBudgetUsd?: number;
  includePartialMessages?: boolean;
  title?: string;
  skills?: string[] | 'all';
  agents?: Record<string, unknown>;
}

interface Session {
  id: string;
  cwd: string;
  input: InputQueue;
  query: Query;
  /** The turn currently accepting output, or null between turns. */
  turnId: string | null;
  closed: boolean;
  /** Resolves when query() has produced its first `system/init`. */
  ready: Promise<void>;
  /** Resolves when the conversation's stream has ended, for any reason. */
  ended: Promise<void>;
  options: SessionOptions;
}

const sessions = new Map<string, Session>();

function requireSession(sessionId: unknown): Session {
  if (typeof sessionId !== 'string' || !sessionId) {
    throw new HostError('invalid_request', '`sessionId` is required.');
  }
  const session = sessions.get(sessionId);
  if (!session || session.closed) {
    throw new HostError('thread_not_found', `No live conversation ${sessionId}.`);
  }
  return session;
}

/** The salient argument of a tool call, for the approval card. */
const SALIENT_ARGS: Record<string, string[]> = {
  Bash: ['command'],
  Edit: ['file_path'],
  Write: ['file_path'],
  Read: ['file_path'],
  Grep: ['pattern', 'path'],
  Glob: ['pattern'],
  WebFetch: ['url'],
  WebSearch: ['query'],
  Task: ['description'],
  NotebookEdit: ['notebook_path'],
};

function describeInput(toolName: string, input: unknown): string {
  if (input == null || typeof input !== 'object') return '';
  const record = input as Record<string, unknown>;
  const keys = SALIENT_ARGS[toolName];
  const parts = keys
    ?.map((key) => record[key])
    .filter((value) => value != null)
    .map((value) => (typeof value === 'string' ? value : JSON.stringify(value)));
  const text = parts?.length ? parts.join(' · ') : JSON.stringify(record);
  return text.length > 700 ? `${text.slice(0, 700)}…` : text;
}

function isDangerous(toolName: string, input: unknown): boolean {
  const text = `${toolName} ${JSON.stringify(input)}`.toLowerCase();
  return (
    text.includes('bypasspermissions') ||
    text.includes('dangerously-skip-permissions') ||
    text.includes('rm -rf')
  );
}

const optionSchema = z.union([
  z.string(),
  z.object({ label: z.string(), description: z.string().optional() }),
]);

/** `ask_user`, so a question reaches the bridge instead of a TTY that is not there. */
function askUserServer(sessionId: string, getTurnId: () => string | null) {
  return createSdkMcpServer({
    name: 'mother-claude-bridge',
    version: '1.0.0',
    tools: [
      tool(
        'ask_user',
        'Ask the human operator a question and wait for their answer. When the question has ' +
          'natural choices, give 2-4 options, each with a short label and a one-line description. ' +
          'Set multiSelect true when several options can apply together. The user can always ' +
          'type a free-text answer instead of picking an option.',
        {
          question: z.string(),
          header: z.string().optional().describe('Very short topic chip, e.g. "Auth method"'),
          options: z.array(optionSchema).optional(),
          multiSelect: z.boolean().optional(),
        },
        async (args) => {
          const answer = await askHuman('question', sessionId, getTurnId(), {
            prompt: args.question,
            header: args.header,
            options: args.options,
            multiSelect: args.multiSelect ?? false,
          });
          // Never hand the model String(object) — that is the literal
          // "[object Object]", which reads as a real answer.
          let text: string;
          if (answer && typeof answer === 'object') {
            const record = answer as Record<string, unknown>;
            if (typeof record.answer === 'string') text = record.answer;
            else if (record.behavior === 'deny') text = 'The operator declined to answer.';
            else text = JSON.stringify(answer);
          } else {
            text = String(answer ?? '');
          }
          return { content: [{ type: 'text' as const, text }] };
        },
      ),
    ],
  });
}

function buildOptions(sessionId: string, opts: SessionOptions, session: () => Session): Options {
  const permissionMode = (opts.permissionMode ?? 'default') as PermissionMode;
  const options: Options = {
    cwd: opts.cwd,
    permissionMode,
    // Without this the SDK refuses to enter bypassPermissions at all, so
    // `POST /threads/{id}/permission-mode` would silently never take effect.
    // The bridge gates that mode to local clients before it ever gets here.
    allowDangerouslySkipPermissions: true,
    includePartialMessages: opts.includePartialMessages ?? true,
    mcpServers: { 'mother-claude-bridge': askUserServer(sessionId, () => session().turnId) },
    // The native AskUserQuestion tool needs a TTY the host does not have.
    disallowedTools: ['AskUserQuestion', ...(opts.disallowedTools ?? [])],
    canUseTool: async (toolName, input, ctx): Promise<PermissionResult> => {
      const decision = (await askHuman(
        'permission',
        sessionId,
        session().turnId,
        {
          tool: toolName,
          // The SDK hands us bridge-rendered prompt text; prefer it over
          // anything reconstructed from the raw tool input.
          prompt: ctx.title ?? ctx.displayName ?? `Claude wants to use ${toolName}.`,
          detail: ctx.description ?? describeInput(toolName, input),
          blockedPath: ctx.blockedPath,
          decisionReason: ctx.decisionReason,
          suggestions: ctx.suggestions,
          toolUseId: ctx.toolUseID,
          input,
          dangerous: isDangerous(toolName, input),
        },
        ctx.signal,
      )) as {
        behavior?: string;
        updatedInput?: Record<string, unknown>;
        updatedPermissions?: PermissionUpdate[];
        message?: string;
      } | null;

      if (decision?.behavior === 'allow') {
        return {
          behavior: 'allow',
          updatedInput: decision.updatedInput ?? (input as Record<string, unknown>),
          ...(decision.updatedPermissions ? { updatedPermissions: decision.updatedPermissions } : {}),
        };
      }
      return {
        behavior: 'deny',
        message: decision?.message ?? 'Denied from the Mother Claude bridge.',
      };
    },
  };

  if (opts.model) options.model = opts.model;
  if (opts.effort) options.effort = opts.effort as EffortLevel;
  if (opts.thinking === 'off') options.thinking = { type: 'disabled' };
  else if (opts.thinking === 'on') options.thinking = { type: 'adaptive' };
  if (opts.allowedTools?.length) options.allowedTools = opts.allowedTools;
  if (opts.additionalDirectories?.length) options.additionalDirectories = opts.additionalDirectories;
  if (opts.settingSources) options.settingSources = opts.settingSources as Options['settingSources'];
  if (opts.maxTurns) options.maxTurns = opts.maxTurns;
  if (opts.maxBudgetUsd) options.maxBudgetUsd = opts.maxBudgetUsd;
  if (opts.title) options.title = opts.title;
  if (opts.skills) options.skills = opts.skills;
  if (opts.agents) options.agents = opts.agents as Options['agents'];
  if (opts.outputFormat) options.outputFormat = opts.outputFormat as Options['outputFormat'];
  if (opts.systemPromptAppend) {
    options.systemPrompt = {
      type: 'preset',
      preset: 'claude_code',
      append: opts.systemPromptAppend,
    };
  }
  if (opts.mcpServers) {
    options.mcpServers = {
      ...(options.mcpServers ?? {}),
      ...(opts.mcpServers as Options['mcpServers']),
    };
  }

  // Session identity. `sessionId` pre-assigns a brand-new conversation's id so
  // the HTTP caller already holds it; `resume` continues an existing one. They
  // are mutually exclusive unless forking — passing `resume` for an id that has
  // no transcript fails the whole query with "No conversation found".
  if (opts.resume) {
    options.resume = opts.resume;
    if (opts.forkSession) {
      options.forkSession = true;
      options.sessionId = sessionId;
    }
  } else {
    options.sessionId = sessionId;
  }

  return options;
}

/**
 * Claude Code reports a missing or expired sign-in as ordinary turn output
 * rather than a typed failure. Recognising that one string is what lets the
 * bridge answer 401 with an actionable message instead of 200 with a sentence.
 */
const LOGIN_REQUIRED = /not logged in|please run \/login|invalid api key|authentication_error/i;

/** The error envelope for a turn that did not produce an answer. */
function turnError(message: Record<string, unknown>): { code: string; message: string } {
  const text = Array.isArray(message.errors)
    ? message.errors.join('; ')
    : typeof message.result === 'string' && message.result.trim()
      ? message.result
      : String(message.subtype ?? 'The turn failed.');
  return {
    code: LOGIN_REQUIRED.test(text) ? 'claude_login_required' : 'claude_error',
    message: text,
  };
}

/** Terminal result payload for a turn, mirrored from SDKResultMessage. */
function turnResult(message: Record<string, unknown>): Record<string, unknown> {
  return {
    subtype: message.subtype,
    result: message.result ?? null,
    structured_output: message.structured_output ?? null,
    is_error: message.is_error ?? false,
    errors: message.errors ?? null,
    num_turns: message.num_turns ?? null,
    stop_reason: message.stop_reason ?? null,
    duration_ms: message.duration_ms ?? null,
    duration_api_ms: message.duration_api_ms ?? null,
    total_cost_usd: message.total_cost_usd ?? null,
    usage: message.usage ?? null,
    modelUsage: message.modelUsage ?? null,
    permission_denials: message.permission_denials ?? [],
    session_id: message.session_id ?? null,
    uuid: message.uuid ?? null,
  };
}

/** Drain one conversation's message stream for its whole lifetime. */
async function pump(session: Session, onReady: () => void, onEnded: () => void): Promise<void> {
  let ready = false;
  let failure: { code: string; message: string } | null = null;
  try {
    for await (const message of session.query as AsyncIterable<SDKMessage>) {
      const record = message as unknown as Record<string, unknown>;
      send({ t: 'event', sessionId: session.id, turnId: session.turnId, message: record });

      if (!ready && record.type === 'system' && record.subtype === 'init') {
        ready = true;
        onReady();
      }

      if (record.type === 'result') {
        const turnId = session.turnId;
        session.turnId = null;
        if (turnId) {
          // `subtype` alone is not enough: the SDK reports an unusable account
          // as subtype 'success' with is_error set and the failure in `result`.
          // Calling that a completed turn would hand the caller HTTP 200 and a
          // sentence where an answer should be.
          const failed = record.subtype !== 'success' || record.is_error === true;
          send({
            t: 'turn',
            sessionId: session.id,
            turnId,
            status: failed ? 'failed' : 'completed',
            result: turnResult(record),
            error: failed ? turnError(record) : null,
          });
        }
      }
    }
  } catch (err) {
    failure = errorShape(err);
    log('error', `session ${session.id} stream failed: ${failure.message}`);
    if (!ready) onReady();
  } finally {
    // Settle any turn still in flight, whether the stream threw or simply
    // ended. Without this a conversation that dies (or is closed) mid-turn
    // leaves its operation Running forever, holding one of the 16 slots for
    // the lifetime of the process.
    if (session.turnId) {
      send({
        t: 'turn',
        sessionId: session.id,
        turnId: session.turnId,
        status: 'failed',
        result: null,
        error: failure ?? {
          code: 'claude_error',
          message: 'The conversation ended before the turn finished.',
        },
      });
      session.turnId = null;
    }
    session.closed = true;
    sessions.delete(session.id);
    onEnded();
    send({ t: 'session', sessionId: session.id, state: 'closed', reason: 'stream ended' });
  }
}

async function createSession(params: Record<string, unknown>): Promise<Record<string, unknown>> {
  const sessionId = String(params.sessionId ?? randomUUID());
  if (sessions.has(sessionId)) {
    throw new HostError('invalid_state', `Conversation ${sessionId} is already live.`);
  }
  const opts = (params.options ?? {}) as SessionOptions;
  const input = new InputQueue();

  let resolveReady!: () => void;
  const ready = new Promise<void>((resolve) => {
    resolveReady = resolve;
  });
  let resolveEnded!: () => void;
  const ended = new Promise<void>((resolve) => {
    resolveEnded = resolve;
  });

  const session: Session = {
    id: sessionId,
    cwd: opts.cwd ?? process.cwd(),
    input,
    // Assigned immediately below; buildOptions only reads it lazily.
    query: undefined as unknown as Query,
    turnId: null,
    closed: false,
    ready,
    ended,
    options: opts,
  };
  sessions.set(sessionId, session);

  try {
    session.query = query({
      prompt: input,
      options: buildOptions(sessionId, opts, () => session),
    });
  } catch (err) {
    sessions.delete(sessionId);
    throw err;
  }

  void pump(session, resolveReady, resolveEnded);

  // Do NOT wait for `system/init`. With streaming input the SDK does not spawn
  // its CLI until the first user message arrives, so init cannot arrive before
  // the first turn — waiting for it made every create take the full timeout.
  //
  // Instead, give the conversation a brief moment to fail: an immediate crash
  // (bad executable, unusable options) ends the stream within milliseconds and
  // should be an HTTP error, while everything else returns straight away and
  // surfaces on the first turn, where it belongs.
  const grace = Number(params.readyGraceMs ?? 250);
  await Promise.race([
    ready,
    ended,
    new Promise<void>((resolve) => setTimeout(resolve, grace)),
  ]);
  if (session.closed) {
    throw new HostError('claude_error', `Conversation ${sessionId} ended before it could start.`);
  }

  return { sessionId, cwd: session.cwd };
}

function userMessage(content: unknown, priority?: string): SDKUserMessage {
  const message: SDKUserMessage = {
    type: 'user',
    message: { role: 'user', content: content as SDKUserMessage['message']['content'] },
    parent_tool_use_id: null,
  };
  if (priority === 'now' || priority === 'next' || priority === 'later') {
    message.priority = priority;
  }
  return message;
}

// --------------------------------------------------------------------------
// Operations
// --------------------------------------------------------------------------

type Handler = (params: Record<string, unknown>) => Promise<unknown>;

const dir = (params: Record<string, unknown>): { dir?: string } =>
  typeof params.dir === 'string' ? { dir: params.dir } : {};

const handlers: Record<string, Handler> = {
  'host.ping': async () => ({ ok: true, pid: process.pid }),

  'session.create': (params) => createSession(params),

  'session.input': async (params) => {
    const session = requireSession(params.sessionId);
    const turnId = typeof params.turnId === 'string' ? params.turnId : null;

    // Ownership is settled BEFORE the message is pushed. Pushing first and
    // reporting the mismatch afterwards still delivered the input into whatever
    // turn was running — the caller got a 409 and Claude got their instructions
    // anyway, attributed to someone else's turn.
    if (turnId && session.turnId && session.turnId !== turnId) {
      throw new HostError(
        'busy',
        `Conversation ${session.id} is already running turn ${session.turnId}.`,
      );
    }
    if (turnId && !session.turnId) session.turnId = turnId;

    session.input.push(userMessage(params.content, params.priority as string | undefined));
    return { sessionId: session.id, turnId: session.turnId };
  },

  'session.interrupt': async (params) => {
    const session = requireSession(params.sessionId);
    const expected = typeof params.turnId === 'string' ? params.turnId : null;

    // Read and claim the turn BEFORE awaiting. interrupt() can take long enough
    // for the target turn to finish and a new one to start, and reading
    // session.turnId afterwards would then mark that innocent newer turn
    // interrupted while the caller's turn completed normally.
    const turnId = session.turnId;
    if (expected && turnId && expected !== turnId) {
      throw new HostError(
        'invalid_state',
        `Turn ${expected} is not the running turn (${turnId}).`,
      );
    }
    if (expected && !turnId) {
      return { sessionId: session.id, turnId: null, alreadyFinished: true };
    }
    session.turnId = null;

    await session.query.interrupt();
    if (turnId) {
      send({
        t: 'turn',
        sessionId: session.id,
        turnId,
        status: 'interrupted',
        result: null,
        error: null,
      });
    }
    return { sessionId: session.id, turnId };
  },

  'session.setModel': async (params) => {
    const session = requireSession(params.sessionId);
    const model = params.model === null ? undefined : (params.model as string | undefined);
    await session.query.setModel(model);
    session.options.model = model;
    return { sessionId: session.id, model: model ?? null };
  },

  'session.setPermissionMode': async (params) => {
    const session = requireSession(params.sessionId);
    const mode = String(params.mode) as PermissionMode;
    await session.query.setPermissionMode(mode);
    session.options.permissionMode = mode;
    return { sessionId: session.id, permissionMode: mode };
  },

  'session.models': async (params) => ({ models: await requireSession(params.sessionId).query.supportedModels() }),
  'session.commands': async (params) => ({ commands: await requireSession(params.sessionId).query.supportedCommands() }),
  'session.agents': async (params) => ({ agents: await requireSession(params.sessionId).query.supportedAgents() }),
  'session.mcpStatus': async (params) => ({ servers: await requireSession(params.sessionId).query.mcpServerStatus() }),
  'session.contextUsage': (params) => requireSession(params.sessionId).query.getContextUsage(),
  'session.accountInfo': (params) => requireSession(params.sessionId).query.accountInfo(),
  'session.initResult': (params) => requireSession(params.sessionId).query.initializationResult(),

  'session.readFile': async (params) => {
    const session = requireSession(params.sessionId);
    const encoding = params.encoding === 'base64' ? 'base64' : 'utf-8';
    const result = await session.query.readFile(String(params.path), {
      encoding,
      ...(typeof params.maxBytes === 'number' ? { maxBytes: params.maxBytes } : {}),
    });
    if (!result) throw new HostError('not_found', `Cannot read ${String(params.path)}.`);
    return result;
  },

  'session.stopTask': async (params) => {
    const session = requireSession(params.sessionId);
    await session.query.stopTask(String(params.taskId));
    return { ok: true };
  },

  'session.close': async (params) => {
    const session = sessions.get(String(params.sessionId));
    if (!session) return { ok: true, alreadyClosed: true };
    if (session.turnId) {
      send({
        t: 'turn',
        sessionId: session.id,
        turnId: session.turnId,
        status: 'interrupted',
        result: null,
        error: null,
      });
      session.turnId = null;
    }
    session.closed = true;
    session.input.close();
    try {
      session.query.close();
    } catch (err) {
      log('warn', `closing ${session.id}: ${errorShape(err).message}`);
    }
    sessions.delete(session.id);
    return { ok: true };
  },

  'session.list': async () => ({
    sessions: Array.from(sessions.values()).map((s) => ({
      sessionId: s.id,
      cwd: s.cwd,
      turnId: s.turnId,
      model: s.options.model ?? null,
      permissionMode: s.options.permissionMode ?? 'default',
    })),
  }),

  // --- offline: these read ~/.claude/projects without a subprocess ---------

  'store.list': async (params) =>
    ({
      sessions: await listSessions({
        ...dir(params),
        ...(typeof params.limit === 'number' ? { limit: params.limit } : {}),
        ...(typeof params.offset === 'number' ? { offset: params.offset } : {}),
        ...(typeof params.includeWorktrees === 'boolean'
          ? { includeWorktrees: params.includeWorktrees }
          : {}),
      }),
    }),

  'store.info': async (params) => {
    const info = await getSessionInfo(String(params.sessionId), dir(params));
    if (!info) throw new HostError('thread_not_found', `No conversation ${String(params.sessionId)}.`);
    return info;
  },

  'store.messages': async (params) => ({
    // Without this an unknown id answers 200 with an empty transcript, which
    // reads as "this conversation has no messages" rather than "no such thing".
    ...(await getSessionInfo(String(params.sessionId), dir(params)).then((info) => {
      if (!info) {
        throw new HostError('thread_not_found', `No conversation ${String(params.sessionId)}.`);
      }
      return {};
    })),
    messages: await getSessionMessages(String(params.sessionId), {
      ...dir(params),
      ...(typeof params.limit === 'number' ? { limit: params.limit } : {}),
      ...(typeof params.offset === 'number' ? { offset: params.offset } : {}),
      ...(typeof params.includeSystemMessages === 'boolean'
        ? { includeSystemMessages: params.includeSystemMessages }
        : {}),
    }),
  }),

  'store.subagents': async (params) => ({
    subagents: await listSubagents(String(params.sessionId), dir(params)),
  }),

  'store.subagentMessages': async (params) => ({
    messages: await getSubagentMessages(String(params.sessionId), String(params.agentId), dir(params)),
  }),

  'store.fork': (params) =>
    forkSession(String(params.sessionId), {
      ...dir(params),
      ...(typeof params.upToMessageId === 'string' ? { upToMessageId: params.upToMessageId } : {}),
      ...(typeof params.title === 'string' ? { title: params.title } : {}),
    }),

  'store.rename': async (params) => {
    await renameSession(String(params.sessionId), String(params.title), dir(params));
    return { ok: true };
  },

  'store.tag': async (params) => {
    await tagSession(
      String(params.sessionId),
      params.tag === null ? null : String(params.tag),
      dir(params),
    );
    return { ok: true };
  },

  'store.delete': async (params) => {
    await deleteSession(String(params.sessionId), dir(params));
    return { ok: true };
  },

  // --- human-in-the-loop --------------------------------------------------

  'request.respond': async (params) => {
    const requestId = String(params.requestId);
    const pending = pendingRequests.get(requestId);
    if (!pending) throw new HostError('request_not_found', `No pending request ${requestId}.`);
    pendingRequests.delete(requestId);
    pending.resolve(params.result);
    return { requestId, status: 'answered' };
  },
};

// --------------------------------------------------------------------------
// Command loop
// --------------------------------------------------------------------------

async function dispatch(line: string): Promise<void> {
  let command: Record<string, unknown>;
  try {
    command = JSON.parse(line) as Record<string, unknown>;
  } catch {
    log('error', 'ignored a malformed command line');
    return;
  }
  const id = typeof command.id === 'string' ? command.id : null;
  const op = String(command.op ?? '');
  const handler = handlers[op];
  if (!handler) {
    if (id) send({ t: 'reply', id, ok: false, error: { code: 'unknown_method', message: `Unknown op ${op}.` } });
    return;
  }
  try {
    const result = await handler(command);
    if (id) send({ t: 'reply', id, ok: true, result: result ?? null });
  } catch (err) {
    if (id) send({ t: 'reply', id, ok: false, error: errorShape(err) });
    else log('error', `${op} failed: ${errorShape(err).message}`);
  }
}

function main(): void {
  process.stdin.setEncoding('utf8');
  const rl = readline.createInterface({ input: process.stdin });
  rl.on('line', (line) => {
    if (line.trim()) void dispatch(line);
  });
  rl.on('close', () => {
    for (const session of sessions.values()) {
      session.input.close();
      try {
        session.query.close();
      } catch {
        // The process is going away regardless.
      }
    }
    process.exit(0);
  });

  // The SDK writes diagnostics to stderr; stdout is the protocol channel and
  // must carry nothing else.
  send({ t: 'ready', pid: process.pid });
}

main();
