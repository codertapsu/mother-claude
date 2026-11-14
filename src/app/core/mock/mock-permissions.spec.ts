import { PendingItem, ServerFrame } from '../models';
import { PermissionRule, PermissionsView } from '../permission-rules';
import { MockPermissions } from './mock-permissions';

/** Calls the mock's route: its status and its JSON. */
async function call(
  mock: MockPermissions,
  path: string,
  method = 'GET',
  body?: unknown,
): Promise<{ status: number; body: unknown }> {
  const r = mock.route(path, method, body);
  expect(r, `${method} ${path}`).not.toBeNull();
  return { status: r!.status, body: await r!.json() };
}

/** A Bash request in `shop`, as a session asks it. */
const bash = (command: string, over: Partial<PendingItem> = {}): PendingItem => ({
  id: `req-${command}`,
  sessionId: 's1',
  kind: 'permission',
  source: 'owned',
  toolName: 'Bash',
  title: 'Allow Bash?',
  input: { command },
  suggestions: [],
  dangerous: false,
  answerable: true,
  createdAt: 1,
  grant: {
    kind: 'tool',
    tool: 'Bash',
    patterns: 'command',
    suggested: [command],
    session: 's1',
    sessionName: 'tests',
    project: '/Users/me/dev/shop',
    projectName: 'shop',
  },
  ...over,
});

/** What the answer to a `remember` is when it is refused. */
const refusal = async (r: PermissionRule | Response) => {
  expect(r).toBeInstanceOf(Response);
  const res = r as Response;
  return {
    status: res.status,
    code: ((await res.json()) as { error: { code: string } }).error.code,
  };
};

