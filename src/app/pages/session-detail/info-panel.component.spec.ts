import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, useFakeServer } from '../../core/mock/fake-server.testing';
import { ownedCaps } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { PlanUsage, SessionDetail, SessionSummary } from '../../core/models';
import { RealtimeService } from '../../core/realtime.service';
import { InfoPanelComponent } from './info-panel.component';
import { SessionContext } from './session-context';

const M = 60_000;

const session = (over: Partial<SessionSummary>): SessionDetail =>
  ({
    id: 's1',
    name: 'auth',
    projectName: 'api',
    cwd: '/Users/me/dev/api',
    surface: 'mother-claude',
    owned: false,
    live: true,
    status: 'idle',
    activity: { kind: 'idle', label: 'Idle' },
    usage: { input: 1_000, output: 41_000, cacheRead: 0, cacheWrite: 0 },
    messageCount: 3,
    agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
    queued: 0,
    pending: [],
    control: { ...ownedCaps(), channel: 'hooks', setUltracode: false, setThinking: false },
    ...over,
  }) as SessionDetail;

const plan: PlanUsage = {
  available: true,
  windows: [{ key: 'five_hour', label: 'Current session', pct: 42 }],
  refreshing: false,
  intervalSecs: 300,
};

describe('InfoPanelComponent', () => {
  beforeEach(() => useFakeServer(new FakeBackend(), [provideRouter([]), SessionContext]));
  afterEach(() => setActiveMock(null));

  const render = (s: SessionDetail) => {
    TestBed.inject(SessionContext).detail.set(s);
    const fixture = TestBed.createComponent(InfoPanelComponent);
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };

  /** The words of the `<dd>` after the `<dt>` named `name` (each text node
   * on its own, as the flex layout shows them). */
  const row = (el: HTMLElement, name: string): string => {
    const dt = [...el.querySelectorAll('dt')].find((d) => d.textContent?.trim() === name);
    const dd = dt?.nextElementSibling;
    if (!dd) return '';
    const parts: string[] = [];
    const walk = document.createTreeWalker(dd, NodeFilter.SHOW_TEXT);
    for (let n = walk.nextNode(); n; n = walk.nextNode()) parts.push(n.textContent ?? '');
    return parts.join(' ').replace(/\s+/g, ' ').trim();
  };

  it('shows effort, ultracode, thinking and fast with how each is known', () => {
    const el = render(
      session({
        effort: 'max',
        effortSource: 'observed',
        ultracode: 'on',
        ultracodeSource: 'transcript',
        ultracodeAt: Date.now() - 6 * M,
        thinking: { mode: 'adaptive', source: 'launch' },
        usage: { input: 1_000, output: 12_500, cacheRead: 0, cacheWrite: 0, thinking: 3_100 },
        lastThinkingAt: Date.now() - 3 * M,
        fast: true,
      }),
    );
    expect(row(el, 'Effort')).toBe('Max · as sent with the latest request');
    expect(row(el, 'Ultracode')).toBe('On · as of the last message, 6m ago');
    expect(row(el, 'Thinking')).toBe(
      'Adaptive (at launch) 3.1k tokens · 25% of output · last used 3m ago',
    );
    expect(row(el, 'Fast mode')).toBe('On · the latest request ran in fast mode');
  });

  it('says what is not known instead of guessing', () => {
    const el = render(session({ usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 } }));
    expect(row(el, 'Effort')).toBe('Not reported');
    expect(row(el, 'Ultracode')).toBe('Unknown · shows after the next message');
    expect(row(el, 'Thinking')).toBe('Not reported No thinking tokens so far');
    expect(row(el, 'Fast mode')).toBe('Not seen');
  });

  it('says why ultracode is unknown and when it will show', () => {
    const vscode = (over: Partial<SessionSummary>) => session({ surface: 'vscode', ...over });
    expect(row(render(vscode({ ultracodeUnknown: 'no-prompt' })), 'Ultracode')).toBe(
      'Unknown · shows after the first message in VS Code',
    );
    expect(
      row(
        render(
          vscode({
            ultracodeUnknown: 'restarted',
            ultracodeBefore: 'on',
            ultracodeBeforeAt: Date.now() - 10.5 * 60 * M,
          }),
        ),
        'Ultracode',
      ),
    ).toBe('Unknown · was on at the last message (10h ago), before the restart');
    expect(row(render(vscode({ ultracodeUnknown: 'reading' })), 'Ultracode')).toBe(
      "Unknown · still reading this session's history",
    );
    // A session that has ended: not recorded.
    expect(row(render(vscode({ live: false, status: 'ended' })), 'Ultracode')).toBe('Not recorded');
  });

  it('explains an owned session without effort levels and unavailable ultracode', () => {
    const el = render(
      session({
        owned: true,
        effortSource: 'applied',
        ultracode: 'unavailable',
        ultracodeSource: 'applied',
        thinking: { mode: 'off', source: 'control' },
        control: ownedCaps(),
      }),
    );
    expect(row(el, 'Effort')).toBe('None · this model has no effort levels');
    expect(row(el, 'Ultracode')).toContain('Unavailable · requested, but this model or workspace');
    expect(row(el, 'Thinking')).toContain('Off (set by Mother Claude)');
  });

  it("says an owned session's thinking is Claude Code's default until someone sets it", () => {
    const el = render(
      session({
        owned: true,
        fast: true,
        control: ownedCaps(),
        usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
      }),
    );
    expect(row(el, 'Thinking')).toBe(
      "Claude Code's default (not reported) No thinking tokens so far",
    );
    // Fast mode may be reported as switched on before any request ran in it.
    expect(row(el, 'Fast mode')).toBe('On · switched on, or the latest request ran in it');
  });

  it('does not call thinking off when the session has thought since launch', () => {
    const now = Date.now();
    const el = render(
      session({
        thinking: { mode: 'off', source: 'launch' },
        startedAt: now - 30 * M,
        lastThinkingAt: now - 3 * M,
        usage: { input: 1_000, output: 12_500, cacheRead: 0, cacheWrite: 0, thinking: 3_100 },
      }),
    );
    expect(row(el, 'Thinking')).toBe(
      'Off (at launch), but used since 3.1k tokens · 25% of output · last used 3m ago',
    );
  });

  it('shows the plan usage card only when the account has plan usage', () => {
    const heading = (el: HTMLElement) =>
      [...el.querySelectorAll('h2')].some((h) => h.textContent?.trim() === 'Plan usage');
    const rt = TestBed.inject(RealtimeService);
    expect(heading(render(session({})))).toBeFalse();
    rt.mergePlanUsage({ ...plan, available: false });
    expect(heading(render(session({})))).toBeFalse();
    rt.mergePlanUsage(plan);
    const el = render(session({}));
    expect(heading(el)).toBeTrue();
    expect(el.querySelector('[role="meter"]')?.getAttribute('aria-valuenow')).toBe('42');
  });
});
