import { ComponentFixture, TestBed } from '@angular/core/testing';
import { ActivatedRoute, convertToParamMap, provideRouter } from '@angular/router';

import { FakeBackend, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { RoleView, TeamTask, TeamView } from '../../core/teams';
import { ViewportService } from '../../core/viewport.service';
import { TeamComponent } from './team.component';

const role = (over: Partial<RoleView>): RoleView => ({
  id: 'frontend',
  title: 'Frontend',
  lead: false,
  state: 'idle',
  sessionName: 'shop/frontend',
  foreign: false,
  permissionMode: 'acceptEdits',
  worktree: true,
  worktreePath: '/w/shop/.claude/worktrees/frontend',
  paths: ['src/app/**'],
  readOnly: false,
  budgetApplied: false,
  autostart: false,
  classes: [],
  queuedTasks: 0,
  guardDenials: 0,
  ...over,
});

const team: TeamView = {
  id: 'tm_0123456789ab',
  project: '/w/shop',
  label: 'shop',
  templateId: 'software-delivery',
  templateName: 'Software delivery',
  account: 'default',
  stopped: false,
  createdAt: 1,
  integration: {},
  idleStopMinutes: 30,
  reviews: true,
  roles: [
    role({
      id: 'lead',
      title: 'Lead',
      lead: true,
      sessionId: 'b2c3d4e5-0000-4000-8000-00000000a001',
      sessionName: 'shop/lead',
      worktree: false,
      model: 'opus',
      effort: 'high',
    }),
    role({
      id: 'frontend',
      state: 'running',
      sessionId: 'b2c3d4e5-0000-4000-8000-00000000a002',
      currentTask: 3,
      guardDenials: 2,
      budgetUsd: 5,
    }),
    role({
      id: 'backend',
      title: 'Backend',
      state: 'waiting',
      sessionName: 'shop/backend',
      waitReasons: ['3 role sessions run already (the limit is 3)'],
      waitingSince: Date.now() - 60_000,
    }),
  ],
  counts: { todo: 1, in_progress: 1, review: 1 },
};

const task = (id: number, status: TeamTask['status'], over: Partial<TeamTask> = {}): TeamTask => ({
  id,
  title: `Task ${id}`,
  goal: 'Do it',
  role: 'frontend',
  priority: 2,
  status,
  escalations: 0,
  createdAt: id,
  updatedAt: id,
  createdBy: 'lead',
  ...over,
});

describe('TeamComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<TeamComponent>;
  let tasks: TeamTask[];

  beforeEach(() => {
    backend = new FakeBackend();
    tasks = [
      task(1, 'todo', { role: 'backend', priority: 1 }),
      task(2, 'review', { result: { summary: 'Form done', branch: 'worktree-frontend' } }),
      task(3, 'in_progress', {
        claimedBy: 'b2c3d4e5-0000-4000-8000-00000000a002',
        blocked: 'Which API?',
      }),
    ];
    useFakeServer(backend, [
      provideRouter([]),
      {
        provide: ActivatedRoute,
        useValue: { snapshot: { paramMap: convertToParamMap({ id: team.id }) } },
      },
    ]);
    backend.routes.set(`/teams/${team.id}`, () => ({ status: 200, body: team }));
    backend.routes.set(`/teams/${team.id}/tasks`, () => ({ status: 200, body: { tasks } }));
    backend.routes.set(`/teams/${team.id}/tasks/2`, (method, body) => ({
      status: 200,
      body: { ...tasks[1], ...(body as object), ...(method === 'PATCH' ? {} : {}) },
    }));
  });

  afterEach(() => setActiveMock(null));

  const render = async (phone = false) => {
    TestBed.inject(ViewportService).phone.set(phone);
    fixture = TestBed.createComponent(TeamComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };

  it('shows each role with its state, session, model and why it waits', async () => {
    const el = await render();
    const roles = [...el.querySelectorAll('.role')].map((r) => r.textContent ?? '');
    expect(roles[0]).toContain('lead');
    expect(roles[0]).toContain('opus @ high');
    expect(roles[1]).toContain('Running');
    expect(roles[1]).toContain('On task #3');
    expect(roles[1]).toContain('2 guard denials');
    expect(roles[1]).toContain('$5 (plan)');
    expect(roles[2]).toContain('Waiting: 3 role sessions run already (the limit is 3)');
    const link = el.querySelector('.role a.mono') as HTMLAnchorElement;
    expect(link.getAttribute('href')).toBe('/session/b2c3d4e5-0000-4000-8000-00000000a001');
    // A waiting role can be started by hand; a running one stopped.
    expect(roles[2]).toContain('Start');
    expect(roles[1]).toContain('Stop');
  });

  it('lays the board out in columns on a wide screen, by status on a phone', async () => {
    let el = await render();
    const columns = [...el.querySelectorAll('.column')];
    expect(columns.length).toBe(7);
    expect(columns[0].textContent).toContain('Task 1');
    expect(columns[3].textContent).toContain('Task 2');
    expect(columns[2].textContent).toContain('blocked');
    fixture.destroy();
    el = await render(true);
    expect(el.querySelectorAll('.column').length).toBe(0);
    const pick = el.querySelector('.status-pick select') as HTMLSelectElement;
    expect(pick.options[0].textContent).toContain('To do (1)');
    pick.value = 'review';
    pick.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    expect(el.querySelector('.task')?.textContent).toContain('Task 2');
  });

  it('reprioritises and accepts a task from its sheet', async () => {
    const el = await render(true);
    const pick = el.querySelector('.status-pick select') as HTMLSelectElement;
    pick.value = 'review';
    pick.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    (el.querySelector('.task') as HTMLButtonElement).click();
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const sheet = el.querySelector('mc-task-sheet') as HTMLElement;
    expect(sheet.textContent).toContain('Form done');
    const priority = sheet.querySelector('select') as HTMLSelectElement;
    priority.value = '0';
    priority.dispatchEvent(new Event('change'));
    await flush();
    expect(backend.bodyOf(`PATCH /teams/${team.id}/tasks/2`)).toEqual({ priority: 0 });
    fixture.detectChanges();
    const accept = [...sheet.querySelectorAll('button')].find(
      (b) => b.textContent?.trim() === 'Accept',
    );
    accept?.click();
    await flush();
    expect(backend.bodyOf(`PATCH /teams/${team.id}/tasks/2`)).toEqual({ status: 'done' });
  });
});