describe('the permissions mock (`?mock=1`)', () => {
  let frames: ServerFrame[];
  const make = (desktop: boolean, machines = false) =>
    new MockPermissions(
      { desktop, who: desktop ? 'Mac' : 'Phone', broadcast: (f) => frames.push(f) },
      { now: Date.now(), machines },
    );

  beforeEach(() => {
    frames = [];
  });

  it('serves three permissions, the run grant of DESKTOP-PC too with machines, and pauses them', async () => {
    let view = (await call(make(true), '/permissions')).body as PermissionsView;
    expect(view.paused).toBe(false);
    expect(view.rules.map((r) => [r.kind, r.tool, r.scope, r.patterns])).toEqual([
      ['tool', 'Bash', 'project', ['npm test', 'npm run lint']],
      ['tool', 'Edit', 'session', ['docs/**']],
      ['tool', 'WebFetch', 'computer', ['docs.rs', '*.github.com']],
    ]);
    const mock = make(true, true);
    view = (await call(mock, '/permissions')).body as PermissionsView;
    expect(view.rules[3]).toEqual(
      expect.objectContaining({
        kind: 'run',
        patterns: ['cargo test'],
        scope: 'project',
        machineName: 'DESKTOP-PC',
      }),
    );
    view = (await call(mock, '/permissions', 'PATCH', { paused: true })).body as PermissionsView;
    expect(view.paused).toBe(true);
    expect(view.rules.length).toBe(4);
    expect(frames).toEqual([{ t: 'permissions' }]);
    expect((await call(mock, '/permissions', 'PATCH', { paused: 'yes' })).status).toBe(400);
  });

  it('remembers an answer, and grows the same permission', async () => {
    const mock = make(true);
    const rule = mock.remember(bash('cargo test --lib'), {
      scope: 'project',
      patterns: [' cargo test ', 'cargo test'],
    }) as PermissionRule;
    expect(rule).toEqual(
      expect.objectContaining({
        kind: 'tool',
        tool: 'Bash',
        patterns: ['cargo test'],
        patternKind: 'command',
        scope: 'project',
        project: '/Users/me/dev/shop',
        projectName: 'shop',
        createdBy: 'Mac',
        uses: 0,
      }),
    );
    expect(rule.id).toMatch(/^pr_[0-9a-f]{12}$/);
    expect(rule.session).toBeUndefined();
    expect(frames).toEqual([{ t: 'permissions' }]);
    // The same one again, with another pattern: it grows.
    const again = mock.remember(bash('npm run build'), {
      scope: 'project',
      patterns: ['npm run build'],
    }) as PermissionRule;
    expect(again.id).toBe(rule.id);
    expect(again.patterns).toEqual(['cargo test', 'npm run build']);
    // This session only, any command.
    const session = mock.remember(bash('ls'), { scope: 'session' }) as PermissionRule;
    expect(session).toEqual(
      expect.objectContaining({
        scope: 'session',
        session: 's1',
        sessionName: 'tests',
        patterns: [],
      }),
    );
    expect(mock.describe(rule)).toBe('Bash commands like cargo test, npm run build in shop');
    expect(mock.describe(session)).toBe('every Bash call in tests');
  });

  it('keeps a project or everywhere for the dangerous right, this session for anyone', async () => {
    const phone = make(false);
    expect(await refusal(phone.remember(bash('cargo test'), { scope: 'project' }))).toEqual({
      status: 403,
      code: 'dangerous_blocked',
    });
    expect(await refusal(phone.remember(bash('cargo test'), { scope: 'computer' }))).toEqual({
      status: 403,
      code: 'dangerous_blocked',
    });
    expect(phone.remember(bash('cargo test'), { scope: 'session' })).not.toBeInstanceOf(Response);
  });

  it('refuses what cannot be kept', async () => {
    const mock = make(true);
    const invalid = async (patterns: unknown) =>
      refusal(mock.remember(bash('cargo test'), { scope: 'session', patterns }));
    for (const bad of [['cargo test && rm -rf /'], ['echo $(id)'], ['bash'], [], ['  ']]) {
      expect(await invalid(bad), JSON.stringify(bad)).toEqual({
        status: 422,
        code: 'invalid_pattern',
      });
    }
    expect(await invalid(Array.from({ length: 21 }, (_, i) => `make t${i}`))).toEqual({
      status: 422,
      code: 'invalid_pattern',
    });
    const edit = bash('x', {
      toolName: 'Edit',
      grant: { ...bash('x').grant!, tool: 'Edit', patterns: 'path' },
    });
    for (const bad of ['/etc/passwd', '../secrets/**', 'C:\\Windows\\*']) {
      expect(
        await refusal(mock.remember(edit, { scope: 'session', patterns: [bad] })),
        bad,
      ).toEqual({ status: 422, code: 'invalid_pattern' });
    }
    const fetch = bash('x', {
      grant: { ...bash('x').grant!, tool: 'WebFetch', patterns: 'domain' },
    });
    expect(
      await refusal(mock.remember(fetch, { scope: 'session', patterns: ['https://x/'] })),
    ).toEqual({ status: 422, code: 'invalid_pattern' });
    expect(
      mock.remember(fetch, { scope: 'session', patterns: ['*.github.com'] }),
    ).not.toBeInstanceOf(Response);
    // A task takes no patterns; a dangerous request is never remembered;
    // a session without a project folder has no project to keep it for.
    const task = bash('x', { grant: { ...bash('x').grant!, kind: 'task', patterns: undefined } });
    expect(await refusal(mock.remember(task, { scope: 'session', patterns: ['a'] }))).toEqual({
      status: 422,
      code: 'invalid_pattern',
    });
    const blocked = bash('x', {
      grant: { ...bash('x').grant!, blocked: 'Dangerous actions always ask.' },
    });
    expect(await refusal(mock.remember(blocked, { scope: 'session' }))).toEqual({
      status: 422,
      code: 'not_rememberable',
    });
    const loose = bash('x', { grant: { ...bash('x').grant!, project: undefined } });
    expect(await refusal(mock.remember(loose, { scope: 'project' }))).toEqual({
      status: 422,
      code: 'no_project',
    });
    expect(
      await refusal(mock.remember(bash('x', { grant: undefined }), { scope: 'session' })),
    ).toEqual({ status: 422, code: 'not_rememberable' });
  });

  it('allows what a permission covers, and nothing while they are paused', async () => {
    const mock = make(true);
    mock.remember(bash('cargo test'), { scope: 'project', patterns: ['cargo test'] });
    expect(mock.covering(bash('cargo test --lib'))?.patterns).toEqual(['cargo test']);
    expect(mock.covering(bash('cargo test-foo'))).toBeUndefined();
    // One that chains or redirects always asks.
    expect(mock.covering(bash('cargo test; rm -rf ~'))).toBeUndefined();
    expect(mock.covering(bash('cargo test > out.txt'))).toBeUndefined();
    // Another project's session, a dangerous request.
    const elsewhere = bash('cargo test', {
      grant: { ...bash('cargo test').grant!, project: '/Users/me/dev/blog' },
    });
    expect(mock.covering(elsewhere)).toBeUndefined();
    expect(mock.covering(bash('cargo test', { dangerous: true }))).toBeUndefined();
    await call(mock, '/permissions', 'PATCH', { paused: true });
    expect(mock.covering(bash('cargo test'))).toBeUndefined();
    // The seeded ones: a site, a path in this session.
    await call(mock, '/permissions', 'PATCH', { paused: false });
    const page = bash('x', {
      toolName: 'WebFetch',
      input: { url: 'https://api.github.com/repos' },
      grant: { ...bash('x').grant!, tool: 'WebFetch', patterns: 'domain' },
    });
    expect(mock.covering(page)?.tool).toBe('WebFetch');
    const doc = bash('x', {
      sessionId: 's-auth',
      toolName: 'Edit',
      input: { file_path: '/Users/marcus/dev/api-server/docs/auth.md' },
      grant: {
        kind: 'tool',
        tool: 'Edit',
        patterns: 'path',
        session: 's-auth',
        project: '/Users/marcus/dev/api-server',
      },
    });
    const used = mock.covering(doc)!;
    expect(used.patterns).toEqual(['docs/**']);
    const uses = used.uses;
    mock.use(used);
    expect(used.uses).toBe(uses + 1);
    expect(used.usedAt).toBeGreaterThan(0);
  });

  it('edits and removes a permission, widening it only with the dangerous right', async () => {
    const phone = make(false);
    const list = ((await call(phone, '/permissions')).body as PermissionsView).rules;
    const bashRule = list[0];
    // Narrower: anyone.
    let r = await call(phone, `/permissions/${bashRule.id}`, 'PATCH', { patterns: ['npm test'] });
    expect(r.status).toBe(200);
    expect((r.body as PermissionRule).patterns).toEqual(['npm test']);
    // Wider (any command, every project): the dangerous right.
    r = await call(phone, `/permissions/${bashRule.id}`, 'PATCH', { patterns: [] });
    expect(r.status).toBe(403);
    r = await call(phone, `/permissions/${bashRule.id}`, 'PATCH', { scope: 'computer' });
    expect(r.status).toBe(403);
    r = await call(phone, `/permissions/${bashRule.id}`, 'PATCH', { patterns: ['npm'] });
    expect(r.status).toBe(403);
    // A session's own: anyone, wider or not.
    r = await call(phone, `/permissions/${list[1].id}`, 'PATCH', {
      patterns: ['docs/**', 'README.md'],
    });
    expect(r.status).toBe(200);
    r = await call(phone, `/permissions/${list[1].id}`, 'PATCH', { scope: 'project' });
    expect(r.status).toBe(403);
    // A pattern that can't be kept; a scope it has nothing for.
    r = await call(phone, `/permissions/${bashRule.id}`, 'PATCH', { patterns: ['a | b'] });
    expect(r.status).toBe(422);
    r = await call(phone, `/permissions/${list[2].id}`, 'PATCH', { scope: 'session' });
    expect(r.status).toBe(422);

    const desk = make(true);
    r = await call(desk, `/permissions/${bashRule.id}`, 'PATCH', {
      scope: 'computer',
      patterns: [],
    });
    expect(r.body).toEqual(expect.objectContaining({ scope: 'computer', patterns: [] }));
    const left = (await call(desk, `/permissions/${bashRule.id}`, 'DELETE'))
      .body as PermissionsView;
    expect(left.rules.map((x) => x.id)).not.toContain(bashRule.id);
    expect((await call(desk, `/permissions/${bashRule.id}`, 'DELETE')).status).toBe(404);
    expect(frames.length).toBeGreaterThan(2);
  });
});
