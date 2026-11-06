import {
  RoleView,
  TeamTask,
  TeamView,
  applyOverrides,
  columns,
  lines,
  needsDangerous,
  openCount,
  parseTemplate,
  roleStatusLine,
  templateJson,
  upsertTask,
  upsertTeam,
} from './teams';

const task = (id: number, status: TeamTask['status'], priority = 2): TeamTask => ({
  id,
  title: `Task ${id}`,
  goal: '',
  priority,
  status,
  escalations: 0,
  createdAt: id,
  updatedAt: id,
  createdBy: 'lead',
});

const role = (over: Partial<RoleView> = {}): RoleView => ({
  id: 'frontend',
  title: 'Frontend',
  lead: false,
  state: 'idle',
  sessionName: 'shop/frontend',
  foreign: false,
  permissionMode: 'acceptEdits',
  worktree: true,
  paths: [],
  readOnly: false,
  budgetApplied: false,
  autostart: false,
  classes: [],
  queuedTasks: 0,
  guardDenials: 0,
  ...over,
});

const team = (id: string, createdAt: number): TeamView => ({
  id,
  project: '/w/shop',
  label: 'shop',
  templateId: 'software-delivery',
  templateName: 'Software delivery',
  account: 'default',
  stopped: false,
  createdAt,
  integration: {},
  idleStopMinutes: 30,
  reviews: true,
  roles: [],
  counts: { todo: 2, claimed: 1, review: 1, done: 4, failed: 1 },
});

describe('teams helpers', () => {
  it('sorts the board into columns by priority, then age', () => {
    const c = columns([
      task(3, 'todo', 2),
      task(1, 'todo', 2),
      task(2, 'todo', 0),
      task(4, 'done'),
    ]);
    expect(c.todo.map((t) => t.id)).toEqual([2, 1, 3]);
    expect(c.done.map((t) => t.id)).toEqual([4]);
    expect(c.review).toEqual([]);
  });

  it('adds and replaces tasks and teams', () => {
    let tasks = upsertTask([task(1, 'todo')], task(2, 'todo'));
    expect(tasks.map((t) => t.id)).toEqual([2, 1]);
    tasks = upsertTask(tasks, { ...task(1, 'done') });
    expect(tasks.find((t) => t.id === 1)?.status).toBe('done');
    const teams = upsertTeam([team('a', 1)], team('b', 2));
    expect(teams.map((t) => t.id)).toEqual(['b', 'a']);
    expect(
      upsertTeam(teams, { ...team('a', 1), stopped: true }).find((t) => t.id === 'a')?.stopped,
    ).toBeTrue();
  });

  it('applies overrides and knows when a start needs the dangerous right', () => {
    const roles = [
      { id: 'lead', title: 'Lead', lead: true, model: 'opus' },
      { id: 'dev', title: 'Dev', permissionMode: 'acceptEdits' },
    ];
    const out = applyOverrides(roles, { dev: { model: 'haiku', autostart: true } });
    expect(out[1].model).toBe('haiku');
    expect(out[1].autostart).toBeTrue();
    expect(out[1].permissionMode).toBe('acceptEdits');
    expect(out[0]).toBe(roles[0]);
    expect(needsDangerous(out)).toBeFalse();
    expect(
      needsDangerous(applyOverrides(roles, { dev: { permissionMode: 'bypassPermissions' } })),
    ).toBeTrue();
  });

  it('parses templates tolerantly and shows them without the list fields', () => {
    expect(parseTemplate('{nope').error).toContain('Not JSON');
    expect(parseTemplate('[]').error).toBe('A template is a JSON object.');
    expect(parseTemplate('{"id":"x"}').error).toContain('"roles"');
    const ok = parseTemplate('{"id":"x","name":"X","roles":[],"future":1}');
    expect(ok.template?.['future']).toBe(1);
    const shown = templateJson({
      id: 'x',
      name: 'X',
      roles: [],
      builtin: true,
      report: { errors: [], warnings: [] },
    });
    expect(shown).not.toContain('builtin');
    expect(shown).not.toContain('report');
  });

  it('says why a role waits', () => {
    expect(roleStatusLine(role())).toBe('Idle');
    expect(
      roleStatusLine(role({ state: 'waiting', waitReasons: ['3 role sessions run already'] })),
    ).toBe('Waiting: 3 role sessions run already');
    expect(roleStatusLine(role({ state: 'stopped', lastError: 'spawn failed' }))).toBe(
      'Stopped · spawn failed',
    );
  });

  it('counts open tasks and reads textarea lines', () => {
    expect(openCount(team('a', 1))).toBe(4);
    expect(lines(' a \n\n b\n')).toEqual(['a', 'b']);
  });
});
