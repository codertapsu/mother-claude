// Dev-only: conversations on disk for `?mock=1` (docs/ACCOUNTS.md §14) —
// every project folder (`GET /data/projects`), where a conversation's files
// are (the detail's `files`), revealing them (`POST /sessions/{id}/reveal`,
// `POST /data/projects/reveal`) and deleting conversations and projects in
// bulk (`POST /data/delete`): the dry run with what goes and what is
// skipped (a running conversation, one open in the API, one not there), one
// job per directory, and Undo. Never bundled in production builds.
//
// With `?mockaccounts=1` the projects are in two directories (the Default's
// and Work's). A device (`?as=device`) has no dangerous right here, so it
// may look but not delete, and never reveal.

import { DEFAULT_ACCOUNT, homeOf } from '../accounts';
import { AccountView, JobView, SessionSummary } from '../models';
import { MockRole } from './mock-api';
import { MockData } from './mock-data';

const SHARED_DIR = '/Users/marcus/.claude';

/** What the delete mock needs from the fixture backend. */
export interface DeleteHost {
  readonly role: MockRole;
  /** Every session the mock knows (live list and history). */
  sessions(): SessionSummary[];
  accounts(): AccountView[];
  /** The account a session is recorded as (the Default when none). */
  accountOf(sessionId: string): string;
  /** Take sessions out of the store (a delete); the answer puts them back. */
  removeSessions(ids: readonly string[]): () => void;
  /** Conversations open in the API's host. */
  openThreads(): string[];
}

interface Planned {
  s: SessionSummary;
  account: string;
  home: string;
  bytes: number;
}

interface Skip {
  id?: string;
  cwd?: string;
  title?: string;
  code: string;
  message: string;
}

export class MockDelete {
  constructor(
    private readonly host: DeleteHost,
    private readonly data: MockData,
  ) {}

  /** Answers a route of §14, or null to let others answer. */
  route(path: string, method: string, body: unknown): Response | null {
    const b = obj(body) ?? {};
    if (path === '/data/projects' && method === 'GET') return json({ projects: this.projects() });
    if (path === '/data/delete' && method === 'POST') return this.delete(b);
    if (path === '/data/projects/reveal' && method === 'POST') return this.revealProject(b);
    const m = /^\/sessions\/([^/]+)\/reveal$/.exec(path);
    if (m && method === 'POST') return this.revealSession(decodeURIComponent(m[1]));
    return null;
  }

  /** Where a session's files are (the detail's `files`). */
  filesOf(s: SessionSummary): Record<string, unknown> {
    const home = this.homeOf(this.host.accountOf(s.id));
    const dir = `${this.dirOf(home)}/projects/${encode(s.cwd)}`;
    return {
      transcript: `${dir}/${s.id}.jsonl`,
      folder: `${dir}/${s.id}`,
      fileHistory: `${this.dirOf(home)}/file-history/${s.id}`,
      account: home,
    };
  }

  private projects(): Record<string, unknown>[] {
    const rows = new Map<
      string,
      { account: string; cwd: string; sessions: number; live: number; bytes: number; last: number }
    >();
    for (const s of this.host.sessions()) {
      const account = this.host.accountOf(s.id);
      const key = `${account}\u0000${s.cwd}`;
      const row = rows.get(key) ?? { account, cwd: s.cwd, sessions: 0, live: 0, bytes: 0, last: 0 };
      row.sessions += 1;
      row.live += s.live ? 1 : 0;
      row.bytes += sizeOf(s);
      row.last = Math.max(row.last, s.lastActivityAt ?? 0);
      rows.set(key, row);
    }
    return [...rows.values()]
      .sort((a, b) => b.last - a.last)
      .map((r) => ({
        account: r.account,
        cwd: r.cwd,
        name: r.cwd.split('/').filter(Boolean).pop() ?? r.cwd,
        folder: `${this.dirOf(this.homeOf(r.account))}/projects/${encode(r.cwd)}`,
        sessions: r.sessions,
        live: r.live,
        bytes: r.bytes,
        lastActivityAt: r.last,
      }));
  }

