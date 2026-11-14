// Dev-only: permissions — what Claude may do without asking you — for
// `?mock=1`: `GET/PATCH /api/permissions`, `PATCH/DELETE
// /api/permissions/{id}`, `remember` on an answer, and the requests a
// permission allows without asking. Mirrors the server's rules: a project
// or the whole computer needs the dangerous right (here: the desktop), and
// a pattern that can't be kept is a `422 invalid_pattern`. Three are there
// to start with, and with `?mockmachines=1` the standing grant of runs on
// DESKTOP-PC, as it became one. Never bundled in production builds.

import { GrantOffer, GrantScope, PatternKind, PendingItem, ServerFrame } from '../models';
import {
  MAX_PATTERNS,
  PermissionRule,
  PermissionsView,
  SCOPES,
  cleanPatterns,
  wider,
} from '../permission-rules';
import { MOCK_DESKTOP } from './mock-moves';

const S = 1000;
const M = 60 * S;
const H = 60 * M;
const D = 24 * H;

const API_SERVER = '/Users/marcus/dev/api-server';

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function err(status: number, code: string, message: string): Response {
  return json({ error: { code, message } }, status);
}

const invalid = (message: string) => err(422, 'invalid_pattern', message);

/** What a command pattern can't hold: a command with one always asks. */
const CHAINS = /[;&|<>$`(){}\n]/;
/** A shell or an interpreter alone allows anything. */
const ALONE = /^(bash|sh|zsh|fish|pwsh|powershell|cmd|python3?|node|ruby|perl|env|sudo)$/i;

/** Why `p` can't be a pattern of this kind ('' when it can). */
function patternProblem(kind: PatternKind, p: string): string {
  if (p.length > 200) return 'A pattern is 200 characters at most.';
  switch (kind) {
    case 'command':
      if (CHAINS.test(p)) {
        return `“${p}” can’t hold ; & | < > $ ( ) { } or a backtick: a command with one always asks.`;
      }
      return ALONE.test(p)
        ? `Name the program and what it runs, such as “npm run build”, not “${p}” alone.`
        : '';
    case 'path':
      return /^([a-zA-Z]:[\\/]|[\\/]|~)/.test(p) || p.split(/[\\/]/).includes('..')
        ? `“${p}” must stay inside the project, such as src/** or *.md.`
        : '';
    case 'domain':
      return /^(\*\.)?([a-z0-9-]+\.)*[a-z0-9-]+$/i.test(p)
        ? ''
        : `“${p}” isn’t a site: use one such as docs.rs or *.github.com.`;
  }
}

/** A glob as a pattern of the path relative to the project: `**` any
 * folders, `*` and `?` within one. */
function glob(p: string): RegExp {
  const part = (s: string) =>
    s
      .replace(/[.+^${}()|[\]\\]/g, '\\$&')
      .replace(/\*/g, '[^/]*')
      .replace(/\?/g, '[^/]');
  return new RegExp(`^${p.split('**').map(part).join('.*')}$`);
}

/** What a request's patterns are matched against: its command, its file
 * (relative to the project) or its site ('' when none: it asks). */
function subjectOf(item: PendingItem, kind: PatternKind | undefined, project?: string): string {
  const input = (item.input ?? {}) as Record<string, unknown>;
  switch (kind) {
    case 'command': {
      const cmd = typeof input['command'] === 'string' ? input['command'].trim() : '';
      // One that chains or redirects always asks.
      return CHAINS.test(cmd) ? '' : cmd;
    }
    case 'path': {
      const p = String(input['file_path'] ?? input['notebook_path'] ?? input['path'] ?? '');
      return project && p.startsWith(`${project}/`) ? p.slice(project.length + 1) : p;
    }
    case 'domain':
      try {
        return new URL(String(input['url'] ?? '')).hostname;
      } catch {
        return '';
      }
    default:
      return '';
  }
}

function matches(kind: PatternKind | undefined, p: string, subject: string): boolean {
  switch (kind) {
    case 'command':
      return subject === p || subject.startsWith(`${p} `);
    case 'path':
      return glob(p).test(p.includes('/') ? subject : (subject.split('/').pop() ?? subject));
    case 'domain':
      return p.startsWith('*.') ? subject.endsWith(p.slice(1)) : subject === p;
    default:
      return false;
  }
}

/** The rule allows the request without asking. */
function covers(r: PermissionRule, g: GrantOffer, item: PendingItem): boolean {
  if (r.kind !== g.kind) return false;
  if (r.kind === 'tool' ? r.tool !== g.tool : r.machine !== g.machine) return false;
  if (r.scope === 'session' && r.session !== g.session) return false;
  if (r.scope === 'project' && r.project !== g.project) return false;
  if (!r.patterns.length) return true;
  const subject = subjectOf(item, r.patternKind, g.project);
  return !!subject && r.patterns.some((p) => matches(r.patternKind, p, subject));
}

/** `after` allows something `before` did not. */
function coversMore(kind: PatternKind | undefined, before: string[], after: string[]): boolean {
  if (!after.length) return before.length > 0;
  if (!before.length) return false;
  return after.some(
    (p) => !before.some((q) => p === q || (kind === 'command' && p.startsWith(`${q} `))),
  );
}

/** The same permission as the offer would make, in this scope. */
function sameTarget(r: PermissionRule, g: GrantOffer, scope: GrantScope): boolean {
  return (
    r.kind === g.kind &&
    r.tool === g.tool &&
    r.machine === g.machine &&
    r.scope === scope &&
    (scope !== 'session' || r.session === g.session) &&
    (scope !== 'project' || r.project === g.project)
  );
}

function seed(now: number, machines: boolean): PermissionRule[] {
  const rules: PermissionRule[] = [
    {
      id: 'pr_0000000000a1',
      kind: 'tool',
      tool: 'Bash',
      patterns: ['npm test', 'npm run lint'],
      patternKind: 'command',
      scope: 'project',
      project: API_SERVER,
      projectName: 'api-server',
      createdAt: now - 3 * D,
      createdBy: 'Mac',
      usedAt: now - 43 * M,
      uses: 9,
    },
    {
      id: 'pr_0000000000a2',
      kind: 'tool',
      tool: 'Edit',
      patterns: ['docs/**'],
      patternKind: 'path',
      scope: 'session',
      session: 's-auth',
      sessionName: 'auth-refactor',
      project: API_SERVER,
      projectName: 'api-server',
      createdAt: now - 50 * M,
      createdBy: "Marcus's iPhone",
      usedAt: now - 31 * M,
      uses: 4,
    },
    {
      id: 'pr_0000000000a3',
      kind: 'tool',
      tool: 'WebFetch',
      patterns: ['docs.rs', '*.github.com'],
      patternKind: 'domain',
      scope: 'computer',
      createdAt: now - 9 * D,
      createdBy: 'Mac',
      usedAt: now - 2 * H,
      uses: 23,
    },
  ];
  if (machines) {
    // The standing grant of runs it had, kept as a permission.
    rules.push({
      id: 'pr_0000000000a4',
      kind: 'run',
      patterns: ['cargo test'],
      patternKind: 'command',
      scope: 'project',
      project: '/Users/marcus/dev/mother-claude',
      projectName: 'mother-claude',
      machine: MOCK_DESKTOP,
      machineName: 'DESKTOP-PC',
      createdAt: now - 2 * D,
      createdBy: 'Mac',
      usedAt: now - 20 * S,
      uses: 14,
    });
  }
  return rules;
}

export interface PermissionsHost {
  /** The desktop app: the dangerous right (the mock's phone has none). */
  desktop: boolean;
  /** Who saves one from here, as the server names them. */
  who: string;
  broadcast: (frame: ServerFrame) => void;
}

export class MockPermissions {
  private rules: PermissionRule[];
  private paused = false;
  private made = 0;

  constructor(
    private readonly host: PermissionsHost,
    opts: { now?: number; machines?: boolean } = {},
  ) {
    this.rules = seed(opts.now ?? Date.now(), !!opts.machines);
  }

  route(path: string, method: string, body: unknown): Response | null {
    if (path === '/permissions') {
      if (method === 'GET') return json(this.view());
      if (method !== 'PATCH') return null;
      const paused = (body as { paused?: unknown } | null)?.paused;
      if (typeof paused !== 'boolean') {
        return err(400, 'invalid_request', 'paused must be true or false.');
      }
      this.paused = paused;
      this.changed();
      return json(this.view());
    }
    const one = /^\/permissions\/([^/]+)$/.exec(path);
    if (!one) return null;
    const rule = this.rules.find((r) => r.id === decodeURIComponent(one[1]));
    if (!rule) return err(404, 'not_found', 'No such permission.');
    if (method === 'DELETE') {
      this.rules = this.rules.filter((r) => r !== rule);
      this.changed();
      return json(this.view());
    }
    return method === 'PATCH' ? this.patch(rule, body) : null;
  }

  /** `remember` on an allow of `item`: the permission it made (or grew),
   * or the refusal to answer with. */
  remember(item: PendingItem, choice: unknown): PermissionRule | Response {
    const g = item.grant;
    if (!g) return err(422, 'not_rememberable', 'Mother Claude can’t remember this answer.');
    if (g.blocked) return err(422, 'not_rememberable', g.blocked);
    const c = (choice ?? {}) as { scope?: unknown; patterns?: unknown };
    const scope = SCOPES.find((s) => s === c.scope);
    if (!scope) return err(400, 'invalid_request', 'scope is session, project or computer.');
    if (scope === 'project' && !g.project) {
      return err(422, 'no_project', 'This session has no project folder.');
    }
    const patterns = this.readPatterns(g.patterns, c.patterns);
    if (patterns instanceof Response) return patterns;
    if (scope !== 'session' && !this.host.desktop) return this.refused();
    // The same permission already: it grows (none of either: any).
    const same = this.rules.find((r) => sameTarget(r, g, scope));
    const grown =
      same && (!same.patterns.length || !patterns.length)
        ? []
        : [...new Set([...(same?.patterns ?? []), ...patterns])];
    if (same && grown.length <= MAX_PATTERNS) {
      same.patterns = grown;
      this.changed();
      return same;
    }
    const rule: PermissionRule = {
      id: `pr_${(0xb00 + ++this.made).toString(16).padStart(12, '0')}`,
      kind: g.kind,
      ...(g.tool ? { tool: g.tool } : {}),
      patterns,
      ...(g.patterns ? { patternKind: g.patterns } : {}),
      scope,
      ...(scope === 'session'
        ? { session: g.session, ...(g.sessionName ? { sessionName: g.sessionName } : {}) }
        : {}),
      ...(scope !== 'computer' && g.project
        ? { project: g.project, ...(g.projectName ? { projectName: g.projectName } : {}) }
        : {}),
      ...(g.machine ? { machine: g.machine, machineName: g.machineName } : {}),
      createdAt: Date.now(),
      createdBy: this.host.who,
      uses: 0,
    };
    this.rules.push(rule);
    this.changed();
    return rule;
  }

  /** The permission that allows `item` without asking (none while they
   * are paused, or for a dangerous request). */
  covering(item: PendingItem): PermissionRule | undefined {
    const g = item.grant;
    if (this.paused || item.dangerous || !g || g.blocked) return undefined;
    return this.rules.find((r) => covers(r, g, item));
  }

  /** It allowed a request. */
  use(rule: PermissionRule): void {
    rule.uses += 1;
    rule.usedAt = Date.now();
    this.changed();
  }

  /** A permission in a few words, as the server names it in
   * `pendingResolved` ("Bash commands like npm test in api-server"). */
  describe(r: PermissionRule): string {
    const where =
      r.scope === 'session'
        ? `in ${r.sessionName ?? 'its session'}`
        : r.scope === 'project'
          ? `in ${r.projectName ?? 'its project'}`
          : r.kind === 'tool'
            ? 'everywhere'
            : 'from every project';
    const on = r.kind === 'tool' ? '' : ` on ${r.machineName ?? 'the other computer'}`;
    if (!r.patterns.length) {
      const what =
        r.kind === 'run'
          ? 'any command'
          : r.kind === 'task'
            ? 'any task'
            : `every ${r.tool ?? 'tool'} call`;
      return `${what} ${where}${on}`;
    }
    const like =
      r.patternKind === 'path'
        ? 'files like'
        : r.patternKind === 'domain'
          ? 'pages on'
          : 'commands like';
    const tool = r.kind === 'tool' && r.tool ? `${r.tool} ` : '';
    return `${tool}${like} ${r.patterns.join(', ')} ${where}${on}`;
  }

  private view(): PermissionsView {
    return {
      rules: this.rules.map((r) => ({ ...r, patterns: [...r.patterns] })),
      paused: this.paused,
    };
  }

  private changed(): void {
    this.host.broadcast({ t: 'permissions' });
  }

  private refused(): Response {
    return err(
      403,
      'dangerous_blocked',
      'A project or the whole computer needs the desktop app, or a device allowed dangerous actions.',
    );
  }

  /** The patterns asked for (none: any), or why they can't be kept. */
  private readPatterns(kind: PatternKind | undefined, raw: unknown): string[] | Response {
    if (raw === undefined || raw === null) return [];
    if (!Array.isArray(raw) || raw.some((p) => typeof p !== 'string')) {
      return invalid('Patterns are a list of words.');
    }
    if (!kind) return invalid('This one takes no patterns.');
    const list = cleanPatterns(raw as string[]);
    if (!list.length || list.length > MAX_PATTERNS) {
      return invalid(`Give 1 to ${MAX_PATTERNS} patterns.`);
    }
    for (const p of list) {
      const why = patternProblem(kind, p);
      if (why) return invalid(why);
    }
    return list;
  }

  /** `PATCH /permissions/{id}`: its patterns (`[]`: any) or its scope. */
  private patch(rule: PermissionRule, body: unknown): Response {
    const b = (body ?? {}) as { patterns?: unknown; scope?: unknown };
    let scope = rule.scope;
    if (b.scope !== undefined) {
      const s = SCOPES.find((x) => x === b.scope);
      if (!s) return err(400, 'invalid_request', 'scope is session, project or computer.');
      if (s === 'session' && !rule.session) {
        return err(422, 'no_session', 'It was never kept for one session.');
      }
      if (s === 'project' && !rule.project) {
        return err(422, 'no_project', 'It has no project to be kept for.');
      }
      scope = s;
    }
    let patterns = rule.patterns;
    if (Array.isArray(b.patterns) && !b.patterns.length) {
      patterns = [];
    } else if (b.patterns !== undefined) {
      const read = this.readPatterns(rule.patternKind, b.patterns);
      if (read instanceof Response) return read;
      patterns = read;
    }
    const widens =
      wider(rule.scope, scope) || coversMore(rule.patternKind, rule.patterns, patterns);
    if (widens && scope !== 'session' && !this.host.desktop) return this.refused();
    rule.scope = scope;
    rule.patterns = patterns;
    this.changed();
    return json({ ...rule, patterns: [...rule.patterns] });
  }
}
