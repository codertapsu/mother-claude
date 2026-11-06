// Dev-only: teams of sessions (docs/TEAMS.md) for `?mock=1` — the built-in
// templates and the user's own, a demo team on "storefront" with its roles
// in every state (running, idle, waiting for capacity with the reasons, not
// started) and a board in every column, and every `/api/team-templates*`
// and `/api/teams*` route with the server's refusals and `teams` frames.
// Never bundled in production builds.

import { ProjectInfo, SessionSummary } from '../models';
import {
  ObservedEvent,
  RoleDef,
  RoleView,
  TaskPatch,
  TaskStatus,
  TeamTask,
  TeamTemplate,
  TeamView,
  TeamsFrame,
  TemplateEntry,
  TemplateIssue,
  TASK_STATUSES,
} from '../teams';

const M = 60_000;

type Body = Record<string, unknown>;

/** What the teams mock needs from the fixture backend. */
export interface TeamsHost {
  readonly desktop: boolean;
  broadcast(frame: TeamsFrame): void;
  projects(): ProjectInfo[];
  sessions(): SessionSummary[];
}

const role = (r: Partial<RoleDef> & { id: string; title: string }): RoleDef => ({
  permissionMode: 'default',
  ...r,
});

/** The five built-ins, as the server lists them (charters shortened). */
function builtins(): TemplateEntry[] {
  const ok = { errors: [], warnings: [] };
  const t = (template: TeamTemplate): TemplateEntry => ({ ...template, builtin: true, report: ok });
  return [
    t({
      id: 'software-delivery',
      name: 'Software delivery',
      description:
        'A lead with frontend and backend workers in their own worktrees, and a read-only scout.',
      roles: [
        role({
          id: 'lead',
          title: 'Lead',
          lead: true,
          model: 'opus',
          effort: 'high',
          paths: ['docs/**', '*.md'],
          autostart: true,
          charter: 'You run the delivery of features on this project.',
        }),
        role({
          id: 'frontend',
          title: 'Frontend',
          model: 'sonnet',
          effort: 'medium',
          permissionMode: 'acceptEdits',
          worktree: true,
          paths: ['src/app/**', 'src/components/**', 'public/**'],
          budgetUsd: 5,
          charter: 'You build the user interface.',
        }),
        role({
          id: 'backend',
          title: 'Backend & API',
          model: 'sonnet',
          effort: 'medium',
          permissionMode: 'acceptEdits',
          worktree: true,
          paths: ['src/server/**', 'server/**', 'api/**'],
          budgetUsd: 5,
          charter: 'You build the server side.',
        }),
        role({
          id: 'scout',
          title: 'Scout',
          model: 'haiku',
          effort: 'low',
          permissionMode: 'plan',
          readOnly: true,
          charter: 'You answer questions about the code base.',
        }),
      ],
      integration: { branch: 'main', mergeBy: 'lead', gate: 'npm test', review: true },
      idleStopMinutes: 30,
    }),
    t({
      id: 'content-production',
      name: 'Content production',
      description:
        'From a brief to a finished piece: plan, script, design, produce, check, publish.',
      roles: [
        role({
          id: 'planner',
          title: 'Planner',
          lead: true,
          model: 'opus',
          effort: 'high',
          autostart: true,
        }),
        role({
          id: 'scriptwriter',
          title: 'Scriptwriter',
          model: 'sonnet',
          permissionMode: 'acceptEdits',
          paths: ['scripts/**'],
        }),
        role({
          id: 'designer',
          title: 'Designer',
          model: 'sonnet',
          permissionMode: 'acceptEdits',
          paths: ['assets/**'],
        }),
        role({
          id: 'producer',
          title: 'Producer',
          model: 'sonnet',
          permissionMode: 'acceptEdits',
          paths: ['renders/**'],
        }),
        role({ id: 'qa', title: 'QA', model: 'opus', permissionMode: 'plan', readOnly: true }),
        role({ id: 'publisher', title: 'Publisher', model: 'sonnet', paths: ['publish/**'] }),
      ],
      integration: { mergeBy: 'planner', review: true },
      idleStopMinutes: 20,
    }),
    t({
      id: 'research',
      name: 'Research & analysis',
      description: 'A lead, a researcher, a synthesiser and a fact-checker.',
      roles: [
        role({ id: 'lead', title: 'Research lead', lead: true, model: 'opus', autostart: true }),
        role({
          id: 'researcher',
          title: 'Researcher',
          model: 'sonnet',
          permissionMode: 'acceptEdits',
          paths: ['notes/**'],
        }),
        role({
          id: 'synthesiser',
          title: 'Synthesiser',
          model: 'opus',
          permissionMode: 'acceptEdits',
          paths: ['reports/**'],
        }),
        role({
          id: 'fact-checker',
          title: 'Fact-checker',
          model: 'sonnet',
          permissionMode: 'plan',
          readOnly: true,
        }),
      ],
      integration: { mergeBy: 'lead', review: true },
      idleStopMinutes: 20,
    }),
    t({
      id: 'browser-operations',
      name: 'Browser operations',
      description: 'An operator that acts in the browser with approvals, and a read-only verifier.',
      roles: [
        role({
          id: 'operator',
          title: 'Operator',
          lead: true,
          model: 'sonnet',
          autostart: true,
          paths: ['runs/**'],
        }),
        role({
          id: 'verifier',
          title: 'Verifier',
          model: 'haiku',
          permissionMode: 'plan',
          readOnly: true,
        }),
      ],
      integration: { review: true },
      idleStopMinutes: 15,
    }),
    t({
      id: 'documentation',
      name: 'Documentation',
      description: 'An editor, a read-only librarian and a writer in its own worktree.',
      roles: [
        role({
          id: 'editor',
          title: 'Editor',
          lead: true,
          model: 'opus',
          autostart: true,
          paths: ['docs/**', '*.md'],
        }),
        role({
          id: 'librarian',
          title: 'Librarian',
          model: 'haiku',
          permissionMode: 'plan',
          readOnly: true,
        }),
        role({
          id: 'writer',
          title: 'Writer',
          model: 'sonnet',
          permissionMode: 'acceptEdits',
          worktree: true,
          paths: ['docs/**'],
        }),
      ],
      integration: { branch: 'main', mergeBy: 'editor', review: true },
      idleStopMinutes: 20,
    }),
  ];
}

