import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { MachineRun } from '../../core/runs';
import { RunsCardComponent } from './runs-card.component';
import { PEER_ID } from './machines.testing';

function run(over: Partial<MachineRun> = {}): MachineRun {
  return {
    id: 'run_0123456789abcdef',
    direction: 'out',
    machine: PEER_ID,
    machineName: 'DESKTOP-PC',
    os: 'windows',
    shell: 'pwsh',
    session: 'a0ee0000-1111-4222-8333-444455556666',
    sessionTitle: 'Windows build',
    project: '/Users/you/dev/app',
    projectName: 'app',
    work: 'wk_0123456789abcdef',
    folder: 'C:\\data\\machines\\work\\wk_0123456789abcdef',
    checkout: 'D:\\dev\\app',
    cwd: '',
    command: 'cargo test --lib',
    program: 'cargo',
    commandSha256: 'a'.repeat(64),
    dangerous: false,
    timeoutSecs: 1800,
    artifactsGlob: null,
    state: 'done',
    error: null,
    exitCode: 0,
    createdAt: Date.now() - 120_000,
    startedAt: Date.now() - 115_000,
    endedAt: Date.now() - 60_000,
    durationMs: 55_000,
    outputSize: 2048,
    truncated: false,
    tail: 'test result: ok. 1106 passed',
    artifacts: [],
    fetched: [],
    approvedBy: 'Mac',
    grant: null,
    unreachable: false,
    updatedAt: Date.now() - 60_000,
    ...over,
  };
}

describe('RunsCardComponent', () => {
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
  const render = async (runs: MachineRun[]) => {
    backend.routes.set('/machines/runs', () => ({ status: 200, body: { runs } }));
    fixture = TestBed.createComponent(RunsCardComponent);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };

  it('lists runs both ways with how each stands, and cancels a running one', async () => {
    asDesktop({ hostOs: 'macos' });
    const running = run({
      id: 'run_1111111111111111',
      state: 'running',
      exitCode: null,
      endedAt: null,
      durationMs: null,
      tail: 'Compiling mother-claude',
    });
    const failed = run({
      id: 'run_2222222222222222',
      direction: 'in',
      machineName: 'MacBook',
      command: 'npm test',
      state: 'failed',
      exitCode: 1,
      durationMs: 3_000,
    });
    const el = await render([running, run(), failed]);
    const rows = [...el.querySelectorAll('ul.list > li')];
    expect(rows.length).toBe(3);
    expect(text(rows[0])).toContain('On DESKTOP-PC');
    expect(text(rows[0])).toContain('cargo test --lib');
    expect(text(rows[0])).toContain('Running');
    expect(text(rows[1])).toContain('Done');
    expect(text(rows[1])).toContain('exit code 0');
    expect(text(rows[1])).toContain('55s');
    expect(text(rows[2])).toContain('From MacBook, on this');
    expect(text(rows[2])).toContain('Failed');
    expect(text(rows[2])).toContain('exit code 1');
    // Only the running one can be cancelled.
    expect(button(rows[0], 'Cancel')).toBeTruthy();
    expect(button(rows[1], 'Cancel')).toBeUndefined();
    backend.routes.set('/machines/runs/run_1111111111111111/cancel', () => ({
      status: 200,
      body: { ...running, state: 'cancelled', endedAt: Date.now() },
    }));
    button(rows[0], 'Cancel')!.click();
    await settle();
    expect(backend.calls).toContain('POST /machines/runs/run_1111111111111111/cancel');
    expect(text(el.querySelectorAll('ul.list > li')[0])).toContain('Cancelled');
  });

  it('shows a run’s output and the files it made', async () => {
    asDesktop({ hostOs: 'macos' });
    const done = run({
      tail: 'short',
      artifacts: [{ path: 'target/release/app.exe', size: 3 << 20 }],
    });
    backend.routes.set(`/machines/runs/${done.id}`, () => ({
      status: 200,
      body: { ...done, tail: 'the whole tail: test result: ok' },
    }));
    const el = await render([done]);
    button(el, 'Output')!.click();
    await settle();
    expect(text(el.querySelector('pre.out'))).toBe('the whole tail: test result: ok');
    expect(text(el)).toContain('target/release/app.exe');
    expect(text(el)).toContain('3.0 MB');
    button(el, 'Hide output')!.click();
    fixture.detectChanges();
    expect(el.querySelector('pre.out')).toBeNull();
  });

  it('sends the commands allowed without asking to Permissions', async () => {
    asDesktop({ hostOs: 'macos' });
    const el = await render([]);
    expect(text(el)).toContain('No run yet.');
    const line = el.querySelector('.allowed');
    expect(text(line)).toBe(
      'Commands and tasks your sessions may run on another computer without asking are kept in Permissions. Open Permissions',
    );
    expect(line?.querySelector('a')?.getAttribute('href')).toBe('/permissions?on=others');
    // The old standing grants are not asked for any more.
    expect(backend.calls.some((c) => c.includes('run-grants'))).toBe(false);
    // Only beside the commands.
    fixture.destroy();
    fixture = TestBed.createComponent(RunsCardComponent);
    fixture.componentRef.setInput('part', 'tasks');
    fixture.detectChanges();
    await settle();
    expect((fixture.nativeElement as HTMLElement).querySelector('.allowed')).toBeNull();
  });
});
