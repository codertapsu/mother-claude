// Dev-only: runs, tasks and team roles on other computers
// (docs/PLAN-MACHINES.md Phase 4) for `?mock=1&mockmachines=1`: a run on
// DESKTOP-PC that goes on and ends while you look, a finished build with an
// artifact, a failed check, one DESKTOP-PC asked this computer for; a
// standing grant; a task done with its branch, one that works and one
// DESKTOP-PC handed here; a team role this computer runs for DESKTOP-PC's
// lead. `GET /api/machines/{runs,tasks,roles,run-grants}` and their actions.
// Never bundled in production builds.

import { MachinesFrame } from '../machines';
import { PendingItem } from '../models';
import { MachineRun, RunGrant } from '../runs';
import { HostedRole, MachineTask, taskEnded } from '../tasks';
import { Fixtures, toPreview } from './fixtures';
import { MOCK_DESKTOP } from './mock-moves';

const S = 1000;
const M = 60 * S;
const H = 60 * M;

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function err(status: number, code: string, message: string): Response {
  return json({ error: { code, message } }, status);
}

const PROJECT = '/Users/marcus/dev/mother-claude';
const CHECKOUT = 'D:\\development\\projects\\mother-claude';

/** What `cargo test` prints, a line at a time, while the running one goes. */
const TEST_LINES = [
  '   Compiling mother-claude v0.11.0 (D:\\development\\projects\\mother-claude\\src-tauri)',
  '    Finished `test` profile [unoptimized + debuginfo] target(s) in 41.07s',
  '     Running unittests src\\lib.rs',
  'running 1163 tests',
  'test machines::federation::tests::sub_frames_carry_only_what_is_set ... ok',
  'test machines::code::tests::cases::a_snapshot_round_trips ... ok',
  'test result: ok. 1163 passed; 0 failed; 0 ignored; finished in 38.21s',
];

export class MockRuns {
  private runs: MachineRun[];
  private grants: RunGrant[];
  private tasks: MachineTask[];
  private roles: HostedRole[];
  private ticker?: ReturnType<typeof setInterval>;

