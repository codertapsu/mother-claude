import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { ApiError, MachineApiError } from './api.service';
import { hostOsSignal, useLanguage } from './i18n/i18n';
import { MachinesService } from './machines.service';
import { FakeBackend, useFakeServer } from './mock/fake-server.testing';
import { setActiveMock } from './mock/mock-api';
import { MOVE_ID, moveCheck, movedTo, moveRecord, PEER_ID, SELF_ID } from './mock/moves.testing';
import { pairedMachines } from './mock/remote.testing';
import {
  canCancel,
  canRetry,
  canUndo,
  normalizeMove,
  normalizeMoves,
  sessionInTurn,
} from './moves';
import {
  failureText,
  moveErrorText,
  moveSummary,
  moveText,
  moveWarnings,
  progressLines,
} from './moves-text';
import { MovesService } from './moves.service';
import { NoticeService } from './notice.service';

describe('moves: the records', () => {
  it('takes a record as the server sends it, every field tolerant, and nothing else', () => {
    const m = normalizeMove({ id: MOVE_ID, state: 'sending', role: 'out', bytesSent: 'x' });
    expect(m).toMatchObject({
      id: MOVE_ID,
      state: 'sending',
      role: 'out',
      bytesSent: 0,
      steps: [],
      files: [],
      warnings: [],
      finished: false,
      code: null,
    });
    expect(normalizeMove({ id: 'mv_nothex', state: 'sending' })).toBeNull();
    expect(normalizeMove({ id: MOVE_ID, state: 'teleporting' })).toBeNull();
    expect(normalizeMove(null)).toBeNull();
    const list = normalizeMoves({
      moves: [
        moveRecord({ id: 'mv_0000000000000001', createdAt: 1 }),
        { id: 'broken' },
        moveRecord({ id: 'mv_0000000000000002', createdAt: 2 }),
      ],
    });
    expect(list.map((x) => x.id)).toEqual(['mv_0000000000000002', 'mv_0000000000000001']);
  });

  it('says what can be done with a move, and until when', () => {
    expect(canCancel(movedTo('sending'))).toBe(true);
    expect(canCancel(movedTo('landing'))).toBe(false);
    expect(canCancel(movedTo('sending', { role: 'in' }))).toBe(false);
    expect(canRetry(movedTo('committed', { resumeError: 'Not logged in' }))).toBe(true);
    expect(canRetry(movedTo('committed'))).toBe(false);
    const now = Date.now();
    expect(canUndo(movedTo('resumed', { undoUntil: now + 1000 }), now)).toBe(true);
    expect(canUndo(movedTo('resumed', { undoUntil: now - 1000 }), now)).toBe(false);
    expect(canUndo(movedTo('resumed', { asCopy: true, undoUntil: now + 1000 }), now)).toBe(false);
    expect(canUndo(movedTo('landed', { role: 'in', undoUntil: now + 1000 }), now)).toBe(true);
    expect(canUndo(movedTo('landed', { undoUntil: now + 1000 }), now)).toBe(false);
  });

  it('knows a turn the move waits for', () => {
    const base = { live: true, owned: true, activity: { kind: 'idle' as const, label: 'Idle' } };
    expect(sessionInTurn({ ...base, status: 'working' })).toBe(true);
    // Waiting on someone in the middle of its turn.
    expect(sessionInTurn({ ...base, status: 'waiting' })).toBe(true);
    expect(sessionInTurn({ ...base, status: 'idle' })).toBe(false);
    expect(sessionInTurn({ ...base, status: 'background' })).toBe(false);
    expect(sessionInTurn({ ...base, owned: false, status: 'working' })).toBe(false);
  });

  it('marks where a move stopped in its steps', () => {
    const stopped = movedTo('failed', {
      error: { code: 'target_offline', message: 'x', step: 'sending' },
    });
    expect(progressLines(stopped).map((l) => l.state)).toEqual([
      'done',
      'done',
      'done',
      'failed',
      'todo',
      'todo',
    ]);
    const waiting = movedTo('committed', { resumeError: 'Not logged in' });
    expect(progressLines(waiting).at(-1)?.state).toBe('failed');
    expect(progressLines(movedTo('undone')).every((l) => l.state === 'done')).toBe(true);
  });

  it('says when the code went through the remote instead of a bundle over the limit', () => {
    const m = movedTo('resumed', {
      code: { ...moveRecord().code!, mode: 'remote', remote: 'origin' },
      warnings: [
        moveCheck(
          'code_through_remote',
          'The code went through your remote origin: machine to machine it would have been 1.4 GB (over 1.0 GB).',
        ),
      ],
    });
    expect(progressLines(m)[2].label).toBe(
      'Packed — conversation 18 MB, code through your remote origin',
    );
    expect(moveWarnings(m)).toEqual([
      'The code went through your remote origin: machine to machine it would have been 1.4 GB (over 1.0 GB).',
    ]);
  });
});