/** The server's checks a mock can make. */
export function validateTemplate(t: Partial<TeamTemplate>): TemplateIssue[] {
  const out: TemplateIssue[] = [];
  const id = /^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/;
  if (typeof t.id !== 'string' || !id.test(t.id)) {
    out.push({ path: 'id', message: 'Use lower case letters, digits and - (at most 64).' });
  }
  if (typeof t.name !== 'string' || !t.name.trim()) {
    out.push({ path: 'name', message: 'Give the template a name.' });
  }
  const roles = Array.isArray(t.roles) ? t.roles : [];
  if (!roles.length) out.push({ path: 'roles', message: 'A team needs at least one role.' });
  const leads = roles.filter((r) => r.lead).length;
  if (roles.length && leads !== 1) {
    out.push({
      path: 'roles',
      message: leads ? 'Only one role can be the lead.' : 'Make one role the lead ("lead": true).',
    });
  }
  roles.forEach((r, i) => {
    if (!r.id || !id.test(r.id)) out.push({ path: `roles[${i}].id`, message: 'Not a role id.' });
    if (!r.title?.trim())
      out.push({ path: `roles[${i}].title`, message: 'Give the role a title.' });
    (r.paths ?? []).forEach((p, j) => {
      if (p.startsWith('/') || p.startsWith('~') || p.split(/[\\/]/).includes('..')) {
        out.push({
          path: `roles[${i}].paths[${j}]`,
          message: 'A path glob is relative to the project (no leading /, ~ or ..).',
        });
      }
    });
  });
  return out;
}

export class MockTeams {
  private readonly user = new Map<string, TemplateEntry>();
  private readonly teams = new Map<string, TeamView>();
  private readonly tasks = new Map<string, TeamTask[]>();
  private readonly events: ObservedEvent[] = [];
  private nextTask = new Map<string, number>();

  constructor(private readonly host: TeamsHost) {
    this.seed(Date.now());
  }

