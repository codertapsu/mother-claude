/**
 * The Claude API on the HTTP API page (docs/CLAUDE-API.md): its keys, and
 * the clients and conversations each API server keeps.
 */
import { tr } from './i18n/i18n';

/** Where an API server can be reached from (docs/CLAUDE-API.md "Access"). */
export type Reach = 'local' | 'lan' | 'all';

/** The reach choices, as the page names them. */
export const REACH_CHOICES: readonly { value: Reach; label: string; hint: string }[] = [
  { value: 'local', label: 'This computer', hint: 'programs on this {computer} only' },
  {
    value: 'lan',
    label: 'Local network',
    hint: "other computers on your network too, at this {computer}'s local addresses",
  },
  {
    value: 'all',
    label: 'Anywhere',
    hint: 'every address of this {computer}, public ones too, and tunnels such as ngrok',
  },
];

/** A strong key: `mck-` and 40 random letters and digits. */
export function newKey(): string {
  const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
  const bytes = new Uint8Array(40);
  crypto.getRandomValues(bytes);
  return 'mck-' + Array.from(bytes, (b) => chars[b % chars.length]).join('');
}

/** Tokens a client's conversations used. */
export interface ApiUsage {
  inputTokens: number;
  outputTokens: number;
  cacheReadInputTokens: number;
  cacheCreationInputTokens: number;
}

/** One client of an API server's folder. */
export interface ApiClientView {
  /** Its folder's name. */
  id: string;
  kind: 'default' | 'ip' | 'key' | 'user' | 'header' | string;
  label: string;
  firstSeen: number;
  lastSeen: number;
  requests: number;
  folder: string;
  conversations?: number;
  /** Conversations answering, or waiting for the client's tool results. */
  active?: number;
  usage?: ApiUsage;
}

/** A folder clients are kept in, and the servers that use it. */
export interface ApiClientRoot {
  path: string;
  default: boolean;
  servers: string[];
  clients: ApiClientView[];
}

/** One conversation of a client. */
export interface ApiConversationRow {
  id: string;
  title?: string | null;
  model: string;
  createdAt: number;
  updatedAt: number;
  /** Messages kept: the client's and the answers. */
  messages: number;
  requests: number;
  usage: ApiUsage;
  seeded: boolean;
  forkedFrom?: string | null;
  server: string;
  account: string;
  busy: boolean;
  waitingForTools: boolean;
  /** Its Claude Code process is open. */
  live: boolean;
}

/** One message as a conversation keeps it (images' bytes left out). */
export interface ApiMessage {
  role: 'user' | 'assistant';
  content: Record<string, unknown>[];
}

export interface ApiConversationFull extends ApiConversationRow {
  client: string;
  messageList: ApiMessage[];
}

/** What a delete did. */
export interface ApiDeleteResult {
  deleted: string[];
  /** Answering a request: left as they are. */
  busy?: string[];
  /** The transcripts' delete jobs. */
  jobs?: unknown[];
}

/** How a server tells clients apart, as the page names it. */
export const GROUP_CHOICES: readonly { value: string; label: string; hint: string }[] = [
  {
    value: '',
    label: 'Not at all',
    hint: 'Every client shares one folder.',
  },
  {
    value: 'ip',
    label: 'IP address',
    hint: 'A folder per address the requests come from.',
  },
  {
    value: 'key',
    label: 'API key',
    hint: 'A folder per key (any key a client sends: on a server without one it is only a name).',
  },
  {
    value: 'user',
    label: 'metadata.user_id',
    hint: 'A folder per user the request names; else by IP address.',
  },
  {
    value: 'header',
    label: 'X-Client-Id header',
    hint: 'A folder per X-Client-Id the client sends; else by IP address.',
  },
];

/** A client's kind in words. */
export function clientKind(kind: string): string {
  switch (kind) {
    case 'ip':
      return tr('IP address');
    case 'key':
      return tr('API key');
    case 'user':
      return tr('User');
    case 'header':
      return tr('Client id');
    default:
      return tr('All clients');
  }
}

/** A conversation's state in words. */
export function conversationState(c: ApiConversationRow): string {
  if (c.busy) return tr('answering');
  if (c.waitingForTools) return tr('waiting for tool results');
  return c.live ? tr('open') : tr('idle');
}

/** "12.3k" tokens. */
export function tokens(n: number | undefined): string {
  const v = n ?? 0;
  if (v < 1000) return String(v);
  if (v < 1_000_000) return `${(v / 1000).toFixed(v < 10_000 ? 1 : 0)}k`;
  return `${(v / 1_000_000).toFixed(1)}M`;
}

/** The text of a kept message, for a preview. */
export function messageText(m: ApiMessage): string {
  return m.content
    .map((b) => {
      switch (b['type']) {
        case 'text':
          return String(b['text'] ?? '');
        case 'tool_use':
          return `[${tr('tool')} ${String(b['name'] ?? '')}]`;
        case 'tool_result':
          return `[${tr('tool result')}]`;
        case 'image':
          return `[${tr('image')}]`;
        case 'document':
          return `[${tr('document')}]`;
        default:
          return '';
      }
    })
    .filter((t) => t)
    .join(' ');
}
