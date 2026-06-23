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

interface Resolution {
  behavior: 'allow' | 'deny' | 'answer';
  answer?: string;
}

/** Block on the dashboard for a decision/answer. */
async function ask(
  kind: 'permission' | 'question',
  payload: { tool?: string; prompt?: string; options?: string[]; dangerous?: boolean },
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

/** Custom ask_user tool so questions surface remotely instead of via the TUI. */
const askUserServer = createSdkMcpServer({
  name: 'mother-claude',
  version: '0.1.0',
  tools: [
    tool(
      'ask_user',
      'Ask the human operator a question and wait for their answer.',
      { question: z.string(), options: z.array(z.string()).optional() },
      async (args: { question: string; options?: string[] }) => {
        const r = await ask('question', { prompt: args.question, options: args.options });
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
      resume: SESSION_ID,
      permissionMode: PERMISSION_MODE as 'default',
      mcpServers: { 'mother-claude': askUserServer },
      // Force questions through ask_user instead of the (TTY-only) native tool.
      disallowedTools: ['AskUserQuestion'],
      // Gate every non-auto-approved tool through the dashboard.
      canUseTool: async (toolName: string, input: unknown) => {
        const r = await ask('permission', {
          tool: toolName,
          prompt: `Allow ${toolName}?`,
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
