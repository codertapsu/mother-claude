import { TestBed } from '@angular/core/testing';

import { hostOsSignal, useLanguage } from './i18n/i18n';
import { FakeBackend, flush, me, useFakeServer } from './mock/fake-server.testing';
import { setActiveMock } from './mock/mock-api';
import { GrantOffer } from './models';
import { resolutionText } from './pending.service';
import {
  PermissionRule,
  cleanPatterns,
  needsDangerous,
  normalizePermissions,
  quickChoices,
  rememberedText,
  ruleAction,
  scopeLabel,
  wider,
} from './permission-rules';
import { PermissionRulesService } from './permission-rules.service';
import { RealtimeService } from './realtime.service';

const bash: GrantOffer = {
  kind: 'tool',
  tool: 'Bash',
  patterns: 'command',
  suggested: ['cargo test'],
  session: 's1',
  sessionName: 'tests',
  project: '/Users/me/dev/shop',
  projectName: 'shop',
};

const run: GrantOffer = {
  kind: 'run',
  patterns: 'command',
  suggested: ['npm run'],
  session: 's1',
  project: '/Users/me/dev/shop',
  projectName: 'shop',
  machine: 'm1',
  machineName: 'MacBook',
};

const rule = (over: Partial<PermissionRule> = {}): PermissionRule => ({
  id: 'pr_000000000001',
  kind: 'tool',
  tool: 'Bash',
  patterns: [],
  scope: 'project',
  project: '/Users/me/dev/shop',
  projectName: 'shop',
  createdAt: 1,
  createdBy: 'Mac',
  uses: 0,
  ...over,
});

describe('permission words (core/permission-rules.ts)', () => {
  beforeEach(() => hostOsSignal.set('macos'));
  afterEach(() => hostOsSignal.set(undefined));

  it('words the one-tap choices for what is asked', () => {
    const labels = (o: GrantOffer, host: string | null = null) =>
      quickChoices(o, host).map((c) => c.label);
    expect(labels(bash)).toEqual([
      'In this session',
      'In shop (every session)',
      'Everywhere on this Mac',
    ]);
    expect(quickChoices(bash).map((c) => c.hint)).toEqual([
      'Any Bash command, in this session only.',
      'Any Bash command, in every session of shop.',
      'Any Bash command, in every project on this Mac.',
    ]);
    expect(labels(run)).toEqual([
      'Commands from this session',
      'Commands from shop',
      'Every command on MacBook',
    ]);
    expect(labels({ ...run, kind: 'task', patterns: undefined })).toEqual([
      'Tasks from this session',
      'Tasks from shop',
      'Every task handed to MacBook',
    ]);
    // Another computer's request: everywhere on that one.
    expect(labels(bash, 'DESKTOP-PC')[2]).toBe('Everywhere on DESKTOP-PC');
    // No project folder: no project to choose.
    expect(quickChoices({ ...bash, project: undefined }).map((c) => c.scope)).toEqual([
      'session',
      'computer',
    ]);
    expect(quickChoices({ ...bash, tool: 'Edit' })[0].hint).toBe(
      'Changing any file, in this session only.',
    );
    expect(quickChoices({ ...bash, tool: 'mcp__github__create_issue' })[1].hint).toBe(
      'Every use of mcp__github__create_issue, in every session of shop.',
    );
  });

  it('says what a rule allows, and how an answer was remembered', () => {
    expect(ruleAction(rule())).toBe('Any Bash command');
    expect(ruleAction(rule({ patterns: ['cargo test'], patternKind: 'command' }))).toBe(
      'Bash commands that start with',
    );
    expect(ruleAction(rule({ tool: 'Edit', patterns: ['src/**'], patternKind: 'path' }))).toBe(
      'Changing files that match',
    );
    expect(ruleAction(rule({ tool: 'WebFetch', patterns: ['docs.rs'] }))).toBe('Fetching pages on');
    expect(ruleAction(rule({ kind: 'run', tool: undefined, machineName: 'MacBook' }))).toBe(
      'Any command on MacBook',
    );
    expect(
      ruleAction(rule({ kind: 'run', tool: undefined, machineName: 'MacBook', patterns: ['npm'] })),
    ).toBe('Commands on MacBook that start with');
    expect(scopeLabel('tool', 'session', { session: 'tests' })).toBe('In the session tests');
    expect(scopeLabel('run', 'computer')).toBe('From every project');
    expect(rememberedText(bash, 'project')).toBe('Allowed and remembered: In shop (every session)');
    expect(rememberedText(run, 'session', ['cargo test', 'npm run'])).toBe(
      'Allowed and remembered: From this session, matching cargo test, npm run',
    );
  });

  it('cleans patterns, ranks scopes and reads the list tolerantly', () => {
    expect(cleanPatterns([' cargo test ', '', 'cargo test', 'npm run'])).toEqual([
      'cargo test',
      'npm run',
    ]);
    expect(needsDangerous('session')).toBe(false);
    expect(needsDangerous('project')).toBe(true);
    expect(wider('session', 'computer')).toBe(true);
    expect(wider('computer', 'project')).toBe(false);
    const view = normalizePermissions({
      paused: true,
      rules: [
        { id: 'pr_1', kind: 'tool', tool: 'Bash', scope: 'session', patterns: ['a', 3] },
        { id: 'pr_2', kind: 'someday', scope: 'project' },
        { id: 'pr_3', kind: 'run', scope: 'galaxy' },
        null,
      ],
    });
    expect(view.paused).toBe(true);
    expect(view.rules).toEqual([
      {
        id: 'pr_1',
        kind: 'tool',
        tool: 'Bash',
        scope: 'session',
        patterns: ['a'],
        createdAt: 0,
        createdBy: '',
        uses: 0,
      },
    ]);
    expect(normalizePermissions(null)).toEqual({ rules: [], paused: false });
  });

  it('speaks Vietnamese', async () => {
    try {
      await useLanguage('vi');
      expect(quickChoices(bash).map((c) => c.label)).toEqual([
        'Trong phiên này',
        'Trong shop (mọi phiên)',
        'Mọi nơi trên máy Mac này',
      ]);
      expect(quickChoices(run)[2].label).toBe('Mọi lệnh trên MacBook');
    } finally {
      await useLanguage('en');
    }
  });
});