  private delete(b: Record<string, unknown>): Response {
    // Deleting a transcript is a dangerous action: devices here lack it.
    if (this.host.role !== 'desktop') {
      return fail(
        403,
        'dangerous_blocked',
        'Deleting conversations is restricted to the Mother Claude desktop app, or to a device you granted dangerous actions in Remote > Devices.',
      );
    }
    const items = obj(b['items']);
    if (!items) return fail(422, 'invalid_body', 'missing field `items`');
    const ids = [...new Set(strings(items['sessions']))];
    const projects = [
      ...new Set(strings(items['projects']).map((p) => p.replace(/\/+$/, '') || '/')),
    ];
    if (!ids.length && !projects.length) {
      return fail(422, 'invalid_transfer', 'nothing to delete: name sessions or projects');
    }
    const fromRaw = b['from'];
    const from =
      typeof fromRaw === 'string' ? fromRaw : (str(obj(fromRaw)?.['account']) ?? undefined);
    if (fromRaw !== undefined && (!from || !this.account(from))) {
      return fail(404, 'not_found', `no account ${JSON.stringify(from ?? fromRaw)}`);
    }
    const known = this.host.sessions();
    const titleOf = (id?: string) => known.find((x) => x.id === id)?.title?.trim() || undefined;
    const open = new Set(this.host.openThreads());
    const skipped: Skip[] = [];
    const skip = (k: Omit<Skip, 'title'>) => {
      const title = titleOf(k.id);
      skipped.push({ ...k, ...(title ? { title } : {}) });
    };
    const candidates: SessionSummary[] = [];
    const named: string[] = [];
    const mine = (s: SessionSummary) => !from || this.host.accountOf(s.id) === from;
    const where = from ? `${this.account(from)?.name ?? from}'s directory` : '';
    for (const id of ids) {
      const s = known.find((x) => x.id === id && mine(x));
      if (s) candidates.push(s);
      // A conversation open in the API runs, indexed yet or not.
      else if (open.has(id)) skip({ id, code: 'session_live', message: running(id) });
      else
        skip({
          id,
          code: 'not_found',
          message: from ? `it is not in ${where}` : 'no account has this conversation',
        });
    }
    for (const cwd of projects) {
      const of = known.filter((s) => s.cwd === cwd && mine(s));
      if (!of.length) {
        skip({
          cwd,
          code: 'not_found',
          message: from
            ? `no conversations of ${cwd} here`
            : `no account has conversations of ${cwd}`,
        });
        continue;
      }
      named.push(cwd);
      for (const s of of) if (!candidates.includes(s)) candidates.push(s);
    }
    const planned: Planned[] = [];
    for (const s of candidates) {
      if (open.has(s.id) || s.live) {
        skip({ id: s.id, code: 'session_live', message: running(s.id) });
      } else {
        const account = this.host.accountOf(s.id);
        planned.push({ s, account, home: this.homeOf(account), bytes: sizeOf(s) });
      }
    }
    const homes = new Map<string, Planned[]>();
    for (const p of planned) homes.set(p.home, [...(homes.get(p.home) ?? []), p]);
    // A project folder goes when every conversation in it goes.
    const going = new Set(planned.map((p) => p.s.id));
    const folders = [...homes.entries()].flatMap(([home, group]) =>
      [...new Set(group.map((p) => p.s.cwd))]
        .filter((cwd) =>
          known
            .filter((s) => s.cwd === cwd && this.homeOf(this.host.accountOf(s.id)) === home)
            .every((s) => going.has(s.id)),
        )
        .map((cwd) => `${this.dirOf(home)}/projects/${encode(cwd)}`),
    );
    const goingProjects = named.filter((cwd) => planned.some((p) => p.s.cwd === cwd));
    if (b['dryRun'] === true) {
      return json({
        plan: {
          ...(from ? { from } : {}),
          undo: b['undo'] !== false,
          jobs: Math.max(1, homes.size),
          sessions: planned.map((p) => ({
            id: p.s.id,
            ...(p.s.title ? { title: p.s.title } : {}),
            cwd: p.s.cwd,
            account: p.account,
            files: 3,
            bytes: p.bytes,
          })),
          skipped,
          projects: goingProjects,
          folders,
          sessionsTotal: planned.length,
          skippedTotal: skipped.length,
          projectsTotal: goingProjects.length,
          foldersTotal: folders.length,
          files: planned.length * 3,
          bytes: planned.reduce((n, p) => n + p.bytes, 0),
        },
      });
    }
    const keep = b['undo'] !== false;
    // Nothing to delete still makes one job, which lists what it skipped.
    const groups = homes.size ? [...homes.entries()] : [[from ?? DEFAULT_ACCOUNT, []] as const];
    const jobs: JobView[] = groups.map(([home, group], i) => {
      const ids = group.map((p) => p.s.id);
      const bytes = group.reduce((n, p) => n + p.bytes, 0);
      return this.data.runJob(
        'delete',
        // Named after the account asked for, else the directory's.
        { account: from ?? home },
        ids.length * 3,
        bytes,
        ids,
        1_400,
        () => {
          const putBack = ids.length ? this.host.removeSessions(ids) : () => undefined;
          return {
            result: { deleted: ids, skipped: i === 0 ? skipped : [] },
            sessions: ids,
            undo: keep && ids.length > 0,
            onUndo: putBack,
          };
        },
      );
    });
    return json({ jobs, job: jobs[0] }, 202);
  }