describe('moves: the words', () => {
  afterEach(async () => {
    await useLanguage('en');
    hostOsSignal.set(undefined);
  });

  it("keeps the server's English, names and paths, and words it in Vietnamese", async () => {
    const blocker =
      "D:\\dev\\app has changes of its own (3 files). The code can't land in the project folder.";
    expect(moveText(blocker)).toBe(blocker);
    expect(moveText('Something new the server says.')).toBe('Something new the server says.');
    await useLanguage('vi');
    expect(moveText(blocker)).toBe(
      'D:\\dev\\app có thay đổi riêng (3 tệp). Mã nguồn không thể đặt vào thư mục dự án.',
    );
    expect(moveText("It can't move yet: Work isn't signed in on DESKTOP-PC.")).toBe(
      'Chưa thể chuyển: Work chưa đăng nhập trên DESKTOP-PC.',
    );
    // Not one of its patterns: as it came.
    expect(moveText('Something new the server says.')).toBe('Something new the server says.');
  });

  it('lists what a record says is worth knowing, once each, in the language shown', async () => {
    const m = movedTo('resumed', {
      warnings: [
        moveCheck('ignored_replaced', 'DESKTOP-PC had its own .env.local: kept for Undo move.'),
        moveCheck('claude_older', 'Claude Code on DESKTOP-PC is older (2.1.300, here 2.1.330).'),
        moveCheck('ignored_replaced', 'DESKTOP-PC had its own .env.local: kept for Undo move.'),
      ],
      code: { ...moveRecord().code!, warnings: ['lfs', 'something-newer'] },
    });
    expect(moveWarnings(m)).toEqual([
      'DESKTOP-PC had its own .env.local: kept for Undo move.',
      'Claude Code on DESKTOP-PC is older (2.1.300, here 2.1.330).',
      'The project uses Git LFS: its large files come from the remote, not with the move.',
    ]);
    expect(moveWarnings(moveRecord())).toEqual([]);
    await useLanguage('vi');
    expect(moveWarnings(m)[0]).toBe(
      'DESKTOP-PC đã có .env.local của riêng nó: được giữ lại để Hoàn tác việc chuyển.',
    );
    expect(moveWarnings(m)[2]).toBe(
      'Dự án dùng Git LFS: các tệp lớn được lấy từ remote, không đi kèm việc chuyển.',
    );
  });

  it('words a failure that came from the other computer with its name', () => {
    hostOsSignal.set('macos');
    const m = movedTo('failed', {
      error: {
        code: 'session_live_there',
        message: 'This computer runs the session right now.',
        step: 'landing',
      },
    });
    expect(failureText(m)).toBe('DESKTOP-PC runs this session right now.');
    expect(moveSummary(m)).toBe(
      "Couldn't move Fix the login: DESKTOP-PC runs this session right now. It stays on MacBook, as it was.",
    );
  });

  it('words refusals of the move routes for a person', () => {
    const refusal = (code: string, message: string, extra: Record<string, unknown> = {}) =>
      new ApiError(message, 409, 'http', code, undefined, undefined, extra);
    expect(
      moveErrorText(
        refusal(
          'session_lives_elsewhere',
          'This session lives on BUILD-BOX now: open it there, or bring it back to this computer first.',
          { machine: 'e'.repeat(32), name: 'BUILD-BOX' },
        ),
      ),
    ).toBe('This session lives on BUILD-BOX now: open it there, or bring it back first.');
    expect(
      moveErrorText(
        refusal(
          'move_too_late',
          'It is being placed on DESKTOP-PC now: once it is there, Undo move takes it back.',
        ),
      ),
    ).toBe('It is being placed on DESKTOP-PC now: once it is there, Undo move takes it back.');
    expect(moveErrorText(refusal('move_underway', 'This session is moving already.'))).toBe(
      'This session is moving already.',
    );
    expect(moveErrorText(refusal('not_elsewhere', 'It lives on this computer already.'))).toBe(
      'It lives on this computer already.',
    );
    // The computer it goes to is not paired with the one it lives on.
    const unknown = new MachineApiError(
      new ApiError('Not a paired computer.', 404, 'http', 'machines_unknown'),
      'DESKTOP-PC is no longer paired with this computer.',
      PEER_ID,
    );
    expect(moveErrorText(unknown, { source: 'DESKTOP-PC', target: 'BUILD-BOX' })).toBe(
      "BUILD-BOX isn't paired with DESKTOP-PC.",
    );
    const noMove = new MachineApiError(
      new ApiError('x', 403, 'http', 'grant_missing', undefined, undefined, { grant: 'move' }),
      "DESKTOP-PC hasn't given this computer that permission.",
      PEER_ID,
    );
    expect(moveErrorText(noMove, { source: 'DESKTOP-PC' })).toBe(
      "DESKTOP-PC doesn't let this computer move its sessions.",
    );
  });

  it("words the check of a move's remote, here and on the other computer", async () => {
    hostOsSignal.set('macos');
    expect(moveText("The remote origin doesn't answer from this computer without a prompt.")).toBe(
      "The remote origin doesn't answer from this Mac without a prompt.",
    );
    const fix =
      'On DESKTOP-PC: The remote has no such repository, or this computer may not see it.';
    expect(moveText(fix)).toBe(fix);
    await useLanguage('vi');
    expect(moveText(fix)).toBe(
      'Trên DESKTOP-PC: Remote không có kho này, hoặc máy tính này không được xem.',
    );
    expect(moveText("The remote origin doesn't answer on DESKTOP-PC without a prompt.")).toBe(
      'Remote origin không phản hồi trên DESKTOP-PC mà không yêu cầu đăng nhập.',
    );
  });

  it('says on which computer Computers is off, in the old words too', async () => {
    hostOsSignal.set('macos');
    const here = new ApiError(
      'Computers is off on this computer: turn it on in Computers first.',
      409,
      'http',
      'machines_off',
    );
    expect(moveErrorText(here)).toBe(
      'Computers is off on this Mac: turn it on in Computers first.',
    );
    const there = new ApiError('Computers is off on DESKTOP-PC.', 403, 'http', 'machines_off');
    expect(moveErrorText(there)).toBe('Computers is off on DESKTOP-PC.');
    // As the other computer's check says it in the sheet — an older
    // Mother Claude says Machines.
    expect(moveText('Computers is off on DESKTOP-PC.')).toBe('Computers is off on DESKTOP-PC.');
    expect(moveText('Machines is off on DESKTOP-PC.')).toBe('Computers is off on DESKTOP-PC.');
    expect(moveText("Change it on DESKTOP-PC's Machines page.")).toBe(
      'Change it in Computers on DESKTOP-PC.',
    );
    await useLanguage('vi');
    expect(moveText('Computers is off on DESKTOP-PC.')).toBe(
      'Tính năng Máy tính đang tắt trên DESKTOP-PC.',
    );
  });
});

