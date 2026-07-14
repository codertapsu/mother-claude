/**
 * Mother Claude — Path A control bridge (optional).
 *
 * Drives one OWNED session via the Claude Agent SDK so that every
 * non-auto-approved tool call is gated by `canUseTool` and every question is
 * surfaced through a custom in-process `ask_user` MCP tool. Both round-trip to
 * the Mother Claude server's `/api/sessions/:id/permission-request` endpoint,
 * which blocks until a human answers from the dashboard (desktop or phone).
 *
 * This is the documented workaround for: the native AskUserQuestion TUI failing
 * without a TTY under the SDK (we set `disallowedTools: ["AskUserQuestion"]`),
 * and for remote permission approval of owned sessions.
 *
 * Built automatically by `npm run tauri:dev` / `tauri:build` (and bundled into
 * the packaged app), and used by the Rust core by default whenever present.
 * Disable with MOTHER_CLAUDE_SIDECAR=0 to force the headless path. Runtime
 * correctness depends on the installed SDK version.
 *
 * Env in: MOTHER_CLAUDE_URL, MOTHER_CLAUDE_TOKEN, MC_SESSION_ID, MC_CWD,
 * MC_PROMPT, MC_MODEL (optional), MC_PERMISSION_MODE (optional).
 */
import { query, createSdkMcpServer, tool } from '@anthropic-ai/claude-agent-sdk';
import { z } from 'zod';
import * as readline from 'node:readline';

const URL = process.env.MOTHER_CLAUDE_URL ?? 'http://127.0.0.1:6725';
const TOKEN = process.env.MOTHER_CLAUDE_TOKEN ?? '';
const SESSION_ID = process.env.MC_SESSION_ID ?? '';
const CWD = process.env.MC_CWD ?? process.cwd();
const PROMPT = process.env.MC_PROMPT ?? '';
const MODEL = process.env.MC_MODEL || undefined;
const PERMISSION_MODE = process.env.MC_PERMISSION_MODE || 'default';
/** Reasoning effort: low | medium | high | xhigh | max (unset = user default). */
const EFFORT = process.env.MC_EFFORT || undefined;
/** Thinking override: 'on' | 'off' (unset = model/settings default). */
const THINKING = process.env.MC_THINKING || undefined;

interface Resolution {
  behavior: 'allow' | 'deny' | 'answer';
  answer?: string;
}

type OptionInput = string | { label: string; description?: string };

/** Block on the dashboard for a decision/answer. */
async function ask(
  kind: 'permission' | 'question',
  payload: {
    tool?: string;
    prompt?: string;
    header?: string;
    options?: OptionInput[];
    multiSelect?: boolean;
    detail?: string;
    dangerous?: boolean;
  },
): Promise<Resolution> {
  const res = await fetch(`${URL}/api/sessions/${encodeURIComponent(SESSION_ID)}/permission-request`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', authorization: `Bearer ${TOKEN}` },
    body: JSON.stringify({ kind, ...payload }),
  });
  if (!res.ok) return { behavior: 'deny' };
  return (await res.json()) as Resolution;
}

const isDangerous = (toolName: string, input: unknown): boolean => {
  const text = `${toolName} ${JSON.stringify(input)}`.toLowerCase();
  return (
    text.includes('bypasspermissions') ||
    text.includes('dangerously-skip-permissions') ||
    text.includes('rm -rf')
  );
};

/** The argument of `input` that tells a human what the tool will actually do
 * (the Bash command, the file path, …) — shown on the permission card. */
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
  const rec = input as Record<string, unknown>;
  const keys = SALIENT_ARGS[toolName];
  const parts = keys
    ?.map((k) => rec[k])
    .filter((v) => v != null)
    .map((v) => (typeof v === 'string' ? v : JSON.stringify(v)));
  const text = parts?.length ? parts.join(' · ') : JSON.stringify(rec);
  return text.length > 700 ? `${text.slice(0, 700)}…` : text;
}

const optionSchema = z.union([
  z.string(),
  z.object({ label: z.string(), description: z.string().optional() }),
]);