  /** The answer to a request this mock takes, or null. */
  route(path: string, method: string, body: unknown): Response | null {
    const b = (body && typeof body === 'object' ? body : {}) as Body;
    if (path === '/team-templates' && method === 'GET') {
      return json({ templates: [...builtins(), ...this.user.values()] });
    }
    if (path === '/team-templates/validate' && method === 'POST') {
      const t = (b['template'] ?? b) as Partial<TeamTemplate>;
      const errors = validateTemplate(t);
      return json({ ok: !errors.length, errors, warnings: [] });
    }
    if (path === '/team-templates' && method === 'POST') return this.saveTemplate(b, true);
    const tm = /^\/team-templates\/([^/]+)(?:\/(duplicate))?$/.exec(path);
    if (tm) {
      const id = decodeURIComponent(tm[1]);
      if (tm[2] === 'duplicate' && method === 'POST') return this.duplicate(id, b);
      if (method === 'PUT') {
        if (b['id'] !== id) {
          return fail(422, 'template_id_mismatch', "The template's id is the one in the address.");
        }
        return this.saveTemplate(b, false);
      }
      if (method === 'DELETE') {
        if (builtins().some((t) => t.id === id)) {
          return fail(409, 'builtin_template', 'A built-in template cannot be changed.');
        }
        if (!this.user.delete(id)) return fail(404, 'template_not_found', `No template "${id}".`);
        this.host.broadcast({ t: 'teams', templates: true });
        return new Response(null, { status: 204 });
      }
    }
    if (path === '/teams' && method === 'GET') {
      return json({ teams: [...this.teams.values()].sort((a, c) => c.createdAt - a.createdAt) });
    }
    if (path === '/teams' && method === 'POST') return this.start(b);
    if (path === '/teams/events' && method === 'GET')
      return json({ events: [...this.events].reverse() });
    const m = /^\/teams\/([^/]+)(?:\/(.+))?$/.exec(path);
    if (!m) return null;
    const team = this.teams.get(decodeURIComponent(m[1]));
    if (!team) return fail(404, 'team_not_found', 'No such team.');
    const rest = m[2] ?? '';
    if (!rest && method === 'GET') return json(team);
    if (!rest && method === 'DELETE') {
      if (!team.stopped) return fail(409, 'team_running', 'Stop the team before removing it.');
      this.teams.delete(team.id);
      this.host.broadcast({ t: 'teams', removed: team.id });
      return new Response(null, { status: 204 });
    }
    if (rest === 'stop' && method === 'POST') {
      return json(
        this.update(team, (t) => {
          t.stopped = true;
          for (const r of t.roles) {
            if (r.state !== 'notStarted') r.state = r.sessionId ? 'stopped' : 'notStarted';
            r.waitReasons = undefined;
          }
        }),
      );
    }
    if (rest === 'resume' && method === 'POST') {
      return json(this.update(team, (t) => (t.stopped = false)));
    }
    const rm = /^roles\/([^/]+)\/(start|stop|take)$/.exec(rest);
    if (rm) return this.roleAction(team, decodeURIComponent(rm[1]), rm[2], method, b);
    if (rest === 'tasks' && method === 'GET') {
      return json({ tasks: [...(this.tasks.get(team.id) ?? [])].sort((a, c) => c.id - a.id) });
    }
    if (rest === 'tasks' && method === 'POST') return this.createTask(team, b);
    const km = /^tasks\/(\d+)$/.exec(rest);
    if (km) {
      const task = (this.tasks.get(team.id) ?? []).find((t) => t.id === Number(km[1]));
      if (!task) return fail(404, 'task_not_found', `No task ${km[1]} on this board.`);
      if (method === 'GET') return json(task);
      if (method === 'PATCH') return this.patchTask(team, task, b as TaskPatch);
    }
    return null;
  }

  // ───────────────────────────────────────────────────────────── templates