describe('MovesService', () => {
  let backend: FakeBackend;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    pairedMachines(true);
    backend.routes.set('/machines', () => ({
      status: 200,
      body: TestBed.inject(MachinesService).view(),
    }));
  });

  afterEach(() => setActiveMock(null));

  const proxy = `/machines/${PEER_ID}/proxy`;

  it("follows another computer's move until it ends, and says how it ended", async () => {
    // The requests answer on the real clock; the polls wait on the fake one.
    const realTimeout = window.setTimeout.bind(window);
    const answered = () => new Promise((r) => realTimeout(r, 30));
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    try {
      const out = (state: Parameters<typeof movedTo>[0]) =>
        movedTo(state, {
          from: { id: PEER_ID, name: 'DESKTOP-PC', os: 'windows' },
          to: { id: 'e'.repeat(32), name: 'BUILD', os: 'linux' },
        });
      // What it answers when asked (each answer newer than the last).
      let state: Parameters<typeof movedTo>[0] = 'sending';
      backend.routes.set(`${proxy}/sessions/s1/move`, () => ({
        status: 202,
        body: out('preflight'),
      }));
      backend.routes.set(`${proxy}/moves/${MOVE_ID}`, () => ({ status: 200, body: out(state) }));
      const moves = TestBed.inject(MovesService);
      const success = vi.spyOn(TestBed.inject(NoticeService), 'success');

      await moves.start(PEER_ID, 's1', { to: 'e'.repeat(32) });
      expect(moves.following(PEER_ID, MOVE_ID)).toBe(true);
      await vi.advanceTimersByTimeAsync(1600);
      await answered();
      expect(moves.get(PEER_ID, MOVE_ID)?.move.state).toBe('sending');

      state = 'resumed';
      await vi.advanceTimersByTimeAsync(1600);
      await answered();
      expect(moves.get(PEER_ID, MOVE_ID)?.move.state).toBe('resumed');
      expect(moves.following(PEER_ID, MOVE_ID)).toBe(false);
      // Its name here (BUILD-BOX), not the one DESKTOP-PC gave it.
      expect(success).toHaveBeenCalledWith('Fix the login runs on BUILD-BOX now.', {
        action: expect.objectContaining({ label: 'Open' }),
      });
      const asked = backend.calls.filter((c) => c === `GET ${proxy}/moves/${MOVE_ID}`).length;
      await vi.advanceTimersByTimeAsync(10_000);
      expect(backend.calls.filter((c) => c === `GET ${proxy}/moves/${MOVE_ID}`).length).toBe(asked);
    } finally {
      vi.useRealTimers();
    }
  });

  it('takes a record only when it is newer than the one it has', async () => {
    const moves = TestBed.inject(MovesService);
    backend.routes.set('/moves', () => ({
      status: 200,
      body: { moves: [movedTo('resumed', { updatedAt: 10_000_000_000_000 })] },
    }));
    await moves.load();
    backend.routes.set('/moves', () => ({
      status: 200,
      body: { moves: [movedTo('sending', { updatedAt: 1 })] },
    }));
    await moves.load();
    expect(moves.here().map((e) => e.move.state)).toEqual(['resumed']);
    expect(moves.selfId()).toBe(SELF_ID);
  });
});
