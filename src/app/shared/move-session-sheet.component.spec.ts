import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { MachinesService } from '../core/machines.service';
import {
  FakeBackend,
  FakeSocket,
  RouteReply,
  asDesktop,
  asPhoneOf,
  flush,
  useFakeServer,
} from '../core/mock/fake-server.testing';
import { ownedCaps } from '../core/mock/fixtures';
import { setActiveMock } from '../core/mock/mock-api';
import {
  MOVE_ID,
  PEER_ID,
  moveCheck,
  moveRecord,
  movedTo,
  preflightBlocked,
  preflightOk,
} from '../core/mock/moves.testing';
import { PEER2_ID, goLive, pairedMachines } from '../core/mock/remote.testing';
import { MovePreflight, SessionMoveBody } from '../core/moves';
import { SessionSummary } from '../core/models';
import { RealtimeService } from '../core/realtime.service';
import { machinesView } from '../pages/machines/machines.testing';
import { MoveSessionSheetComponent } from './move-session-sheet.component';

const session = (over: Partial<SessionSummary> = {}): SessionSummary =>
  ({
    id: 's1',
    name: 's1',
    title: 'Fix the login',
    projectName: 'api',
    cwd: '/Users/me/dev/api',
    surface: 'mother-claude',
    owned: true,
    live: true,
    status: 'idle',
    activity: { kind: 'idle', label: 'Idle' },
    usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    messageCount: 3,
    agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
    queued: 0,
    pending: [],
    control: ownedCaps(),
    ...over,
  }) as SessionSummary;