  private saveTemplate(b: Body, create: boolean): Response {
    if (!this.host.desktop && !this.mayManage()) {
      return fail(403, 'desktop_only', 'Only the desktop app or a paired device can do this.');
    }
    const t = (b['template'] ?? b) as TeamTemplate;
    if (builtins().some((x) => x.id === t.id)) {
      return fail(409, 'builtin_template', 'A built-in template cannot be changed; duplicate it.');
    }
    const errors = validateTemplate(t);
    if (errors.length) {
      return fail(422, 'invalid_template', `${errors[0].path}: ${errors[0].message}`);
    }
    if (create && this.user.has(t.id)) {
      return fail(409, 'template_exists', `A template called "${t.id}" exists already.`);
    }
    if (!create && !this.user.has(t.id)) {
      return fail(404, 'template_not_found', `No template "${t.id}".`);
    }
    this.user.set(t.id, { ...t, builtin: false, report: { errors: [], warnings: [] } });
    this.host.broadcast({ t: 'teams', templates: true });
    return json(t, create ? 201 : 200);
  }

  /** A paired phone may write templates (`ManageData`). */
  private mayManage(): boolean {
    return true;
  }

  private duplicate(id: string, b: Body): Response {
    const src = [...builtins(), ...this.user.values()].find((t) => t.id === id);
    if (!src) return fail(404, 'template_not_found', `No template "${id}".`);
    let copy = `${id}-copy`;
    for (let n = 2; this.user.has(copy) || builtins().some((t) => t.id === copy); n++) {
      copy = `${id}-copy-${n}`;
    }
    const name =
      typeof b['name'] === 'string' && b['name'].trim() ? b['name'].trim() : `${src.name} (copy)`;
    const t: TemplateEntry = { ...src, id: copy, name, builtin: false };
    this.user.set(copy, t);
    this.host.broadcast({ t: 'teams', templates: true });
    return json(t, 201);
  }

  // ───────────────────────────────────────────────────────────── teams

  private start(b: Body): Response {
    const project = typeof b['project'] === 'string' ? b['project'] : '';
    if (!this.host.projects().some((p) => p.path === project)) {
      return fail(422, 'invalid_cwd', `${project || 'That'} is not a directory.`);
    }
    if ([...this.teams.values()].some((t) => t.project === project)) {
      return fail(
        409,
        'team_exists',
        'This project has a team already: resume it, or remove it first.',
      );
    }
    const template = [...builtins(), ...this.user.values()].find((t) => t.id === b['template']);
    if (!template) return fail(404, 'template_not_found', 'No such template.');
    const overrides = (b['overrides'] ?? {}) as Record<string, Partial<RoleDef>>;
    const roles = template.roles.map((r) => ({ ...r, ...(overrides[r.id] ?? {}) }));
    if (
      roles.some((r) => ['bypassPermissions', 'dontAsk'].includes(r.permissionMode ?? '')) &&
      !this.host.desktop
    ) {
      return fail(
        403,
        'dangerous_blocked',
        'Starting a team with a role that runs without permission prompts is restricted to the Mother Claude desktop app.',
      );
    }
    const label = project.split('/').filter(Boolean).pop() ?? 'project';
    const now = Date.now();
    const team: TeamView = {
      id: `tm_${Math.random().toString(16).slice(2, 14).padEnd(12, '0')}`,
      project,
      label,
      templateId: template.id,
      templateName: template.name,
      account: typeof b['account'] === 'string' ? b['account'] : 'default',
      stopped: false,
      createdAt: now,
      integration: template.integration ?? {},
      idleStopMinutes: template.idleStopMinutes ?? 30,
      reviews: template.integration?.review ?? true,
      roles: roles.map((r) => this.roleView(label, r, r.autostart ? 'starting' : 'notStarted')),
      counts: {},
      ...(typeof b['brief'] === 'string' && b['brief'] ? { brief: b['brief'] } : {}),
    };
    this.teams.set(team.id, team);
    this.tasks.set(team.id, []);
    this.host.broadcast({ t: 'teams', team });
    // The lead comes up a moment later.
    setTimeout(() => {
      this.update(team, (t) => {
        for (const r of t.roles) {
          if (r.state === 'starting') {
            r.state = 'idle';
            r.sessionId = crypto.randomUUID();
            r.startedAt = Date.now();
          }
        }
      });
    }, 700);
    return json(team, 201);
  }

