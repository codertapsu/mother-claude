import { ProjectInfo, SessionSummary } from '../models';
import { TeamTask, TeamView, TeamsFrame, TemplateEntry } from '../teams';
import { MockTeams, validateTemplate } from './mock-teams';

describe('MockTeams', () => {
  let frames: TeamsFrame[];
  let desktop: boolean;
  let mock: MockTeams;
  const projects: ProjectInfo[] = [
    { name: 'storefront', path: '/Users/marcus/dev/storefront' },
    { name: 'shop', path: '/Users/marcus/dev/shop' },
  ];

  beforeEach(() => {
    frames = [];
    desktop = true;
    mock = new MockTeams({
      get desktop() {
        return desktop;
      },
      broadcast: (f) => frames.push(f),
      projects: () => projects,
      sessions: () => [] as SessionSummary[],
    });
  });

  const call = async (path: string, method = 'GET', body?: unknown) => {
    const r = mock.route(path, method, body);
    expect(r, `${method} ${path}`).not.toBeNull();
    const res = r as Response;
    return { status: res.status, body: res.status === 204 ? null : await res.json() };
  };

  it('lists the built-in templates, one of yours, and a demo team in every state', async () => {
    const { body } = await call('/team-templates');
    const list = body.templates as TemplateEntry[];
    expect(list.map((t) => t.id)).toEqual([
      'software-delivery',
      'content-production',
      'research',
      'browser-operations',
      'documentation',
      'weekly-newsletter',
    ]);
    // Built-ins with the server's flows (arranged by the editor), yours
    // placed by it.
    expect(list.slice(0, 5).map((t) => t.flow?.length)).toEqual([2, 5, 2, 1, 1]);
    expect(list[1].flow?.[0]).toEqual({ from: 'planner', to: 'scriptwriter', note: 'the outline' });
    expect(list[1].layout).toBeUndefined();
    expect(list[5].builtin).toBe(false);
    expect(Object.keys(list[5].layout ?? {})).toContain('@merge');
    expect(validateTemplate(list[5])).toEqual([]);
    const teams = (await call('/teams')).body.teams as TeamView[];
    expect(teams.length).toBe(1);
    const states = teams[0].roles.map((r) => r.state);
    expect(states).toEqual(['idle', 'running', 'waiting', 'notStarted']);
    expect(teams[0].roles[2].waitReasons?.length).toBe(2);
    const tasks = (await call(`/teams/${teams[0].id}/tasks`)).body.tasks as TeamTask[];
    expect(new Set(tasks.map((t) => t.status)).size).toBeGreaterThan(4);
  });

  it('starts a team with the server refusals', async () => {
    expect(
      (await call('/teams', 'POST', { project: '/nowhere', template: 'research' })).status,
    ).toBe(422);
    const taken = await call('/teams', 'POST', {
      project: '/Users/marcus/dev/storefront',
      template: 'research',
    });
    expect(taken.status).toBe(409);
    desktop = false;
    const danger = await call('/teams', 'POST', {
      project: '/Users/marcus/dev/shop',
      template: 'research',
      overrides: { researcher: { permissionMode: 'bypassPermissions' } },
    });
    expect(danger.status).toBe(403);
    const ok = await call('/teams', 'POST', {
      project: '/Users/marcus/dev/shop',
      template: 'software-delivery',
      brief: 'Do it',
    });
    expect(ok.status).toBe(201);
    expect(ok.body.label).toBe('shop');
    expect(ok.body.roles[0].state).toBe('starting');
    expect(frames.some((f) => f.team?.id === ok.body.id)).toBe(true);
  });

  it('works the board and publishes each task', async () => {
    const team = (await call('/teams')).body.teams[0] as TeamView;
    const created = await call(`/teams/${team.id}/tasks`, 'POST', {
      title: 'Footer',
      role: 'frontend',
      priority: 3,
    });
    expect(created.status).toBe(201);
    const id = created.body.id as number;
    expect(frames.some((f) => f.task?.id === id && f.teamId === team.id)).toBe(true);
    const moved = await call(`/teams/${team.id}/tasks/${id}`, 'PATCH', { priority: 0 });
    expect(moved.body.priority).toBe(0);
    expect(moved.body.history.at(-1).action).toBe('priority');
    const bad = await call(`/teams/${team.id}/tasks/${id}`, 'PATCH', { status: 'sideways' });
    expect(bad.status).toBe(422);
    const cancelled = await call(`/teams/${team.id}/tasks/${id}`, 'PATCH', { status: 'cancelled' });
    expect(cancelled.body.status).toBe('cancelled');
    expect(
      (await call(`/teams/${team.id}/tasks/${id}`, 'PATCH', { status: 'cancelled' })).status,
    ).toBe(200);
    expect(
      (await call(`/teams/${team.id}/tasks`, 'POST', { title: 'x', role: 'nobody' })).status,
    ).toBe(422);
  });

  it("keeps built-ins fixed and validates the user's templates", async () => {
    expect((await call('/team-templates/research', 'DELETE')).status).toBe(409);
    const copy = await call('/team-templates/research/duplicate', 'POST', { name: 'Mine' });
    expect(copy.status).toBe(201);
    expect(copy.body.id).toBe('research-copy');
    const bad = await call(`/team-templates/${copy.body.id}`, 'PUT', { ...copy.body, roles: [] });
    expect(bad.status).toBe(422);
    const v = await call('/team-templates/validate', 'POST', { id: 'X Y', name: '', roles: [] });
    expect(v.body.ok).toBe(false);
    expect(
      validateTemplate({ id: 'a', name: 'A', roles: [{ id: 'r', title: 'R' }] })[0].message,
    ).toContain('lead');
    expect((await call(`/team-templates/${copy.body.id}`, 'DELETE')).status).toBe(204);
    expect(frames.filter((f) => f.templates).length).toBe(2);
  });

  it('reports a flow that names no role, a role to itself, twice, a long note', async () => {
    const roles = [
      { id: 'lead', title: 'Lead', lead: true },
      { id: 'dev', title: 'Dev' },
    ];
    const flow = [
      { from: 'dev', to: 'ghost' },
      { from: 'dev', to: 'dev' },
      { from: 'lead', to: 'dev' },
      { from: 'lead', to: 'dev', note: 'x'.repeat(201) },
      { from: 'nobody', to: 'lead' },
    ];
    const v = await call('/team-templates/validate', 'POST', {
      id: 'mine',
      name: 'Mine',
      roles,
      flow,
      integration: { mergeBy: 'ghost' },
    });
    expect(v.body.ok).toBe(false);
    expect((v.body.errors as { path: string }[]).map((e) => e.path)).toEqual([
      'flow[0].to',
      'flow[1]',
      'flow[3]',
      'flow[3].note',
      'flow[4].from',
      'integration.mergeBy',
    ]);
    // Saved, a flow and a layout stay as they were sent.
    const saved = await call('/team-templates', 'POST', {
      id: 'mine',
      name: 'Mine',
      roles,
      flow: [{ from: 'lead', to: 'dev', note: 'the plan' }],
      layout: { lead: { x: 0, y: 0 }, '@merge': { x: 300, y: 0 } },
    });
    expect(saved.status).toBe(201);
    const mine = ((await call('/team-templates')).body.templates as TemplateEntry[]).find(
      (t) => t.id === 'mine',
    );
    expect(mine?.flow).toEqual([{ from: 'lead', to: 'dev', note: 'the plan' }]);
    expect(mine?.layout?.['@merge']).toEqual({ x: 300, y: 0 });
  });

  it('stops and removes a team', async () => {
    const team = (await call('/teams')).body.teams[0] as TeamView;
    expect((await call(`/teams/${team.id}`, 'DELETE')).status).toBe(409);
    const stopped = await call(`/teams/${team.id}/stop`, 'POST');
    expect(stopped.body.stopped).toBe(true);
    expect((await call(`/teams/${team.id}/roles/frontend/start`, 'POST')).status).toBe(409);
    expect((await call(`/teams/${team.id}`, 'DELETE')).status).toBe(204);
    expect(frames.at(-1)?.removed).toBe(team.id);
  });
});