describe('MoveSessionSheetComponent (Continue on another computer)', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<MoveSessionSheetComponent>;
  /** Every preflight's body, in order. */
  let asked: SessionMoveBody[];

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    pairedMachines(true);
    // The view the sheet refreshes is the one it has (`ensureMachines`).
    backend.routes.set('/machines', () => ({
      status: 200,
      body: TestBed.inject(MachinesService).view() ?? machinesView(),
    }));
    asDesktop();
    asked = [];
  });

  afterEach(() => {
    fixture?.destroy();
    TestBed.inject(RealtimeService).stop();
    setActiveMock(null);
  });

  /** The preflight answers `answer(body)`. */
  const preflight = (answer: (b: SessionMoveBody) => MovePreflight | RouteReply) =>
    backend.routes.set('/sessions/s1/move/preflight', (_m, body) => {
      const b = body as SessionMoveBody;
      asked.push(b);
      const a = answer(b);
      return 'status' in a && 'body' in a ? (a as RouteReply) : { status: 200, body: a };
    });
  const settle = async () => {
    for (let i = 0; i < 4; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const open = async (s: SessionSummary, inputs: Record<string, unknown> = {}) => {
    fixture = TestBed.createComponent(MoveSessionSheetComponent);
    fixture.componentRef.setInput('session', s);
    for (const [k, v] of Object.entries(inputs)) fixture.componentRef.setInput(k, v);
    fixture.componentRef.setInput('open', true);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: HTMLElement, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));
  const action = (el: HTMLElement) =>
    el.querySelector<HTMLButtonElement>('.actions .btn.primary') ?? undefined;
  const checks = (el: HTMLElement) =>
    [...el.querySelectorAll('.checks li')].map((li) => `${li.className}: ${text(li)}`);
  /** Each computer offered: "name | its line". */
  const computers = (el: HTMLElement) =>
    [...el.querySelectorAll('[aria-label="Computer"] .opt')].map(
      (o) => `${text(o.querySelector('.oname'))} | ${text(o.querySelector('.ot .small'))}`,
    );
  const started = () => backend.bodyOf('POST /sessions/s1/move');
  const serveStart = () =>
    backend.routes.set('/sessions/s1/move', () => ({ status: 202, body: moveRecord() }));

  it('asks the other computer first, says what goes, and starts with the choices in force', async () => {
    preflight(() =>
      preflightOk({
        warnings: [
          moveCheck('claude_older', 'Claude Code on DESKTOP-PC is older (2.1.322, here 2.1.330).'),
        ],
      }),
    );
    serveStart();
    const el = await open(session());

    // The online computer is picked; the offline one says since when.
    expect(asked[0]).toEqual({ to: PEER_ID });
    const rows = computers(el);
    expect(rows[0]).toContain('DESKTOP-PC');
    expect(rows[1]).toMatch(/^BUILD-BOX \| Offline since .+ — can't now$/);
    expect(
      el.querySelector<HTMLButtonElement>(`[aria-label="Computer"] .opt:nth-child(2)`)?.disabled,
    ).toBe(true);

    expect([...el.querySelectorAll('.kv.what > *')].map(text)).toEqual([
      'Conversation',
      '18 MB, with 3 subagents',
      'Code',
      'feat/login · 6 changed files, 2 new',
    ]);
    expect(text(el.querySelector('.where'))).toContain('D:\\dev\\api');
    expect(checks(el)).toEqual([
      'ok: Default is signed in on DESKTOP-PC',
      'ok: The folder there is the same repository (same remote)',
      'ok: DESKTOP-PC trusts that folder',
      'warning: Recommended: Claude Code on DESKTOP-PC is older (2.1.322, here 2.1.330).',
    ]);
    // Idle: no "when"; keep driving it from here is on by default.
    expect(el.querySelector('[aria-label="When"]')).toBeNull();
    expect(text(el.querySelector('label.check'))).toBe('Keep driving it from here');

    expect(text(action(el))).toBe('Continue on DESKTOP-PC');
    action(el)?.click();
    await settle();
    expect(started()).toEqual({
      to: PEER_ID,
      code: 'bundle',
      apply: 'checkout',
      keepDriving: true,
    });
    // Its steps from then on.
    expect(el.querySelector('mc-move-progress')).not.toBeNull();
    expect(text(el.querySelector('.outcome'))).toBe('Checking with DESKTOP-PC…');
  });

  it('stops the button at a blocker, and asks again with what it offers', async () => {
    preflight((b) =>
      b.apply === 'worktree'
        ? preflightOk({ code: { ...preflightOk().code, apply: 'worktree' } })
        : preflightBlocked([
            moveCheck(
              'checkout_dirty',
              "D:\\dev\\api has changes of its own (3 files). The code can't land in the project folder.",
              { fix: 'Use a separate folder (worktree) there.', offer: 'worktree' },
            ),
          ]),
    );
    serveStart();
    const el = await open(session());
    expect(text(action(el))).toBe('Fix the checks first');
    expect(action(el)?.disabled).toBe(true);
    expect(checks(el)).toContain(
      "blocker: Required: D:\\dev\\api has changes of its own (3 files). The code can't land in the project folder. Use a separate folder (worktree) there. Use a separate folder (worktree)",
    );

    el.querySelector<HTMLButtonElement>('button.offer')?.click();
    await settle();
    expect(asked.at(-1)).toEqual({ to: PEER_ID, apply: 'worktree' });
    expect(checks(el).filter((c) => c.startsWith('blocker'))).toEqual([]);
    expect((el.querySelector('#mv-apply') as HTMLSelectElement).value).toBe('worktree');
    expect(action(el)?.disabled).toBe(false);
    action(el)?.click();
    await settle();
    expect(started()).toEqual({
      to: PEER_ID,
      code: 'bundle',
      apply: 'worktree',
      keepDriving: true,
    });
  });

  it('offers its folders there when it found none, and asks with the one chosen', async () => {
    preflight((b) =>
      b.folder
        ? preflightOk({
            target: {
              ...preflightOk().target!,
              folder: { path: b.folder, how: 'chosen', trusted: true },
            },
          })
        : preflightBlocked(
            [
              moveCheck(
                'folder_elsewhere',
                'DESKTOP-PC has no folder for /Users/me/dev/api: choose its folder there.',
                {
                  offer: 'choose-folder',
                  detail: { candidates: ['D:\\src\\api', 'D:\\work\\api'] },
                },
              ),
            ],
            { target: { ...preflightOk().target!, folder: null, cwd: null } },
          ),
    );
    const el = await open(session());
    button(el, 'Choose its folder on DESKTOP-PC')?.click();
    await settle();
    const options = [...el.querySelectorAll('.chooser .cand')].map(text);
    expect(options).toEqual(['D:\\src\\api', 'D:\\work\\api']);
    el.querySelectorAll<HTMLInputElement>('.chooser .cand input')[1].click();
    fixture.detectChanges();
    button(el, 'Use this folder')?.click();
    await settle();
    expect(asked.at(-1)).toEqual({ to: PEER_ID, folder: 'D:\\work\\api' });
    expect(text(el.querySelector('.where'))).toContain('D:\\work\\api');
    expect(action(el)?.disabled).toBe(false);
  });

  it('moves a session another app runs as a copy, saying so', async () => {
    preflight((b) =>
      b.asCopy
        ? preflightOk({
            asCopy: true,
            session: { ...preflightOk().session, owned: false, foreign: true },
          })
        : preflightBlocked(
            [
              moveCheck(
                'session_foreign',
                "It runs in claude-vscode — Mother Claude can't stop it. A copy can go on on DESKTOP-PC as a new session while this one goes on here, or end it there first.",
                { offer: 'as-copy' },
              ),
            ],
            { session: { ...preflightOk().session, owned: false, foreign: true } },
          ),
    );
    serveStart();
    const el = await open(
      session({ owned: false, surface: 'vscode', control: { ...ownedCaps(), channel: 'hooks' } }),
    );
    expect(asked.map((b) => !!b.asCopy)).toEqual([false, true]);
    expect(text(el.querySelector('.lede'))).toBe(
      "Fix the login runs in VS Code — Mother Claude can't stop it. A copy goes on there as a new session; this one goes on here. Or end it in VS Code first.",
    );
    // A copy leaves this one here: nothing to keep driving.
    expect(el.querySelector('label.check')).toBeNull();
    expect(text(action(el))).toBe('Continue as a copy on DESKTOP-PC');
    action(el)?.click();
    await settle();
    expect(started()).toEqual({
      to: PEER_ID,
      code: 'bundle',
      apply: 'checkout',
      asCopy: true,
      keepDriving: false,
    });
  });

  it('asks after this turn or now while a turn runs, and asks Claude for the recipe when ticked', async () => {
    preflight(() => preflightOk({ recipe: ['npm ci'] }));
    serveStart();
    const el = await open(
      session({ status: 'working', activity: { kind: 'tool', label: 'Running Bash' } }),
    );
    const whens = [...el.querySelectorAll('[aria-label="When"] label')].map(text);
    expect(whens).toEqual(['After this turn — it finishes first', 'Now — interrupts the turn']);
    el.querySelectorAll<HTMLInputElement>('[aria-label="When"] input')[1].click();
    const recipe = [...el.querySelectorAll<HTMLLabelElement>('label.check')].find((l) =>
      text(l).startsWith('On DESKTOP-PC, ask Claude to run:'),
    );
    expect(text(recipe)).toContain('npm ci');
    recipe?.querySelector('input')?.click();
    fixture.detectChanges();
    action(el)?.click();
    await settle();
    expect(started()).toEqual({
      to: PEER_ID,
      code: 'bundle',
      apply: 'checkout',
      keepDriving: true,
      recipe: true,
      when: 'now',
    });
    // Neither changes the checks: asked once.
    expect(asked.length).toBe(1);
  });

  it("offers to carry the folder's trust only to whoever may take dangerous actions", async () => {
    const untrusted = (b: SessionMoveBody) =>
      b.trust
        ? preflightOk({
            target: {
              ...preflightOk().target!,
              folder: { path: 'D:\\dev\\api', how: 'root', trusted: false },
            },
          })
        : preflightBlocked(
            [
              moveCheck('folder_untrusted', "DESKTOP-PC doesn't trust D:\\dev\\api yet.", {
                fix: "Open it once with Claude Code on DESKTOP-PC, or carry the folder's trust (needs the dangerous permission).",
              }),
            ],
            {
              target: {
                ...preflightOk().target!,
                folder: { path: 'D:\\dev\\api', how: 'root', trusted: false },
              },
            },
          );
    preflight(untrusted);
    serveStart();
    let el = await open(session());
    const trust = el.querySelector<HTMLInputElement>('label.trust input');
    expect(text(el.querySelector('label.trust'))).toContain(
      "Carry this folder's trust to DESKTOP-PC",
    );
    trust?.click();
    await settle();
    expect(asked.at(-1)).toEqual({ to: PEER_ID, trust: true });
    action(el)?.click();
    await settle();
    expect(started()).toMatchObject({ trust: true });
    fixture.destroy();

    // A phone without the dangerous permission: no such option.
    asPhoneOf('macos');
    asked = [];
    el = await open(session());
    expect(el.querySelector('label.trust')).toBeNull();
    expect(asked).toEqual([{ to: PEER_ID }]);
    expect(checks(el).some((c) => c.includes("doesn't trust"))).toBe(true);
  });

  it('lists the files git ignores that travel, and leaves them on an untick or without the right', async () => {
    const withIgnored = (b: SessionMoveBody) =>
      preflightOk({
        code: {
          ...preflightOk().code,
          ignored: ['.env.local', 'certs/dev.pem'],
          carryIgnored: b.carryIgnored !== false,
        },
      });
    preflight(withIgnored);
    serveStart();
    let el = await open(session());
    const rows = [...el.querySelectorAll('.kv.what > *')].map(text);
    expect(rows.slice(4, 5)).toEqual(['Not in git']);
    expect(rows[5]).toContain('.env.local · certs/dev.pem');
    expect(rows[5]).toContain('Carry them');
    expect(asked.at(-1)).toEqual({ to: PEER_ID });
    // Unticked: they stay, and the checks are asked again.
    el.querySelector<HTMLInputElement>('.kv.what .ignored input')?.click();
    await settle();
    expect(asked.at(-1)).toEqual({ to: PEER_ID, carryIgnored: false });
    action(el)?.click();
    await settle();
    expect(started()).toMatchObject({ carryIgnored: false });
    fixture.destroy();

    // A phone without the dangerous permission: listed, no tick, nothing
    // asked (they stay, as the server does it then).
    asPhoneOf('macos');
    asked = [];
    el = await open(session());
    const line = text(el.querySelector('.kv.what .ignored'));
    expect(line).toContain('.env.local');
    expect(line).toContain('They stay where they are');
    expect(el.querySelector('.kv.what .ignored input')).toBeNull();
    expect(asked).toEqual([{ to: PEER_ID }]);
  });

  it('shows the checks the start found again', async () => {
    preflight(() => preflightOk());
    backend.routes.set('/sessions/s1/move', () => ({
      status: 409,
      body: {
        error: {
          code: 'move_blocked',
          message: "It can't move yet: DESKTOP-PC runs this session right now.",
          preflight: preflightBlocked([
            moveCheck('session_live_there', 'DESKTOP-PC runs this session right now.', {
              fix: 'Stop it on DESKTOP-PC first.',
            }),
          ]),
        },
      },
    }));
    const el = await open(session());
    action(el)?.click();
    await settle();
    expect(text(el.querySelector('.notice.error'))).toBe(
      "It can't move yet: fix the checks first.",
    );
    expect(checks(el)).toContain(
      'blocker: Required: DESKTOP-PC runs this session right now. Stop it on DESKTOP-PC first.',
    );
    expect(text(action(el))).toBe('Fix the checks first');
  });

  describe('as it runs', () => {
    let socket: FakeSocket;

    beforeEach(async () => {
      socket = await goLive(backend);
      preflight(() => preflightOk());
      serveStart();
    });

    const frame = (m: ReturnType<typeof moveRecord>) => {
      socket.frame({ t: 'move', move: m });
      fixture.detectChanges();
    };
    const lines = (el: HTMLElement) =>
      [...el.querySelectorAll('mc-move-progress li')].map(
        (li) => `${li.getAttribute('data-state')}: ${text(li.querySelector('.label'))}`,
      );
    const startIt = async () => {
      const el = await open(session());
      action(el)?.click();
      await settle();
      return el;
    };

    it('follows the move from its frames, step by step, to where it runs now', async () => {
      const el = await startIt();
      frame(movedTo('sending', { bytesSent: 4 * 1024 * 1024 }));
      expect(lines(el)).toEqual([
        'done: Done: Checked',
        'done: Done: Stopped here',
        'done: Done: Packed — conversation 18 MB, code 2.0 MB',
        'active: Under way: Sending 4.0 MB of 20 MB',
        'todo: Still to come: Placing it on DESKTOP-PC',
        'todo: Still to come: Starting it there',
      ]);
      expect(
        el.querySelector('mc-move-progress [role="progressbar"]')?.getAttribute('aria-valuenow'),
      ).toBe('20');
      expect(text(button(el, 'Close'))).toBe('Close (it goes on)');

      frame(movedTo('resumed', { undoUntil: Date.now() + 7 * 86_400_000 }));
      expect(lines(el).every((l) => l.startsWith('done'))).toBe(true);
      expect(text(el.querySelector('.outcome'))).toBe('Fix the login runs on DESKTOP-PC now.');
      expect(button(el, 'Open it there')).toBeTruthy();
      expect(button(el, 'Cancel the move')).toBeUndefined();
      expect(text(el.querySelector('.until'))).toMatch(/^Possible until .+\.$/);
    });

    it('shows what its record says is worth knowing, as it lands and once it ended', async () => {
      const el = await startIt();
      const warns = () => [...el.querySelectorAll('.warns li')].map((li) => text(li));
      frame(movedTo('sending'));
      expect(el.querySelector('.warns')).toBeNull();
      const landed = {
        warnings: [
          {
            code: 'ignored_replaced',
            message: 'DESKTOP-PC had its own .env.local: kept for Undo move.',
          },
        ],
        code: { ...movedTo('landed').code!, warnings: ['untracked-exec'] },
      };
      frame(movedTo('landed', landed));
      expect(warns()).toEqual([
        'DESKTOP-PC had its own .env.local: kept for Undo move.',
        'New executable files lose their executable bit on Windows: stage them.',
      ]);
      frame(movedTo('resumed', { ...landed, undoUntil: Date.now() + 86_400_000 }));
      expect(warns().length).toBe(2);
      expect(el.querySelector('.warns')?.getAttribute('aria-label')).toBe('Worth knowing');
    });

    it('cancels until it is being placed', async () => {
      backend.routes.set(`/moves/${MOVE_ID}/cancel`, () => ({
        status: 200,
        body: movedTo('freezing'),
      }));
      const el = await startIt();
      frame(movedTo('freezing'));
      button(el, 'Cancel the move')?.click();
      await settle();
      expect(backend.calls).toContain(`POST /moves/${MOVE_ID}/cancel`);
      frame(movedTo('landing'));
      expect(button(el, 'Cancel the move')).toBeUndefined();
      frame(
        movedTo('cancelled', {
          error: { code: 'cancelled', message: 'It was cancelled.', step: 'freezing' },
        }),
      );
      expect(text(el.querySelector('.outcome'))).toBe(
        'The move was cancelled: Fix the login stays on this computer.',
      );
    });

    it('says why it stopped and where, and goes back to the choices with its offer', async () => {
      const el = await startIt();
      frame(
        movedTo('failed', {
          error: {
            code: 'move_code_stopped',
            message:
              "The code can't land here: The target checkout has changes of its own (3 paths).",
            step: 'landing',
            offer: 'worktree',
          },
        }),
      );
      expect(lines(el).at(4)).toBe('failed: Stopped: Placing it on DESKTOP-PC');
      expect(text(el.querySelector('.outcome'))).toBe(
        "Couldn't move Fix the login: The code can't land on DESKTOP-PC: The project folder there has changes of its own (3 files). It stays on this computer, as it was.",
      );
      const before = asked.length;
      button(el, 'Use a separate folder (worktree)')?.click();
      await settle();
      expect(el.querySelector('mc-move-progress')).toBeNull();
      expect(asked.length).toBe(before + 1);
      expect(asked.at(-1)).toEqual({ to: PEER_ID, apply: 'worktree' });
    });

    it('tries again a move placed there that did not start, and undoes one that did', async () => {
      backend.routes.set(`/moves/${MOVE_ID}/retry`, () => ({
        status: 200,
        body: movedTo('resumed', { undoUntil: Date.now() + 86_400_000 }),
      }));
      backend.routes.set(`/moves/${MOVE_ID}/undo`, () => ({
        status: 200,
        body: movedTo('undone', { undoUntil: null }),
      }));
      const el = await startIt();
      frame(movedTo('committed', { resumeError: 'Not logged in · Please run /login' }));
      expect(text(el.querySelector('.outcome'))).toBe(
        "Fix the login is on DESKTOP-PC but didn't start there: Not logged in · Please run /login",
      );
      expect(lines(el).at(5)).toBe("failed: Stopped: Didn't start there");
      button(el, 'Try again')?.click();
      await settle();
      expect(backend.calls).toContain(`POST /moves/${MOVE_ID}/retry`);
      expect(text(el.querySelector('.outcome'))).toBe('Fix the login runs on DESKTOP-PC now.');

      // Undo arms first.
      button(el, 'Undo move')?.click();
      fixture.detectChanges();
      expect(backend.calls).not.toContain(`POST /moves/${MOVE_ID}/undo`);
      button(el, 'Tap again to undo')?.click();
      await settle();
      expect(backend.calls).toContain(`POST /moves/${MOVE_ID}/undo`);
      expect(text(el.querySelector('.outcome'))).toBe(
        'Undone: Fix the login lives on this computer again.',
      );
      expect(button(el, 'Undo move')).toBeUndefined();
    });
  });

  it('brings it back to this computer, started on the one it lives on', async () => {
    const proxy = `/machines/${PEER_ID}/proxy`;
    backend.routes.set(`${proxy}/sessions/s1/move/preflight`, (_m, body) => {
      asked.push(body as SessionMoveBody);
      return {
        status: 200,
        body: preflightOk({
          to: { id: '0'.repeat(32), name: 'MacBook', os: 'macos', online: true },
        }),
      };
    });
    const el = await open(session(), {
      at: PEER_ID,
      to: '0'.repeat(32),
      since: Date.now() - 3_600_000,
    });
    expect(asked).toEqual([{ to: '0'.repeat(32) }]);
    expect(el.querySelector('[aria-label="Computer"]')).toBeNull();
    expect(text(el.querySelector('h3'))).toBe('What comes back');
    expect(text(el.querySelector('.kv.what dd'))).toMatch(
      /^18 MB: what happened on DESKTOP-PC since /,
    );
    expect(checks(el)).toContain('ok: This computer trusts that folder');
    expect(text(action(el))).toBe('Bring back here');
    // Nothing of it ran through this computer's own routes.
    expect(backend.calls.filter((c) => c.startsWith('POST /sessions'))).toEqual([]);
  });

  it('lists the paired computers that can take it', async () => {
    preflight(() => preflightOk());
    TestBed.inject(MachinesService).apply(
      machinesView({
        enabled: true,
        peers: [...TestBed.inject(MachinesService).view()!.peers].map((p) =>
          p.id === PEER2_ID
            ? {
                ...p,
                online: true,
                path: 'relay' as const,
                theirGrants: { ...p.grants, move: { lan: false, internet: false } },
              }
            : p,
        ),
      }),
    );
    const el = await open(session());
    const rows = computers(el);
    expect(rows).toEqual([
      'DESKTOP-PC | Online · Direct (same network)',
      "BUILD-BOX | Doesn't let this computer move sessions there",
    ]);
  });
});