  private roleView(label: string, r: RoleDef, state: RoleView['state']): RoleView {
    return {
      id: r.id,
      title: r.title,
      lead: !!r.lead,
      description: r.description,
      state,
      sessionName: `${label}/${r.id}`,
      foreign: false,
      model: r.model,
      effort: r.effort,
      permissionMode: r.permissionMode ?? 'default',
      worktree: !!r.worktree,
      worktreePath: r.worktree ? `.claude/worktrees/${r.id}` : undefined,
      paths: r.paths ?? [],
      readOnly: !!r.readOnly,
      budgetUsd: r.budgetUsd,
      budgetApplied: false,
      autostart: !!r.autostart,
      classes: r.classes ?? [],
      queuedTasks: 0,
      guardDenials: 0,
    };
  }

  private roleAction(
    team: TeamView,
    id: string,
    action: string,
    method: string,
    b: Body,
  ): Response {
    const r = team.roles.find((x) => x.id === id);
    if (!r) return fail(404, 'role_not_found', `No role "${id}" in this team.`);
    if (team.stopped && action !== 'stop') {
      return fail(409, 'team_stopped', 'Resume the team first.');
    }
    if (action === 'start' && method === 'POST') {
      if (r.state === 'running' || r.state === 'idle') {
        return fail(409, 'role_not_started', `The ${r.title} role's session runs already.`);
      }
      const running = team.roles.filter(
        (x) => !x.lead && ['running', 'idle', 'starting'].includes(x.state),
      ).length;
      if (!r.lead && running >= 3) {
        const reasons = [`${running} role sessions run already (the limit is 3)`];
        this.update(team, (t) => {
          const v = t.roles.find((x) => x.id === id);
          if (v)
            Object.assign(v, { state: 'waiting', waitReasons: reasons, waitingSince: Date.now() });
        });
        return json({ state: 'waiting', reasons });
      }
      const sessionId = r.sessionId ?? crypto.randomUUID();
      this.update(team, (t) => {
        const v = t.roles.find((x) => x.id === id);
        if (v)
          Object.assign(v, {
            state: 'idle',
            sessionId,
            waitReasons: undefined,
            startedAt: Date.now(),
          });
      });
      return json({ state: 'started', sessionId });
    }
    if (action === 'stop' && method === 'POST') {
      return json(
        this.update(team, (t) => {
          const v = t.roles.find((x) => x.id === id);
          if (v)
            Object.assign(v, {
              state: v.sessionId ? 'stopped' : 'notStarted',
              stoppedAt: Date.now(),
              waitReasons: undefined,
            });
        }),
      );
    }
    if (action === 'take' && method === 'POST') {
      const sid = typeof b['sessionId'] === 'string' ? b['sessionId'] : '';
      const s = this.host.sessions().find((x) => x.id === sid);
      if (!s || !s.live)
        return fail(409, 'session_not_live', 'Only a running session can take a role.');
      return json(
        this.update(team, (t) => {
          const v = t.roles.find((x) => x.id === id);
          if (v)
            Object.assign(v, {
              state: s.status === 'working' ? 'running' : 'idle',
              sessionId: sid,
              foreign: true,
            });
        }),
      );
    }
    if (action === 'take' && method === 'DELETE') {
      if (!r.foreign)
        return fail(
          409,
          'not_foreign',
          "The role's session is one Mother Claude runs: stop it instead.",
        );
      return json(
        this.update(team, (t) => {
          const v = t.roles.find((x) => x.id === id);
          if (v) Object.assign(v, { state: 'stopped', foreign: false, sessionId: undefined });
        }),
      );
    }
    return fail(405, 'method_not_allowed', 'Not here.');
  }

  /** Change a team, recount its board, publish it. */
  private update(team: TeamView, f: (t: TeamView) => void): TeamView {
    const t: TeamView = structuredClone(team);
    f(t);
    this.recount(t);
    this.teams.set(t.id, t);
    this.host.broadcast({ t: 'teams', team: t });
    return t;
  }

  private recount(t: TeamView): void {
    const tasks = this.tasks.get(t.id) ?? [];
    t.counts = Object.fromEntries(
      TASK_STATUSES.map((s) => [s, tasks.filter((x) => x.status === s).length]),
    ) as Record<TaskStatus, number>;
    for (const r of t.roles) {
      r.queuedTasks = tasks.filter((x) => x.status === 'todo' && x.role === r.id).length;
      r.currentTask = tasks.find(
        (x) =>
          (x.status === 'claimed' || x.status === 'in_progress') &&
          x.claimedBy &&
          x.claimedBy === r.sessionId,
      )?.id;
    }
  }