describe('a request a permission allowed', () => {
  it('says so, with the rule as the server words it', () => {
    const r = {
      id: 'req-1',
      session: 's1',
      outcome: 'allowed' as const,
      at: Date.now(),
      mine: false,
    };
    expect(
      resolutionText({
        ...r,
        by: { kind: 'permission', name: 'commands like cargo test in shop' },
      }),
    ).toBe('Allowed by your permission · commands like cargo test in shop');
    expect(resolutionText({ ...r, by: { kind: 'permission' } })).toBe('Allowed by your permission');
    // On another computer's page too.
    expect(resolutionText({ ...r, by: { kind: 'permission', name: 'x' } }, 'MacBook')).toBe(
      'Allowed by your permission · x',
    );
  });
});

describe('PermissionRulesService', () => {
  let backend: FakeBackend;
  let view: { rules: PermissionRule[]; paused: boolean };

  beforeEach(() => {
    backend = new FakeBackend();
    view = { rules: [rule()], paused: false };
    backend.routes.set('/permissions', (method, body) => {
      if (method === 'PATCH') view = { ...view, paused: (body as { paused: boolean }).paused };
      return { status: 200, body: view };
    });
    useFakeServer(backend);
  });

  afterEach(() => {
    TestBed.inject(RealtimeService).stop();
    setActiveMock(null);
  });

  it('loads while watched, refetches on a `permissions` frame, and applies its answers', async () => {
    const svc = TestBed.inject(PermissionRulesService);
    const release = svc.watch();
    await flush();
    expect(svc.loaded()).toBe(true);
    expect(svc.rules().map((r) => r.id)).toEqual(['pr_000000000001']);

    // Someone saved another one: the frame says so, the list is read again.
    view = { ...view, rules: [...view.rules, rule({ id: 'pr_000000000002', scope: 'session' })] };
    const rt = TestBed.inject(RealtimeService);
    await rt.start();
    backend.last.open();
    backend.last.frame({ t: 'hello', v: 2, me: me(), serverTime: Date.now() });
    await flush();
    const before = backend.calls.filter((c) => c === 'GET /permissions').length;
    backend.last.frame({ t: 'permissions' });
    await flush();
    expect(backend.calls.filter((c) => c === 'GET /permissions').length).toBe(before + 1);
    expect(svc.rules().length).toBe(2);

    await svc.setPaused(true);
    expect(backend.bodyOf('PATCH /permissions')).toEqual({ paused: true });
    expect(svc.paused()).toBe(true);

    backend.routes.set('/permissions/pr_000000000002', (method) =>
      method === 'DELETE'
        ? { status: 200, body: { rules: [rule()], paused: true } }
        : {
            status: 200,
            body: rule({ id: 'pr_000000000002', scope: 'session', patterns: ['npm test'] }),
          },
    );
    const saved = await svc.update('pr_000000000002', { patterns: ['npm test'] });
    expect(saved.patterns).toEqual(['npm test']);
    expect(svc.rules()[1].patterns).toEqual(['npm test']);
    await svc.remove('pr_000000000002');
    expect(svc.rules().map((r) => r.id)).toEqual(['pr_000000000001']);

    // Released: frames no longer read it.
    release();
    const calls = backend.calls.length;
    backend.last.frame({ t: 'permissions' });
    await flush();
    expect(backend.calls.length).toBe(calls);
  });

  it('says why the list could not be read', async () => {
    backend.routes.set('/permissions', () => ({
      status: 500,
      body: { error: { code: 'internal', message: 'disk full' } },
    }));
    const svc = TestBed.inject(PermissionRulesService);
    svc.watch();
    await flush();
    expect(svc.error()).toBe('The server failed: disk full');
    expect(svc.loaded()).toBe(false);
  });
});