  constructor(
    private readonly opts: {
      desktop: boolean;
      broadcast: (frame: MachinesFrame) => void;
    },
  ) {
    const now = Date.now();
    this.runs = [
      this.run('run_00000000000000a1', {
        command: 'cargo test --workspace',
        program: 'cargo',
        state: 'running',
        startedAt: now - 20 * S,
        tail: TEST_LINES.slice(0, 2).join('\n'),
        outputSize: 180,
        grant: 'rg_00000000a001',
      }),
      this.run('run_00000000000000a2', {
        command: 'npm run tauri:build',
        program: 'npm',
        state: 'done',
        exitCode: 0,
        createdAt: now - 52 * M,
        startedAt: now - 52 * M,
        endedAt: now - 46 * M,
        durationMs: 6 * M + 12 * S,
        tail: 'Finished 2 bundles at:\n    D:\\development\\projects\\mother-claude\\src-tauri\\target\\release\\bundle\\msi\\Mother Claude_0.11.0_x64_en-US.msi',
        outputSize: 48_211,
        artifactsGlob: 'src-tauri/target/release/bundle/msi/*.msi',
        artifacts: [
          {
            path: 'src-tauri/target/release/bundle/msi/Mother Claude_0.11.0_x64_en-US.msi',
            size: 9_412_608,
          },
        ],
        approvedBy: 'desktop',
      }),
      this.run('run_00000000000000a3', {
        command: 'cargo clippy --all-targets -- -D warnings',
        program: 'cargo',
        state: 'failed',
        exitCode: 101,
        createdAt: now - 3 * H,
        startedAt: now - 3 * H,
        endedAt: now - 3 * H + 95 * S,
        durationMs: 95 * S,
        tail: 'error: this `if` has identical blocks\n  --> src\\machines\\shell.rs:212:9\nerror: could not compile `mother-claude` (lib) due to 1 previous error',
        outputSize: 2_904,
        approvedBy: 'device Phone',
      }),
      this.run('run_00000000000000a4', {
        direction: 'in',
        os: 'macos',
        shell: 'zsh',
        command: 'xcodebuild -scheme MotherClaude -destination "platform=iOS Simulator" test',
        program: 'xcodebuild',
        state: 'done',
        exitCode: 0,
        createdAt: now - 5 * H,
        startedAt: now - 5 * H,
        endedAt: now - 5 * H + 4 * M,
        durationMs: 4 * M + 2 * S,
        folder:
          '/Users/marcus/Library/Application Support/dev.motherclaude.desktop/machines/work/wk_3c9e0d1f2a4b5c6d',
        checkout: PROJECT,
        project: 'D:\\development\\projects\\mother-claude',
        tail: '** TEST SUCCEEDED **',
        outputSize: 112_455,
        approvedBy: 'machine DESKTOP-PC',
      }),
    ];
    this.grants = [
      {
        id: 'rg_00000000a001',
        project: PROJECT,
        projectName: 'mother-claude',
        machine: MOCK_DESKTOP,
        machineName: 'DESKTOP-PC',
        prefix: 'cargo test',
        createdAt: now - 2 * 24 * H,
        createdBy: 'desktop',
        usedAt: now - 20 * S,
        uses: 14,
      },
    ];
    this.tasks = [
      this.task('task_00000000000000b1', {
        task: "Fix the Windows tray icon's DPI scaling: it is blurry at 150 %. Test it with the tray's own build.",
        firstLine: "Fix the Windows tray icon's DPI scaling: it is blurry at 150 %.",
        state: 'done',
        createdAt: now - 3 * H,
        startedAt: now - 3 * H,
        endedAt: now - 2 * H,
        turns: 3,
        summary:
          'The tray now loads the 32 px and 48 px icons and picks by the monitor’s DPI; tested at 100 %, 150 % and 200 %.',
        branchName: 'mother-claude/DESKTOP-PC/task_00000000000000b1',
        tip: 'c0ffee1234567890abcdef1234567890abcdef12',
        commits: 2,
        files: 3,
        results: 2,
        landed: 2,
        idleStoppedAt: now - 90 * M,
      }),
      this.task('task_00000000000000b2', {
        task: 'Make the installer sign the MSI with the code-signing certificate in the Windows store.',
        firstLine:
          'Make the installer sign the MSI with the code-signing certificate in the Windows store.',
        state: 'running',
        createdAt: now - 6 * M,
        startedAt: now - 6 * M,
        turns: 1,
      }),
      this.task('task_00000000000000b3', {
        direction: 'in',
        os: 'macos',
        task: 'Run the iOS simulator tests and fix what fails.',
        firstLine: 'Run the iOS simulator tests and fix what fails.',
        state: 'done',
        createdAt: now - 26 * H,
        startedAt: now - 26 * H,
        endedAt: now - 25 * H,
        turns: 2,
        project: CHECKOUT,
        checkout: PROJECT,
        folder:
          '/Users/marcus/Library/Application Support/dev.motherclaude.desktop/machines/work/tasks/task_00000000000000b3/mother-claude',
        summary: 'Two tests failed on the simulator’s locale; both pass now.',
        commits: 1,
        files: 2,
        results: 1,
        landed: 0,
      }),
    ];
    this.roles = [
      {
        binding: 'rb_00000000000000c1',
        lead: MOCK_DESKTOP,
        leadName: 'DESKTOP-PC',
        team: 'tm_release',
        teamLabel: 'Release 0.11',
        role: 'ios',
        roleTitle: 'iOS',
        project: CHECKOUT,
        projectName: 'mother-claude',
        checkout: PROJECT,
        folder:
          '/Users/marcus/Library/Application Support/dev.motherclaude.desktop/machines/work/roles/rb_00000000000000c1/mother-claude',
        session: 'r-role-ios',
        state: 'running',
        error: null,
        permissionMode: 'acceptEdits',
        model: 'claude-opus-4-7',
        accountName: 'Default',
        alive: true,
        busy: true,
        denials: 1,
        results: 1,
        keptNote: null,
        startedAt: now - 40 * M,
        stoppedAt: null,
        heardAt: now - 15 * S,
      },
    ];
    this.goOn();
  }

  private run(id: string, over: Partial<MachineRun>): MachineRun {
    const now = Date.now();
    return {
      id,
      direction: 'out',
      machine: MOCK_DESKTOP,
      machineName: 'DESKTOP-PC',
      os: 'windows',
      shell: 'pwsh',
      session: 's-build',
      sessionTitle: 'Build and notarize v0.5.1',
      project: PROJECT,
      projectName: 'mother-claude',
      work: 'wk_1a2b3c4d5e6f7081',
      folder:
        'C:\\Users\\marcus\\AppData\\Roaming\\dev.motherclaude.desktop\\machines\\work\\wk_1a2b3c4d5e6f7081',
      checkout: CHECKOUT,
      cwd: 'src-tauri',
      command: '',
      program: '',
      commandSha256: 'a'.repeat(64),
      dangerous: false,
      timeoutSecs: 1800,
      artifactsGlob: null,
      state: 'done',
      error: null,
      exitCode: null,
      createdAt: now - 30 * S,
      startedAt: null,
      endedAt: null,
      durationMs: null,
      outputSize: 0,
      truncated: false,
      tail: '',
      artifacts: [],
      fetched: [],
      approvedBy: null,
      grant: null,
      unreachable: false,
      updatedAt: now,
      ...over,
    };
  }