  // ───────────────────────────────────────────────────────────── the board

  private createTask(team: TeamView, b: Body): Response {
    const title = typeof b['title'] === 'string' ? b['title'].trim() : '';
    if (!title) return fail(422, 'invalid_task', 'A task needs a title.');
    const role = typeof b['role'] === 'string' ? b['role'] : undefined;
    if (role && !team.roles.some((r) => r.id === role)) {
      return fail(422, 'invalid_task', `No role "${role}" in this team.`);
    }
    const now = Date.now();
    const id = this.nextTask.get(team.id) ?? 1;
    this.nextTask.set(team.id, id + 1);
    const task: TeamTask = {
      id,
      title,
      goal: typeof b['goal'] === 'string' ? b['goal'] : '',
      acceptance: (b['acceptance'] as string[] | undefined) ?? [],
      role,
      class: typeof b['class'] === 'string' ? b['class'] : undefined,
      priority: typeof b['priority'] === 'number' ? b['priority'] : 2,
      status: 'todo',
      files: (b['files'] as string[] | undefined) ?? [],
      constraints: (b['constraints'] as string[] | undefined) ?? [],
      escalations: 0,
      createdAt: now,
      updatedAt: now,
      createdBy: this.host.desktop ? 'user (desktop)' : 'user (phone)',
      history: [
        { at: now, by: 'user', action: 'created', detail: role ? `for ${role}` : undefined },
      ],
    };
    this.tasks.set(team.id, [task, ...(this.tasks.get(team.id) ?? [])]);
    this.host.broadcast({ t: 'teams', teamId: team.id, task });
    this.update(team, () => undefined);
    return json(task, 201);
  }

  private patchTask(team: TeamView, task: TeamTask, p: TaskPatch): Response {
    const now = Date.now();
    const next: TeamTask = structuredClone(task);
    const by = this.host.desktop ? 'user (desktop)' : 'user (phone)';
    const log = (action: string, detail?: string) =>
      (next.history = [...(next.history ?? []), { at: now, by, action, detail }]);
    if (p.status && !(TASK_STATUSES as readonly string[]).includes(p.status)) {
      return fail(422, 'invalid_task', `"${p.status}" is not a status.`);
    }
    if (p.priority !== undefined) {
      if (p.priority < 0 || p.priority > 3) return fail(422, 'invalid_task', 'Priority is 0 to 3.');
      log('priority', `P${next.priority} → P${p.priority}`);
      next.priority = p.priority;
    }
    if (p.role && p.role !== next.role) {
      next.role = p.role;
      if (next.status === 'claimed' || next.status === 'in_progress') {
        next.status = 'todo';
        next.claimedBy = undefined;
      }
      log('role', p.role);
    }
    if (p.blocked !== undefined) {
      next.blocked = p.blocked || undefined;
      log(p.blocked ? 'blocked' : 'unblocked', p.blocked || undefined);
    }
    if (p.note) next.progress = [...(next.progress ?? []), { at: now, by, text: p.note }];
    if (p.status && p.status !== next.status) {
      const ended = ['done', 'failed', 'cancelled'].includes(next.status);
      if (p.status === 'cancelled' && ended) {
        return fail(
          409,
          'task_conflict',
          `Task ${next.id} cannot go from ${next.status} to cancelled.`,
        );
      }
      log('status', `${next.status} → ${p.status}`);
      next.status = p.status;
      if (p.status === 'todo') {
        next.claimedBy = undefined;
        next.endedAt = undefined;
        if (p.escalate) next.escalations += 1;
      } else if (['done', 'failed', 'cancelled'].includes(p.status)) {
        next.endedAt = now;
      }
    }
    next.updatedAt = now;
    this.tasks.set(
      team.id,
      (this.tasks.get(team.id) ?? []).map((t) => (t.id === next.id ? next : t)),
    );
    this.host.broadcast({ t: 'teams', teamId: team.id, task: next });
    this.update(team, () => undefined);
    return json(next);
  }

  // ───────────────────────────────────────────────────────────── the demo

