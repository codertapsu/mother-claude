// A session asking to read, message, ask or start another session through
// Mother Claude's `session_*` tools (docs/MCP.md "Sessions working
// together"): the approval card's input, read tolerantly, and its words.

import { tr } from './i18n/i18n';

export type SessionTalkAction = 'read' | 'send' | 'ask' | 'start';

/** What the card shows (`PendingItem.input` of a `session` card). */
export interface SessionTalkInput {
  action: SessionTalkAction;
  /** The session it reads, messages or asks (absent for a start). */
  session: string;
  sessionName: string;
  /** Its project, or the project a start runs in. */
  sessionProject: string;
  /** Its folder, or the folder a start runs in. */
  folder: string;
  /** The computer: this one or a paired one, by name. */
  computer: string;
  /** On this computer. */
  here: boolean;
  /** Running now (absent for a start). */
  live: boolean | null;
  status: string;
  permissionMode: string;
  /** The message, question or first prompt. */
  message: string;
  /** A read: how many messages. */
  last: number | null;
  /** The asking session's name. */
  from: string;
}

const ACTIONS: readonly SessionTalkAction[] = ['read', 'send', 'ask', 'start'];
const str = (v: unknown): string => (typeof v === 'string' ? v : '');

/** A session card's input (null: not one). */
export function sessionTalkInput(input: unknown): SessionTalkInput | null {
  if (!input || typeof input !== 'object') return null;
  const o = input as Record<string, unknown>;
  const action = ACTIONS.find((a) => a === o['action']);
  if (!action) return null;
  return {
    action,
    session: str(o['session']),
    sessionName: str(o['sessionName']) || str(o['session']).slice(0, 8),
    sessionProject: str(o['sessionProject']),
    folder: str(o['folder']),
    computer: str(o['computer']),
    here: o['here'] !== false,
    live: typeof o['live'] === 'boolean' ? o['live'] : null,
    status: str(o['status']),
    permissionMode: str(o['permissionMode']),
    message: str(o['message']),
    last: typeof o['last'] === 'number' ? o['last'] : null,
    from: str(o['from']),
  };
}

/** The card's heading: "Message the session “docs”?", "… on DESKTOP-PC?". */
export function sessionTalkHeading(v: SessionTalkInput): string {
  const name = v.sessionName;
  const computer = v.computer;
  switch (v.action) {
    case 'read':
      return v.here
        ? tr('Read the session “{name}”?', { name })
        : tr('Read the session “{name}” on {computer}?', { name, computer });
    case 'send':
      return v.here
        ? tr('Message the session “{name}”?', { name })
        : tr('Message the session “{name}” on {computer}?', { name, computer });
    case 'ask':
      return v.here
        ? tr('Ask the session “{name}”?', { name })
        : tr('Ask the session “{name}” on {computer}?', { name, computer });
    default: {
      const project = v.sessionProject || tr('a project');
      return v.here
        ? tr('Start a session in {project}?', { project })
        : tr('Start a session in {project} on {computer}?', { project, computer });
    }
  }
}

/** The server's title of a session card (`mcp::sessions::approve::title`)
 * in the language shown; `null` when it is not one. */
export function sessionTalkTitle(title: string): string | null {
  const m = /^(Read|Message|Ask) the session “(.*)”(?: on (.+))?$/s.exec(title);
  if (m) {
    return sessionTalkHeading({
      ...EMPTY,
      action: m[1] === 'Read' ? 'read' : m[1] === 'Message' ? 'send' : 'ask',
      sessionName: m[2],
      computer: m[3] ?? '',
      here: !m[3],
    });
  }
  const s = /^Start a session in (.+?)(?: on (.+))?$/s.exec(title);
  if (s) {
    return sessionTalkHeading({
      ...EMPTY,
      action: 'start',
      sessionProject: s[1],
      computer: s[2] ?? '',
      here: !s[2],
    });
  }
  return null;
}

const EMPTY: SessionTalkInput = {
  action: 'read',
  session: '',
  sessionName: '',
  sessionProject: '',
  folder: '',
  computer: '',
  here: true,
  live: null,
  status: '',
  permissionMode: '',
  message: '',
  last: null,
  from: '',
};

/** The server's detail of a session card in the language shown; any other
 * text as it came. */
export function sessionTalkDetail(detail: string): string {
  const read = /^“(.*)” wants to read its last (\d+) messages\.$/s.exec(detail);
  if (read)
    return tr('“{from}” wants to read its last {n} messages.', { from: read[1], n: +read[2] });
  const send = /^“(.*)” sends: (.*)$/s.exec(detail);
  if (send) return tr('“{from}” sends: {text}', { from: send[1], text: send[2] });
  const ask = /^“(.*)” asks, and waits for the answer: (.*)$/s.exec(detail);
  if (ask)
    return tr('“{from}” asks, and waits for the answer: {text}', { from: ask[1], text: ask[2] });
  const start = /^“(.*)” starts it in (.+?) with: (.*)$/s.exec(detail);
  if (start) {
    return tr('“{from}” starts it in {folder} with: {text}', {
      from: start[1],
      folder: start[2],
      text: start[3],
    });
  }
  return detail;
}

/** Whether a permission mode runs without prompts (what it is told runs
 * with nobody approving it). */
export function runsUnapproved(mode: string): boolean {
  return mode === 'bypassPermissions' || mode === 'dontAsk';
}
