import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import {
  FakeBackend,
  asDesktop,
  asPhoneOf,
  flush,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { PendingItem } from '../../core/models';
import { MachineTask } from '../../core/tasks';
import {
  PendingCardComponent,
  pendingTitle,
  taskDetail,
} from '../../shared/pending-card.component';
import { RunsCardComponent } from './runs-card.component';
import { PEER_ID } from './machines.testing';

const SESSION_THERE = 'b0ee0000-1111-4222-8333-444455556666';

function task(over: Partial<MachineTask> = {}): MachineTask {
  return {
    id: 'task_0123456789abcdef',
    direction: 'out',
    machine: PEER_ID,
    machineName: 'DESKTOP-PC',
    os: 'windows',
    session: 'a0ee0000-1111-4222-8333-444455556666',
    sessionTitle: 'Windows build',
    project: '/Users/you/dev/app',
    projectName: 'app',
    cwd: '',
    checkout: 'D:\\dev\\app',
    folder: 'C:\\data\\machines\\work\\tasks\\task_0123456789abcdef\\app',
    task: 'Make the build work on Windows.\nThen run the tests.',
    firstLine: 'Make the build work on Windows.',
    permissionMode: 'default',
    model: null,
    effort: null,
    accountName: 'Default',
    taskSession: SESSION_THERE,
    state: 'done',
    error: null,
    createdAt: Date.now() - 600_000,
    startedAt: Date.now() - 590_000,
    endedAt: Date.now() - 60_000,
    turns: 1,
    summary: 'Added build.ps1 and fixed the path separators.',
    background: false,
    base: '0123456789abcdef0123456789abcdef01234567',
    branchName: 'mother-claude/DESKTOP-PC/task_0123456789abcdef',
    tip: 'a'.repeat(40),
    commits: 2,
    files: 3,
    uncommitted: true,
    results: 1,
    landed: 1,
    branchNote: null,
    approvedBy: 'Mac',
    dangerous: false,
    unreachable: false,
    goneThere: false,
    updatedAt: Date.now() - 60_000,
    ...over,
  };
}

describe('RunsCardComponent: tasks handed to another computer', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<RunsCardComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const buttons = (el: Element) => [...el.querySelectorAll<HTMLButtonElement>('button')];
  const button = (el: Element, label: string) => buttons(el).find((b) => text(b).startsWith(label));
  const settle = async () => {
    for (let i = 0; i < 3; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const render = async (tasks: MachineTask[]) => {
    backend.routes.set('/machines/runs', () => ({ status: 200, body: { runs: [] } }));
    backend.routes.set('/machines/run-grants', () => ({ status: 200, body: { grants: [] } }));
    backend.routes.set('/machines/tasks', () => ({ status: 200, body: { tasks } }));
    fixture = TestBed.createComponent(RunsCardComponent);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const rows = (el: HTMLElement) => [...el.querySelectorAll('ul.tasks > li')];

  it('lists tasks both ways: state, the branch, what it changed, its session and summary', async () => {
    asDesktop({ hostOs: 'macos' });
    const running = task({
      id: 'task_1111111111111111',
      state: 'waiting',
      branchName: null,
      tip: null,
      results: 0,
      landed: 0,
      summary: null,
      endedAt: null,
    });
    const taken = task({
      id: 'task_2222222222222222',
      direction: 'in',
      machineName: 'MacBook',
      firstLine: 'Fix the installer.',
      state: 'done',
      branchName: null,
      tip: null,
      results: 0,
      summary: 'Nothing needed changing.',
    });
    const el = await render([running, task(), taken]);
    const r = rows(el);
    expect(r.length).toBe(3);
    // Waiting for an approval there, with Cancel.
    expect(text(r[0])).toContain('To DESKTOP-PC');
    expect(text(r[0])).toContain('Make the build work on Windows.');
    expect(text(r[0])).toContain('Waiting for an answer');
    expect(button(r[0], 'Cancel')).toBeTruthy();
    expect(button(r[0], 'Remove')).toBeUndefined();
    // Done: its branch and what it changed; Remove, not Cancel.
    expect(text(r[1])).toContain('Done');
    expect(text(r[1])).toContain('mother-claude/DESKTOP-PC/task_0123456789abcdef');
    expect(text(r[1])).toContain('2 commits and uncommitted changes · 3 files');
    expect(button(r[1], 'Cancel')).toBeUndefined();
    expect(button(r[1], 'Remove')).toBeTruthy();
    // Its session opens on the computer it runs on.
    const link = r[1].querySelector<HTMLAnchorElement>('a.btn');
    expect(text(link)).toBe('Open its session');
    expect(link?.getAttribute('href')).toBe(`/session/${SESSION_THERE}/on/${PEER_ID}`);
    // One another computer handed here: its session is this computer's.
    expect(text(r[2])).toContain('From MacBook, on this');
    expect(text(r[2])).toContain('Changed nothing: no branch');
    expect(r[2].querySelector('a.btn')?.getAttribute('href')).toBe(`/session/${SESSION_THERE}`);
    // The summary on demand.
    button(r[1], 'Summary')!.click();
    fixture.detectChanges();
    expect(text(r[1].querySelector('pre.out'))).toBe(
      'Added build.ps1 and fixed the path separators.',
    );
    button(r[1], 'Hide summary')!.click();
    fixture.detectChanges();
    expect(r[1].querySelector('pre.out')).toBeNull();
  });

  it('cancels a running task and removes an ended one, twice to be sure', async () => {
    asDesktop({ hostOs: 'macos' });
    const running = task({ id: 'task_1111111111111111', state: 'running', endedAt: null });
    const done = task();
    const el = await render([running, done]);
    backend.routes.set(`/machines/tasks/${running.id}/cancel`, () => ({
      status: 200,
      body: { ...running, state: 'cancelled', endedAt: Date.now() },
    }));
    button(rows(el)[0], 'Cancel')!.click();
    await settle();
    expect(backend.calls).toContain(`POST /machines/tasks/${running.id}/cancel`);
    expect(text(rows(el)[0])).toContain('Cancelled');
    backend.routes.set(`/machines/tasks/${done.id}`, () => ({
      status: 200,
      body: { removed: true, there: 'unreachable' },
    }));
    button(rows(el)[1], 'Remove')!.click();
    fixture.detectChanges();
    expect(backend.calls).not.toContain(`DELETE /machines/tasks/${done.id}`);
    button(rows(el)[1], 'Tap again to remove')!.click();
    await settle();
    expect(backend.calls).toContain(`DELETE /machines/tasks/${done.id}`);
    expect(rows(el).length).toBe(1);
    expect(text(el.querySelector('.notice'))).toContain(
      'DESKTOP-PC removes its worktree once it is 7 days old.',
    );
  });

  it('says when nothing was handed over, and a phone sees tasks too', async () => {
    asPhoneOf('macos');
    let el = await render([]);
    expect(text(el.querySelector('ul.tasks'))).toContain('No task yet.');
    fixture.destroy();
    el = await render([
      task({ branchNote: 'mother-claude/DESKTOP-PC/task_0123456789abcdef is checked out in /x.' }),
    ]);
    expect(text(rows(el)[0])).toContain('is checked out in /x.');
  });
});

describe('PendingCardComponent: a task to hand to another computer', () => {
  let backend: FakeBackend;

  beforeEach(() => {
    backend = new FakeBackend();
    backend.routes.set('/sessions/s1/respond', () => ({ status: 200, body: { ok: true } }));
    useFakeServer(backend, [provideRouter([])]);
  });

  afterEach(() => setActiveMock(null));

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();

  /** A task approval as `machines::work::tasks::source::approve` raises it. */
  const item = (over: Partial<PendingItem> = {}): PendingItem => ({
    id: 'req-task',
    sessionId: 's1',
    kind: 'permission',
    source: 'machine',
    toolName: 'machine_task',
    title: 'Hand a task to DESKTOP-PC: Make the build work on Windows.',
    detail: "In its own worktree of D:\\dev\\app on DESKTOP-PC, at this session's code.",
    input: {
      kind: 'task',
      machine: PEER_ID,
      machineName: 'DESKTOP-PC',
      os: 'windows',
      checkout: 'D:\\dev\\app',
      cwd: '',
      task: 'Make the build work on Windows.\nThen run the tests.',
      firstLine: 'Make the build work on Windows.',
      permissionMode: 'default',
      model: null,
      effort: 'high',
      account: 'Default',
      project: '/Users/me/dev/app',
      projectName: 'app',
    },
    suggestions: [],
    dangerous: false,
    answerable: true,
    createdAt: Date.now(),
    ...over,
  });

  it('names the computer, its folder and the whole task, and offers no always', () => {
    asDesktop({ hostOs: 'macos' });
    const fixture = TestBed.createComponent(PendingCardComponent);
    fixture.componentRef.setInput('item', item());
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(text(el.querySelector('.title'))).toBe('Hand a task to DESKTOP-PC?');
    const body = text(el.querySelector('mc-machine-task-action'));
    expect(body).toContain('DESKTOP-PC');
    expect(body).toContain('D:\\dev\\app');
    expect(body).toContain('Make the build work on Windows. Then run the tests.');
    expect(body).toContain('default');
    expect(body).toContain('high');
    expect(el.querySelector('mc-machine-run-action')).toBeNull();
    expect(text(el)).not.toContain('Always');
    fixture.destroy();
  });

  it('reads its title and detail in the language shown', () => {
    expect(pendingTitle('Hand a task to DESKTOP-PC: Fix it.')).toBe(
      'Hand a task to DESKTOP-PC: Fix it.',
    );
    expect(taskDetail("In its own worktree of D:\\dev\\app on PC, at this session's code.")).toBe(
      'In its own worktree of D:\\dev\\app on PC, at this session’s code.',
    );
    expect(taskDetail('something else')).toBe('something else');
  });
});