  private seed(now: number): void {
    const project = '/Users/marcus/dev/storefront';
    const label = 'storefront';
    const sd = builtins()[0];
    const ids = {
      lead: 'b2c3d4e5-0000-4000-8000-00000000a001',
      frontend: 'b2c3d4e5-0000-4000-8000-00000000a002',
    };
    const roles = sd.roles.map((r) => this.roleView(label, r, 'notStarted'));
    const set = (id: string, v: Partial<RoleView>) =>
      Object.assign(roles.find((r) => r.id === id) ?? {}, v);
    set('lead', { state: 'idle', sessionId: ids.lead, startedAt: now - 42 * M, model: 'opus' });
    set('frontend', {
      state: 'running',
      sessionId: ids.frontend,
      startedAt: now - 30 * M,
      guardDenials: 1,
    });
    set('backend', {
      state: 'waiting',
      waitReasons: ['3 role sessions run already (the limit is 3)', 'Opus weekly usage at 91%'],
      waitingSince: now - 12 * M,
    });
    const team: TeamView = {
      id: 'tm_demo00000001',
      project,
      label,
      templateId: sd.id,
      templateName: sd.name,
      account: 'default',
      stopped: false,
      createdAt: now - 45 * M,
      integration: sd.integration ?? {},
      idleStopMinutes: 30,
      reviews: true,
      roles,
      counts: {},
      brief: 'Add sign-in with a password and a remember-me option.',
    };
    const task = (
      id: number,
      title: string,
      role: string,
      status: TaskStatus,
      over: Partial<TeamTask> = {},
    ): TeamTask => ({
      id,
      title,
      goal: '',
      role,
      priority: 2,
      status,
      escalations: 0,
      createdAt: now - (60 - id) * M,
      updatedAt: now - (30 - id) * M,
      createdBy: 'lead (lead)',
      history: [
        { at: now - (60 - id) * M, by: 'lead (lead)', action: 'created', detail: `for ${role}` },
      ],
      ...over,
    });
    const tasks: TeamTask[] = [
      task(1, 'Map the auth code', 'scout', 'done', {
        class: 'search',
        result: { summary: 'Auth lives in src/server/auth; sessions in a cookie.' },
        model: { model: 'haiku', effort: 'low' },
        modelReason: "the role's default",
      }),
      task(2, 'Login API', 'backend', 'review', {
        class: 'implement',
        priority: 1,
        goal: 'POST /api/login checks the password and sets the session cookie.',
        acceptance: ['Wrong password answers 401', 'Tests cover both cases'],
        result: {
          summary: 'Endpoint and tests; bcrypt for the hash.',
          branch: 'worktree-backend',
          changedFiles: ['src/server/auth/login.ts', 'src/server/auth/login.test.ts'],
        },
        model: { model: 'sonnet', effort: 'medium' },
      }),
      task(3, 'Login form', 'frontend', 'in_progress', {
        class: 'implement',
        priority: 1,
        goal: 'A form on /login that posts to /api/login and shows errors.',
        files: ['src/app/login/login.component.ts'],
        claimedBy: ids.frontend,
        startedAt: now - 20 * M,
        progress: [{ at: now - 8 * M, by: 'frontend', text: 'Form and validation drafted.' }],
        model: { model: 'sonnet', effort: 'medium' },
      }),
      task(4, 'Remember me', 'backend', 'todo', { priority: 2, class: 'implement' }),
      task(5, 'Password reset email', 'backend', 'todo', {
        priority: 3,
        blocked: 'Which email provider do we use?',
      }),
      task(6, 'Rate-limit login attempts', 'backend', 'failed', {
        result: { summary: 'The rate limiter needs Redis, which is not in this project.' },
        escalations: 1,
      }),
    ];
    this.teams.set(team.id, team);
    this.tasks.set(team.id, tasks);
    this.nextTask.set(team.id, 7);
    this.recount(team);
    this.events.push(
      {
        at: now - 50 * M,
        sessionId: 'a1b2c3d4-0000-4000-8000-000000000001',
        event: 'TaskCreated',
        teammate: 'researcher',
        subject: 'Compare the three vendors',
      },
      {
        at: now - 20 * M,
        sessionId: 'a1b2c3d4-0000-4000-8000-000000000001',
        event: 'TeammateIdle',
        teammate: 'researcher',
      },
    );
  }
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