  private revealSession(id: string): Response {
    if (this.host.role !== 'desktop') {
      return fail(
        403,
        'desktop_only',
        'This action is only available on the Mother Claude desktop app.',
      );
    }
    if (!this.host.sessions().some((s) => s.id === id)) {
      return fail(404, 'session_not_found', `no session ${id}`);
    }
    return new Response(null, { status: 204 });
  }

  private revealProject(b: Record<string, unknown>): Response {
    if (this.host.role !== 'desktop') {
      return fail(
        403,
        'desktop_only',
        'This action is only available on the Mother Claude desktop app.',
      );
    }
    // A row's folder, which may end in a separator.
    const cwd = str(b['cwd'])?.replace(/\/+$/, '') || undefined;
    const account = str(b['account']);
    if (!cwd || !account) return fail(422, 'invalid_body', 'Send account and cwd.');
    const any = this.host
      .sessions()
      .some((s) => s.cwd === cwd && this.host.accountOf(s.id) === account);
    return any
      ? new Response(null, { status: 204 })
      : fail(404, 'project_not_found', 'That account has no conversations of that folder.');
  }

  private account(id: string): AccountView | undefined {
    return this.host.accounts().find((a) => a.id === id);
  }

  /** The account whose directory holds an account's conversations. */
  private homeOf(id: string): string {
    const a = this.account(id);
    return a ? homeOf(a) : DEFAULT_ACCOUNT;
  }

  private dirOf(home: string): string {
    const a = this.account(home);
    return a?.kind === 'directory' && a.dir ? a.dir : SHARED_DIR;
  }
}

/** The server's words for a running conversation (one open in the API's
 * host too). */
function running(id: string): string {
  return `session ${id} is running; stop it first`;
}

/** A conversation's size in the demo. */
function sizeOf(s: SessionSummary): number {
  return 40_000 + s.messageCount * 2_600;
}

/** `/Users/me/dev/api` → `-Users-me-dev-api`. */
function encode(cwd: string): string {
  return cwd.replace(/[^a-zA-Z0-9]/g, '-');
}

function obj(v: unknown): Record<string, unknown> | null {
  return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
}

function str(v: unknown): string | undefined {
  return typeof v === 'string' && v.trim() ? v : undefined;
}

function strings(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string' && !!x) : [];
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function fail(status: number, code: string, message: string): Response {
  return json({ error: { code, message } }, status);
}
