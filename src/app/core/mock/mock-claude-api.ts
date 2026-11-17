// Dev-only: the Claude API's side of the HTTP API page for `?mock=1`
// (docs/CLAUDE-API.md): the clients and conversations each server keeps
// (`/api/bridge/clients*`, `/api/bridge/api-conversations*`) and *Start when
// Mother Claude opens* (`/api/bridge/autostart`). Mirrors the server:
// deleting transcripts needs the dangerous right; a conversation answering a
// request is left. Never bundled in production builds.

import type { ApiClientRoot, ApiConversationFull, ApiConversationRow } from '../claude-api';

const MIN = 60 * 1000;
const ROOT = '/Users/marcus/Library/Application Support/dev.motherclaude.desktop/claude-api';

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function err(status: number, code: string, message: string): Response {
  return json({ error: { code, message } }, status);
}

const usage = (i: number, o: number) => ({
  inputTokens: i,
  outputTokens: o,
  cacheReadInputTokens: 0,
  cacheCreationInputTokens: 0,
});

interface Conv extends ApiConversationRow {
  client: string;
  messageList: ApiConversationFull['messageList'];
}

export class MockClaudeApi {
  private convs: Conv[];
  private clients: ApiClientRoot['clients'];

  constructor(
    private readonly desktop: boolean,
    now: number,
    /** The main status, for `autostart`. */
    private readonly autostart: (on: boolean) => unknown,
  ) {
    const conv = (
      id: string,
      client: string,
      title: string,
      ago: number,
      over: Partial<Conv> = {},
    ): Conv => ({
      id,
      client,
      title,
      model: 'claude-sonnet-5-5',
      createdAt: now - ago - 30 * MIN,
      updatedAt: now - ago,
      messages: 4,
      requests: 2,
      usage: usage(1840, 412),
      seeded: false,
      server: 'main',
      account: 'default',
      busy: false,
      waitingForTools: false,
      live: ago < 10 * MIN,
      messageList: [
        { role: 'user', content: [{ type: 'text', text: title }] },
        { role: 'assistant', content: [{ type: 'text', text: 'Here is what I found…' }] },
        { role: 'user', content: [{ type: 'text', text: 'Shorter, please.' }] },
        { role: 'assistant', content: [{ type: 'text', text: 'In short: yes.' }] },
      ],
      ...over,
    });
    this.convs = [
      conv(
        '5f2c1a90-3b7e-4d21-9c4a-7e1b2d3c4f50',
        '127.0.0.1',
        'Summarise the release notes',
        4 * MIN,
      ),
      conv(
        '8a4e2b11-6c9d-4f03-8e7a-2b5c9d1e3f62',
        '127.0.0.1',
        'Translate the README',
        3 * 60 * MIN,
      ),
      conv(
        'c3d9e7f2-1a4b-4c8d-a2e6-9f0b3c5d7e84',
        '192.168.1.42',
        'What is the weather in Paris?',
        1 * MIN,
        { busy: true, live: true, messages: 2, requests: 1 },
      ),
    ];
    this.clients = [
      this.client('127.0.0.1', now - 4 * MIN, 6),
      this.client('192.168.1.42', now - MIN, 1),
    ];
  }

  private client(id: string, lastSeen: number, requests: number) {
    return {
      id,
      kind: 'ip',
      label: id,
      firstSeen: lastSeen - 2 * 24 * 60 * MIN,
      lastSeen,
      requests,
      folder: `${ROOT}/${id}`,
    };
  }

  private roots(): ApiClientRoot[] {
    const clients = this.clients.map((c) => {
      const mine = this.convs.filter((v) => v.client === c.id);
      return {
        ...c,
        conversations: mine.length,
        active: mine.filter((v) => v.busy || v.waitingForTools).length,
        usage: usage(
          mine.reduce((n, v) => n + v.usage.inputTokens, 0),
          mine.reduce((n, v) => n + v.usage.outputTokens, 0),
        ),
      };
    });
    return [{ path: ROOT, default: true, servers: [], clients }];
  }

  private row(v: Conv): ApiConversationRow {
    const row: Partial<Conv> = { ...v };
    delete row.client;
    delete row.messageList;
    return row as ApiConversationRow;
  }

  /** The answer for a path of these routes, or null. */
  route(path: string, method: string, body: unknown, q: URLSearchParams): Response | null {
    if (path === '/bridge/autostart' && method === 'POST') {
      const on = (body as { on?: boolean }).on === true;
      if (on && !this.desktop) {
        return err(
          403,
          'desktop_only',
          'Starting an API server whenever Mother Claude opens can only be chosen on the Mother Claude desktop app.',
        );
      }
      return json(this.autostart(on));
    }
    if (path === '/bridge/clients') return json({ roots: this.roots() });
    const client = path.match(/^\/bridge\/clients\/([^/]+)$/);
    if (client) {
      const id = decodeURIComponent(client[1]);
      const c = this.roots()[0].clients.find((x) => x.id === id);
      if (!c) return err(404, 'client_not_found', `No client ${id}.`);
      const mine = this.convs.filter((v) => v.client === id);
      if (method === 'DELETE') {
        if (q.get('transcripts') === 'true' && !this.desktop) {
          return err(403, 'dangerous_blocked', "Deleting a client's transcripts is restricted.");
        }
        if (mine.some((v) => v.busy)) {
          return err(409, 'client_busy', 'A conversation of this client is answering a request.');
        }
        this.convs = this.convs.filter((v) => v.client !== id);
        this.clients = this.clients.filter((x) => x.id !== id);
        return json({ deleted: mine.map((v) => v.id), jobs: [] });
      }
      return json({ client: c, conversations: mine.map((v) => this.row(v)) });
    }
    if (path === '/bridge/api-conversations/delete' && method === 'POST') {
      const b = body as { ids?: string[]; transcripts?: boolean };
      if (b.transcripts && !this.desktop) {
        return err(403, 'dangerous_blocked', "Deleting conversations' transcripts is restricted.");
      }
      const asked = new Set(b.ids ?? []);
      const busy = this.convs.filter((v) => asked.has(v.id) && v.busy).map((v) => v.id);
      const deleted = this.convs.filter((v) => asked.has(v.id) && !v.busy).map((v) => v.id);
      this.convs = this.convs.filter((v) => !deleted.includes(v.id));
      return json({ deleted, busy, jobs: [] });
    }
    const conv = path.match(/^\/bridge\/api-conversations\/([^/]+)$/);
    if (conv && method === 'GET') {
      const v = this.convs.find((x) => x.id === decodeURIComponent(conv[1]));
      return v ? json({ ...v }) : err(404, 'conversation_not_found', 'No such conversation.');
    }
    return null;
  }
}
