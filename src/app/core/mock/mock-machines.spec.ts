import { CopiesView } from '../copies';
import { MachinesFrame, MachinesView, SetupView, SyncView } from '../machines';
import { MovePreflight } from '../moves';
import { MachineRun, RunGrantsView } from '../runs';
import { HostedRolesView, MachineTask } from '../tasks';
import { buildFixtures } from './fixtures';
import { MockMachines } from './mock-machines';
import { MOCK_DESKTOP, MockMoves } from './mock-moves';
import { MockRuns, addMachineRequests } from './mock-runs';

/** Calls a mock's route: its status and its JSON. */
async function call(
  route: (path: string, method: string, body?: unknown) => Response | null,
  path: string,
  method = 'GET',
  body?: unknown,
): Promise<{ status: number; body: unknown }> {
  const r = route(path, method, body);
  expect(r, `${method} ${path}`).not.toBeNull();
  return { status: r!.status, body: await r!.json() };
}

describe('the Machines mock (`?mock=1&mockmachines=1`)', () => {
  let frames: MachinesFrame[];

  beforeEach(() => {
    frames = [];
  });

  it('is on, with two computers, an introduction, a revocation and their versions', async () => {
    const m = new MockMachines({ desktop: true, broadcast: (f) => frames.push(f), paired: true });
    const route = m.route.bind(m);
    const view = (await call(route, '/machines')).body as MachinesView;
    expect(view.enabled).toBe(true);
    expect(view.peers.map((p) => [p.name, p.online])).toEqual([
      ['DESKTOP-PC', true],
      ['BUILD-BOX', false],
    ]);
    // Versions and skew: BUILD-BOX runs an older Mother Claude and lacks what
    // it adds; DESKTOP-PC an older Claude Code.
    expect(view.peers[1].skew?.app).toBe('older');
    expect(view.peers[1].skew?.lacks).toContain('sync');
    expect(view.peers[0].skew?.claude).toBe('older');
    const setup = (await call(route, '/machines/setup')).body as SetupView;
    expect(setup.peers[1].checks.find((c) => c.id === 'app')?.fix?.label).toBe(
      'Update Mother Claude',
    );
    // Phase 5: an introduction to answer, a revocation still to deliver.
    expect(view.introductions?.map((i) => [i.fromName, i.other.name, i.state])).toEqual([
      ['DESKTOP-PC', 'STUDIO-MINI', 'offered'],
    ]);
    expect(view.revocations?.[0].pending.map((p) => p.name)).toEqual(['BUILD-BOX']);
    // Code sync, one of its projects in conflict; copies held here.
    const sync = (await call(route, '/machines/sync')).body as SyncView;
    expect(sync.pairs.map((p) => p.status.state)).toEqual(['inSync', 'paused']);
    expect(sync.pairs[1].conflict).toBeTruthy();
    const copies = (await call(route, '/machines/copies')).body as CopiesView;
    expect(copies.held.length).toBeGreaterThan(0);
  });

  it('serves runs, tasks, team roles and standing grants, and their actions', async () => {
    const runs = new MockRuns({ desktop: true, broadcast: (f) => frames.push(f) });
    const route = (path: string, method: string) => runs.route(path, method);
    const list = ((await call(route, '/machines/runs')).body as { runs: MachineRun[] }).runs;
    expect(list.map((r) => [r.direction, r.state])).toEqual([
      ['out', 'running'],
      ['out', 'done'],
      ['out', 'failed'],
      ['in', 'done'],
    ]);
    expect(list[1].artifacts.length).toBe(1);
    const cancelled = (await call(route, `/machines/runs/${list[0].id}/cancel`, 'POST'))
      .body as MachineRun;
    expect(cancelled.state).toBe('cancelled');
    expect(frames).toContainEqual({ t: 'machines', what: 'runs' });

    const grants = (await call(route, '/machines/run-grants')).body as RunGrantsView;
    expect(grants.grants.map((g) => g.prefix)).toEqual(['cargo test']);
    const left = (await call(route, `/machines/run-grants/${grants.grants[0].id}`, 'DELETE'))
      .body as RunGrantsView;
    expect(left.grants).toEqual([]);

    const tasks = ((await call(route, '/machines/tasks')).body as { tasks: MachineTask[] }).tasks;
    expect(tasks.map((t) => [t.direction, t.state])).toEqual([
      ['out', 'done'],
      ['out', 'running'],
      ['in', 'done'],
    ]);
    expect(tasks[0].branchName).toMatch(/^mother-claude\/DESKTOP-PC\//);
    // A task that still works is not removed; cancelled, it is.
    expect((await call(route, `/machines/tasks/${tasks[1].id}`, 'DELETE')).status).toBe(409);
    await call(route, `/machines/tasks/${tasks[1].id}/cancel`, 'POST');
    expect((await call(route, `/machines/tasks/${tasks[1].id}`, 'DELETE')).body).toEqual({
      removed: true,
      there: 'removed',
    });

    const roles = (await call(route, '/machines/roles')).body as HostedRolesView;
    expect(roles.roles.map((r) => [r.leadName, r.roleTitle, r.state])).toEqual([
      ['DESKTOP-PC', 'iOS', 'running'],
    ]);
    await call(route, `/machines/roles/${roles.roles[0].binding}/stop`, 'POST');
    expect(((await call(route, '/machines/roles')).body as HostedRolesView).roles[0].state).toBe(
      'stopped',
    );

    // A phone may read them, not drop a standing grant.
    const phone = new MockRuns({ desktop: false, broadcast: () => undefined });
    const g = (
      (await call((p, m) => phone.route(p, m), '/machines/run-grants')).body as RunGrantsView
    ).grants[0];
    expect(
      (await call((p, m) => phone.route(p, m), `/machines/run-grants/${g.id}`, 'DELETE')).status,
    ).toBe(403);
  });

  it('asks for a run and a task on DESKTOP-PC from a session, its run one of its tasks', () => {
    const fx = buildFixtures(Date.now());
    addMachineRequests(fx, Date.now());
    const asks = fx.pendingItems.filter((p) => p.source === 'machine');
    expect(asks.map((p) => p.toolName)).toEqual(['machine_run', 'machine_task']);
    const s = fx.sessions.find((x) => x.id === 's-build')!;
    expect(s.pending?.map((p) => p.id)).toEqual(['req-machine-run', 'req-machine-task']);
    expect(asks[0].suggestions[0].kind).toBe('machineRunGrant');
    expect(fx.tasks['s-build'].some((t) => t.kind === 'machine')).toBe(true);
  });

  it("names the files git ignores that travel in a move's preflight", async () => {
    const fx = buildFixtures(Date.now());
    const moves = new MockMoves({
      sessions: () => fx.sessions,
      patchSession: () => undefined,
      addSession: () => undefined,
      broadcast: () => undefined,
    });
    const route = (path: string, method: string, body?: unknown) => moves.route(path, method, body);
    const pre = (
      await call(route, '/sessions/s-haiku/move/preflight', 'POST', { to: MOCK_DESKTOP })
    ).body as MovePreflight;
    expect(pre.code.ignored).toEqual(['.env.local', 'config/dev.secrets.json']);
    expect(pre.code.ignoredBytes).toBeGreaterThan(0);
    expect(pre.code.carryIgnored).toBe(true);
    const left = (
      await call(route, '/sessions/s-haiku/move/preflight', 'POST', {
        to: MOCK_DESKTOP,
        carryIgnored: false,
      })
    ).body as MovePreflight;
    expect(left.code.carryIgnored).toBe(false);
  });
});