  private task(id: string, over: Partial<MachineTask>): MachineTask {
    const now = Date.now();
    return {
      id,
      direction: 'out',
      machine: MOCK_DESKTOP,
      machineName: 'DESKTOP-PC',
      os: 'windows',
      session: 's-build',
      sessionTitle: 'Build and notarize v0.5.1',
      project: PROJECT,
      projectName: 'mother-claude',
      cwd: '',
      checkout: CHECKOUT,
      folder: `C:\\Users\\marcus\\AppData\\Roaming\\dev.motherclaude.desktop\\machines\\work\\tasks\\${id}\\mother-claude`,
      task: '',
      firstLine: '',
      permissionMode: 'acceptEdits',
      model: null,
      effort: 'high',
      accountName: 'Default',
      taskSession: `${id.slice(5, 13)}-0000-4000-8000-000000000000`,
      state: 'running',
      error: null,
      createdAt: now,
      startedAt: null,
      endedAt: null,
      turns: 0,
      summary: null,
      background: false,
      base: 'a1b2c3d4e5f60718293a4b5c6d7e8f9012345678',
      branchName: null,
      tip: null,
      commits: 0,
      files: 0,
      uncommitted: false,
      results: 0,
      landed: 0,
      branchNote: null,
      approvedBy: 'desktop',
      dangerous: false,
      unreachable: false,
      goneThere: false,
      idleStoppedAt: null,
      updatedAt: now,
      ...over,
    };
  }

  /** The running run prints a line every few seconds, then ends well. */
  private goOn(): void {
    let line = 2;
    this.ticker = setInterval(() => {
      const r = this.runs.find((x) => x.id === 'run_00000000000000a1');
      if (!r || r.state !== 'running') {
        clearInterval(this.ticker);
        return;
      }
      line += 1;
      r.tail = TEST_LINES.slice(0, line).join('\n');
      r.outputSize = r.tail.length;
      r.updatedAt = Date.now();
      if (line >= TEST_LINES.length) {
        r.state = 'done';
        r.exitCode = 0;
        r.endedAt = Date.now();
        r.durationMs = r.endedAt - (r.startedAt ?? r.createdAt);
        clearInterval(this.ticker);
      }
      this.frame('runs');
    }, 4 * S);
  }

  private frame(what: 'runs' | 'tasks' | 'roles' | 'runGrants'): void {
    this.opts.broadcast({ t: 'machines', what });
  }

  route(path: string, method: string): Response | null {
    if (path === '/machines/runs' && method === 'GET') return json({ runs: this.runs });
    if (path === '/machines/run-grants' && method === 'GET') return json({ grants: this.grants });
    if (path === '/machines/tasks' && method === 'GET') return json({ tasks: this.tasks });
    if (path === '/machines/roles' && method === 'GET') return json({ roles: this.roles });

    const run = /^\/machines\/runs\/(run_[0-9a-f]{16})(\/cancel)?$/.exec(path);
    if (run) {
      const r = this.runs.find((x) => x.id === run[1]);
      if (!r) return err(404, 'run_not_found', 'No such run here.');
      if (!run[2] && method === 'GET') return json(r);
      if (run[2] && method === 'POST') {
        if (['sending', 'queued', 'preparing', 'running'].includes(r.state)) {
          r.state = 'cancelled';
          r.endedAt = Date.now();
          r.durationMs = r.endedAt - (r.startedAt ?? r.createdAt);
          r.updatedAt = r.endedAt;
          this.frame('runs');
        }
        return json(r);
      }
    }
    const grant = /^\/machines\/run-grants\/(rg_[0-9a-f]{12})$/.exec(path);
    if (grant && method === 'DELETE') {
      if (!this.opts.desktop) return err(403, 'desktop_only', 'Only the desktop app can do this.');
      this.grants = this.grants.filter((g) => g.id !== grant[1]);
      this.frame('runGrants');
      return json({ grants: this.grants });
    }
    const task = /^\/machines\/tasks\/(task_[0-9a-f]{16})(\/cancel)?$/.exec(path);
    if (task) {
      const t = this.tasks.find((x) => x.id === task[1]);
      if (!t) return err(404, 'task_not_found', 'No such task here.');
      if (!task[2] && method === 'GET') return json(t);
      if (task[2] && method === 'POST') {
        if (!taskEnded(t.state)) {
          t.state = 'cancelled';
          t.endedAt = Date.now();
          t.updatedAt = t.endedAt;
          this.frame('tasks');
        }
        return json(t);
      }
      if (!task[2] && method === 'DELETE') {
        if (!taskEnded(t.state)) {
          return err(409, 'task_running', 'It still works: cancel it first.');
        }
        this.tasks = this.tasks.filter((x) => x.id !== t.id);
        this.frame('tasks');
        return json({ removed: true, there: 'removed' });
      }
    }
    const role = /^\/machines\/roles\/(rb_[0-9a-f]{16})(\/stop)?$/.exec(path);
    if (role) {
      const r = this.roles.find((x) => x.binding === role[1]);
      if (!r) return err(404, 'role_not_found', 'No such team role here.');
      if (role[2] && method === 'POST') {
        r.state = 'stopped';
        r.alive = false;
        r.busy = false;
        r.stoppedAt = Date.now();
        this.frame('roles');
        return json(r);
      }
      if (!role[2] && method === 'DELETE') {
        this.roles = this.roles.filter((x) => x.binding !== r.binding);
        this.frame('roles');
        return json({ removed: true });
      }
    }
    return null;
  }
}

