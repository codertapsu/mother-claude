import { ApiError } from '../core/api.service';
import { SessionSummary } from '../core/models';
import {
  elsewhereText,
  normalizeSwitchReply,
  runningTasks,
  switchBusy,
  switchErrorText,
  switchMode,
  switchedText,
  switchingToOf,
} from './switch-account';

const s = (over: Partial<SessionSummary>): SessionSummary =>
  ({
    id: 's1',
    live: true,
    owned: true,
    status: 'idle',
    surface: 'mother-claude',
    activity: { kind: 'idle', label: 'Idle' },
    agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
    ...over,
  }) as SessionSummary;

describe('switch account helpers', () => {
  it('says what a switch can do for a session', () => {
    expect(switchMode(s({}))).toBe('here');
    expect(switchMode(s({ owned: false, surface: 'vscode' }))).toBe('elsewhere');
    expect(switchMode(s({ live: false, owned: false }))).toBe('later');
    expect(switchBusy(s({ status: 'working' }))).toBeTrue();
    expect(switchBusy(s({ activity: { kind: 'tool', label: 'Running Bash' } }))).toBeTrue();
    // Background tasks are no turn.
    expect(switchBusy(s({ activity: { kind: 'background', label: 'Waiting' } }))).toBeFalse();
    expect(switchBusy(s({ live: false, status: 'working' }))).toBeFalse();
    expect(elsewhereText(s({ surface: 'vscode' }))).toBe(
      "It runs in VS Code: its account can't change while it runs there.",
    );
    expect(elsewhereText(s({ surface: 'unknown' }))).toContain('another app');
  });

  it('reads a switch waiting for its turn and the answers', () => {
    expect(switchingToOf(s({ switchingTo: 'acc_w' }))).toBe('acc_w');
    expect(switchingToOf(s({ switchingTo: { id: 'acc_p', name: 'Pro' } }))).toBe('acc_p');
    expect(switchingToOf(s({ switchingTo: '' }))).toBeNull();
    expect(switchingToOf(s({}))).toBeNull();
    expect(normalizeSwitchReply({ pending: true }, 'acc_w')).toEqual({ pending: true });
    expect(normalizeSwitchReply({ id: 's1', account: 'acc_w', moved: true }, 'acc_w')).toEqual({
      pending: false,
      id: 's1',
      account: 'acc_w',
      moved: true,
    });
    expect(normalizeSwitchReply(null, 'acc_w')).toEqual({
      pending: false,
      id: '',
      account: 'acc_w',
      moved: false,
    });
  });

  it('counts the background tasks a switch stops', () => {
    const summary = s({
      agents: { running: 0, total: 2, workflowsRunning: 0, backgroundRunning: 2 },
    });
    expect(runningTasks(summary)).toBe(2);
    expect(
      runningTasks(summary, {
        tasks: [
          { id: 't1', kind: 'bash', label: 'npm test', status: 'running', outputAvailable: true },
          { id: 't2', kind: 'bash', label: 'build', status: 'completed', outputAvailable: true },
        ],
      }),
    ).toBe(1);
  });

  it('puts a switch and its refusals in words', () => {
    expect(switchedText('Work', false)).toBe('Now on Work.');
    expect(switchedText('Work', true)).toBe('Now on Work: the conversation moved to its folder.');
    expect(switchErrorText(new ApiError('', 409, 'http', 'session_not_owned'))).toContain(
      'continue it on the other account instead',
    );
    expect(switchErrorText(new ApiError('', 409, 'http', 'turn_running'))).toContain(
      'between turns',
    );
    expect(switchErrorText(new ApiError('', 422, 'http', 'unknown_account'))).toBe(
      'That account no longer exists: pick another one.',
    );
  });
});