/** Custom ask_user tool so questions surface remotely instead of via the TUI.
 * Mirrors the native AskUserQuestion shape so the dashboard can render option
 * cards with descriptions and multi-select. */
const askUserServer = createSdkMcpServer({
  name: 'mother-claude',
  version: '0.1.0',
  tools: [
    tool(
      'ask_user',
      'Ask the human operator a question and wait for their answer. When the question has ' +
        'natural choices, give 2-4 options, each with a short label and a one-line description ' +
        'of what it means or implies; set multiSelect true when several options can apply ' +
        'together. The user can always type a free-text answer instead of picking an option.',
      {
        question: z.string(),
        header: z
          .string()
          .optional()
          .describe('Very short topic chip (max ~12 chars), e.g. "Auth method"'),
        options: z.array(optionSchema).optional(),
        multiSelect: z.boolean().optional(),
      },
      async (args: {
        question: string;
        header?: string;
        options?: OptionInput[];
        multiSelect?: boolean;
      }) => {
        const r = await ask('question', {
          prompt: args.question,
          header: args.header,
          options: args.options,
          multiSelect: args.multiSelect,
        });
        return { content: [{ type: 'text', text: r.answer ?? '' }] };
      },
    ),
  ],
});

// A streaming-input user message. `parent_tool_use_id` is required by the SDK's
// SDKUserMessage; it is null for top-level (non-subagent) turns.
interface UserTurn {
  type: 'user';
  message: { role: 'user'; content: string };
  parent_tool_use_id: string | null;
}

const userTurn = (content: string): UserTurn => ({
  type: 'user',
  message: { role: 'user', content },
  parent_tool_use_id: null,
});

/** Async generator: initial prompt, then follow-up user messages from stdin. */
async function* prompts(): AsyncGenerator<UserTurn> {
  if (PROMPT.trim()) {
    yield userTurn(PROMPT);
  }
  const rl = readline.createInterface({ input: process.stdin });
  for await (const line of rl) {
    if (!line.trim()) continue;
    try {
      const parsed = JSON.parse(line);
      const content =
        typeof parsed?.message?.content === 'string'
          ? parsed.message.content
          : Array.isArray(parsed?.message?.content)
            ? parsed.message.content.map((b: { text?: string }) => b.text ?? '').join('')
            : line;
      yield userTurn(content);
    } catch {
      yield userTurn(line);
    }
  }
}

async function main(): Promise<void> {
  const response = query({
    prompt: prompts(),
    options: {
      cwd: CWD,
      model: MODEL,
      effort: EFFORT as 'low' | 'medium' | 'high' | 'xhigh' | 'max' | undefined,
      thinking:
        THINKING === 'off'
          ? { type: 'disabled' as const }
          : THINKING === 'on'
            ? { type: 'adaptive' as const }
            : undefined,
      resume: SESSION_ID,
      permissionMode: PERMISSION_MODE as 'default',
      mcpServers: { 'mother-claude': askUserServer },
      // Force questions through ask_user instead of the (TTY-only) native tool.
      disallowedTools: ['AskUserQuestion'],
      // Gate every non-auto-approved tool through the dashboard.
      canUseTool: async (toolName: string, input: unknown) => {
        const r = await ask('permission', {
          tool: toolName,
          prompt: `Claude wants to use ${toolName}.`,
          detail: describeInput(toolName, input),
          dangerous: isDangerous(toolName, input),
        });
        return r.behavior === 'allow'
          ? { behavior: 'allow' as const, updatedInput: input as Record<string, unknown> }
          : { behavior: 'deny' as const, message: 'Denied from Mother Claude dashboard' };
      },
    },
  });

  for await (const message of response) {
    // Emit each SDK message as NDJSON; the Rust core reads stdout for completion.
    process.stdout.write(JSON.stringify(message) + '\n');
  }
}

main().catch((err) => {
  process.stderr.write(`sidecar error: ${err?.stack ?? err}\n`);
  process.exit(1);
});