/**
 * `?mockmachines=1`: "Build and notarize v0.5.1" asks to run a command on
 * DESKTOP-PC and to hand it a task — the approval cards of Phase 4, each
 * with *Allow for…* (the run's suggests `npm run`) — and the run it has
 * going there is one of its background tasks.
 */
export function addMachineRequests(fx: Fixtures, now: number): void {
  const s = fx.sessions.find((x) => x.id === 's-build');
  if (!s) return;
  const there = {
    machine: MOCK_DESKTOP,
    machineName: 'DESKTOP-PC',
    os: 'windows',
    checkout: CHECKOUT,
    project: PROJECT,
    projectName: 'mother-claude',
  };
  /** What Mother Claude can remember of the answers (*Allow for…*). */
  const offer = {
    session: s.id,
    sessionName: s.name,
    project: PROJECT,
    projectName: 'mother-claude',
    machine: MOCK_DESKTOP,
    machineName: 'DESKTOP-PC',
  };
  const items: PendingItem[] = [
    {
      id: 'req-machine-run',
      sessionId: s.id,
      kind: 'permission',
      source: 'machine',
      toolName: 'machine_run',
      title: 'Run on DESKTOP-PC: npm run tauri:build -- --bundles msi',
      detail: `In ${CHECKOUT} › src-tauri on DESKTOP-PC (PowerShell 7), in a work folder at this session's code.`,
      input: {
        ...there,
        shell: 'pwsh',
        shellLabel: 'PowerShell 7',
        folder:
          'C:\\Users\\marcus\\AppData\\Roaming\\dev.motherclaude.desktop\\machines\\work\\wk_1a2b3c4d5e6f7081',
        cwd: 'src-tauri',
        command: 'npm run tauri:build -- --bundles msi',
        timeoutSecs: 3600,
        artifacts: 'src-tauri/target/release/bundle/msi/*.msi',
      },
      suggestions: [],
      dangerous: false,
      answerable: true,
      createdAt: now - 40 * S,
      grant: { kind: 'run', patterns: 'command', suggested: ['npm run'], ...offer },
    },
    {
      id: 'req-machine-task',
      sessionId: s.id,
      kind: 'permission',
      source: 'machine',
      toolName: 'machine_task',
      title:
        'Hand a task to DESKTOP-PC: Check the MSI installs and starts on a clean Windows user.',
      detail: `In its own worktree of ${CHECKOUT} on DESKTOP-PC, at this session's code.`,
      input: {
        ...there,
        kind: 'task',
        cwd: '',
        task: 'Check the MSI installs and starts on a clean Windows user. Report what the installer shows, and fix what fails.',
        firstLine: 'Check the MSI installs and starts on a clean Windows user.',
        permissionMode: 'acceptEdits',
        model: null,
        effort: 'high',
        account: 'Default',
      },
      suggestions: [],
      dangerous: false,
      answerable: true,
      createdAt: now - 20 * S,
      grant: { kind: 'task', ...offer },
    },
  ];
  fx.pendingItems.push(...items);
  s.pending = [...(s.pending ?? []), ...items.map(toPreview)];
  s.status = 'waiting';
  s.waitingFor = 'permission prompt';
  fx.tasks[s.id] = [
    ...(fx.tasks[s.id] ?? []),
    {
      id: 'run_00000000000000a1',
      kind: 'machine',
      label: 'cargo test --workspace on DESKTOP-PC',
      status: 'running',
      startedAt: now - 20 * S,
      outputAvailable: true,
    },
  ];
}
