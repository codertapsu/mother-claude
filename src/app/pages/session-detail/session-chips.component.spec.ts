import { WritableSignal } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { ownedCaps } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { SessionDetail, SessionSummary } from '../../core/models';
import { NowService } from '../../core/now.service';
import { RealtimeService } from '../../core/realtime.service';
import { SessionChipsComponent } from './session-chips.component';
import { SessionContext } from './session-context';

const M = 60_000;

const session = (over: Partial<SessionSummary>): SessionDetail =>
  ({
    id: 's1',
    name: 'auth',
    projectName: 'api',
    cwd: '/Users/me/dev/api',
    surface: 'vscode',
    owned: false,
    live: true,
    status: 'idle',
    activity: { kind: 'idle', label: 'Idle' },
    usage: { input: 0, output: 12_500, cacheRead: 0, cacheWrite: 0, thinking: 3_100 },
    messageCount: 3,
    agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
    queued: 0,
    pending: [],
    control: {
      ...ownedCaps(),
      channel: 'hooks',
      setEffort: false,
      setModel: false,
      setUltracode: false,
      setThinking: false,
      limits: [
        {
          code: 'no_model_change',
          text: "The model, effort, ultracode, thinking and slash commands can't be changed in this session from here.",
          action: 'fork',
        },
      ],
    },
    ...over,
  }) as SessionDetail;

/** A session Mother Claude runs on Haiku: no effort levels, no ultracode,
 * and a thinking setting nobody chose (Claude Code doesn't report it). */
const ownedHaiku = (over: Partial<SessionSummary> = {}): SessionDetail =>
  session({
    owned: true,
    surface: 'mother-claude',
    model: { id: 'claude-haiku-4-5', label: 'Haiku 4.5' },
    ultracode: 'off',
    ultracodeSource: 'applied',
    usage: { input: 0, output: 2_400, cacheRead: 0, cacheWrite: 0 },
    control: { ...ownedCaps(), setUltracode: false },
    ...over,
  });

describe('SessionChipsComponent', () => {
  let backend: FakeBackend;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([]), SessionContext]);
    // Actions need a working connection (Live or Limited).
    (
      TestBed.inject(RealtimeService) as unknown as { connectionS: { set(v: string): void } }
    ).connectionS.set('limited');
  });
  afterEach(() => setActiveMock(null));

  const render = (s: SessionDetail) => {
    const ctx = TestBed.inject(SessionContext);
    ctx.id.set(s.id);
    ctx.detail.set(s);
    const fixture = TestBed.createComponent(SessionChipsComponent);
    fixture.detectChanges();
    return { fixture, el: fixture.nativeElement as HTMLElement };
  };
  const button = (el: HTMLElement, text: string) =>
    [...el.querySelectorAll('button')].find((b) => b.textContent?.includes(text)) as
      HTMLButtonElement | undefined;
  const words = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const sheet = () => words(document.querySelector('dialog[open]'));
  /** Close the "why" sheet between taps. */
  const closeWhy = (fixture: ComponentFixture<SessionChipsComponent>) => {
    (fixture.componentInstance as unknown as { why: WritableSignal<unknown> }).why.set(null);
    fixture.detectChanges();
  };

  it('offers ultracode and thinking switches on sessions that take them', async () => {
    backend.routes.set('/sessions/s1/ultracode', () => ({ status: 200, body: {} }));
    const { el } = render(
      session({
        owned: true,
        surface: 'mother-claude',
        ultracode: 'off',
        ultracodeSource: 'applied',
        thinking: { mode: 'adaptive', source: 'control' },
        control: ownedCaps(),
      }),
    );
    const switches = [...el.querySelectorAll('[role="switch"]')];
    expect(switches.map((s) => s.getAttribute('aria-label'))).toEqual(['Ultracode', 'Thinking']);
    expect(switches.map((s) => s.getAttribute('aria-checked'))).toEqual(['false', 'true']);
    expect(switches.map(words)).toEqual(['Ultracode off', 'Thinking on']);
    expect(switches[1].getAttribute('title')).toBe('Thinking: Adaptive (set by Mother Claude)');
    (switches[0] as HTMLButtonElement).click();
    await flush();
    expect(backend.calls).toContain('POST /sessions/s1/ultracode');
    expect(backend.bodyOf('POST /sessions/s1/ultracode')).toEqual({ enabled: true });
  });

  it('lets an owned session choose thinking on or off while its setting is unknown', async () => {
    backend.routes.set('/sessions/s1/thinking', () => ({ status: 200, body: {} }));
    const { fixture, el } = render(ownedHaiku());
    // A choice, not a switch claiming "off" for a default nobody reports.
    expect(el.querySelector('[role="switch"][aria-label="Thinking"]')).toBeNull();
    const select = el.querySelector('select[aria-label="Thinking"]') as HTMLSelectElement;
    expect(select).not.toBeNull();
    expect(words(select.closest('label'))).toContain('Thinking default');
    expect([...select.options].map((o) => o.value)).toEqual(['', 'on', 'off']);
    expect(select.disabled).toBeFalse();
    select.value = 'off';
    select.dispatchEvent(new Event('change'));
    await flush();
    fixture.detectChanges();
    expect(backend.bodyOf('POST /sessions/s1/thinking')).toEqual({ enabled: false });
    expect(select.selectedIndex).toBe(0);
    // No read-only thinking chip, and the switch appears once it is known.
    expect(button(el, 'Thinking')).toBeUndefined();
    TestBed.inject(SessionContext).detail.set(
      ownedHaiku({ thinking: { mode: 'off', source: 'control' } }),
    );
    fixture.detectChanges();
    const sw = el.querySelector('[role="switch"][aria-label="Thinking"]');
    expect(sw?.getAttribute('aria-checked')).toBe('false');
    expect(words(sw)).toBe('Thinking off');
  });

  it('says an owned model without effort levels has none, not that it is unreported', () => {
    const effortPick = (el: HTMLElement) =>
      el.querySelector('select[aria-label="Effort"]')?.closest('label');
    // Claude Code applies no level: the model has none.
    const none = effortPick(render(ownedHaiku({ effortSource: 'applied' })).el);
    expect(words(none?.querySelector('.pv'))).toBe('No effort');
    expect(none?.getAttribute('title')).toBe('Effort: none, this model has no effort levels');
    // Nothing reported yet: unknown, not none.
    const unknown = effortPick(render(ownedHaiku()).el);
    expect(words(unknown?.querySelector('.pv'))).toBe('Effort');
    expect(unknown?.getAttribute('title')).toBe('Effort: not reported');
  });

  it("says why an owned session's model can't run ultracode, without a 'no limits' line", () => {
    const { fixture, el } = render(ownedHaiku());
    const chip = button(el, 'Ultracode off');
    expect(chip?.getAttribute('aria-label')).toBe('Ultracode: Off (as Claude Code applies it)');
    chip?.click();
    fixture.detectChanges();
    expect(sheet()).toContain('Ultracode is off.');
    expect(sheet()).toContain("This session's model or workspace can't run ultracode");
    expect(sheet()).not.toContain("doesn't let Mother Claude change");
    expect(sheet()).not.toContain('No limits');
    closeWhy(fixture);
  });

  it('offers to withdraw an ultracode request the model cannot run', async () => {
    backend.routes.set('/sessions/s1/ultracode', () => ({ status: 200, body: {} }));
    const unavailable = { ultracode: 'unavailable' as const, ultracodeSource: 'applied' as const };
    const { fixture, el } = render(ownedHaiku({ ...unavailable, control: ownedCaps() }));
    expect(el.querySelector('[role="switch"][aria-label="Ultracode"]')).toBeNull();
    const chip = button(el, 'Ultracode unavailable');
    expect(chip?.classList.contains('warn')).toBeTrue();
    chip?.click();
    fixture.detectChanges();
    expect(sheet()).toContain("it can't run here");
    expect(sheet()).toContain('The request stays in place');
    expect(sheet()).not.toContain('No limits');
    const cancel = [...document.querySelectorAll('dialog[open] button')].find((b) =>
      b.textContent?.includes('Cancel the ultracode request'),
    ) as HTMLButtonElement;
    expect(cancel.disabled).toBeFalse();
    cancel.click();
    await flush();
    expect(backend.bodyOf('POST /sessions/s1/ultracode')).toEqual({ enabled: false });
    fixture.detectChanges();
    expect(document.querySelector('dialog[open]')).toBeNull();

    // A session that can't take the change: the reason, and no button.
    const other = render(
      ownedHaiku({ ...unavailable, control: { ...ownedCaps(), setUltracode: false } }),
    );
    button(other.el, 'Ultracode unavailable')?.click();
    other.fixture.detectChanges();
    expect(sheet()).toContain('The request stays in place');
    expect(sheet()).not.toContain('Cancel the ultracode request');
    closeWhy(other.fixture);
  });

  it('shows read-only chips elsewhere, and says how each value is known', () => {
    const now = TestBed.inject(NowService).now();
    const { fixture, el } = render(
      session({
        effort: 'xhigh',
        effortSource: 'observed',
        ultracode: 'on',
        ultracodeSource: 'transcript',
        ultracodeAt: now - 6 * M,
        thinking: { mode: 'adaptive', source: 'launch' },
        fast: true,
      }),
    );
    expect(el.querySelectorAll('[role="switch"]').length).toBe(0);
    expect(el.querySelector('select[aria-label="Thinking"]')).toBeNull();
    expect(button(el, 'Extra high')?.getAttribute('aria-label')).toBe(
      'Effort: Extra high (as sent with the latest request)',
    );
    expect(button(el, 'Ultracode on')?.getAttribute('aria-label')).toBe(
      'Ultracode: On (as of the last message, 6m ago)',
    );
    expect(button(el, 'Thinking adaptive')?.getAttribute('title')).toBe(
      'Thinking: Adaptive (at launch)',
    );
    button(el, 'Ultracode on')?.click();
    fixture.detectChanges();
    expect(sheet()).toContain('As of the last message (6m ago)');
    // Where it is switched instead…
    expect(sheet()).toContain('Switch it in VS Code itself; it applies to that session only.');
    // …with the fix (continue here) under it.
    expect(sheet()).toContain('Continue here (fork)');

    closeWhy(fixture);
    button(el, 'Thinking adaptive')?.click();
    fixture.detectChanges();
    expect(sheet()).toContain('without Mother Claude seeing it');
    expect(sheet()).toContain('3.1k thinking tokens');

    closeWhy(fixture);
    const fast = button(el, 'Fast');
    expect(fast?.getAttribute('aria-label')).toBe('Fast mode: the latest request ran in fast mode');
    fast?.click();
    fixture.detectChanges();
    expect(sheet()).toContain('the latest request ran');
    closeWhy(fixture);
  });

  it('says ultracode is not known yet in a session running elsewhere, and why', () => {
    const now = TestBed.inject(NowService).now();
    const { fixture, el } = render(
      session({
        ultracodeUnknown: 'restarted',
        ultracodeBefore: 'on',
        ultracodeBeforeAt: now - 10 * 60 * M,
      }),
    );
    const chip = button(el, 'Ultracode ?');
    expect(chip?.classList.contains('off')).toBeTrue();
    expect(chip?.getAttribute('aria-label')).toBe(
      'Ultracode: not known since the session restarted (it was on); the next message in VS Code shows it',
    );
    chip?.click();
    fixture.detectChanges();
    expect(sheet()).toContain('It was on at the last message before the restart (10h ago)');
    expect(sheet()).toContain(
      'A restart turns ultracode off until it is switched on again in VS Code',
    );
    expect(sheet()).toContain('Switch it in VS Code itself; it applies to that session only.');
    expect(sheet()).toContain('Continue here (fork)');
    closeWhy(fixture);

    // A new conversation: known after its first message.
    const fresh = render(session({ ultracodeUnknown: 'no-prompt', messageCount: 0 }));
    expect(button(fresh.el, 'Ultracode ?')?.getAttribute('title')).toBe(
      'Ultracode: not known; shows after the first message in VS Code',
    );
    // In the terminal: the command that switches it there.
    const term = render(session({ surface: 'terminal' }));
    button(term.el, 'Ultracode ?')?.click();
    term.fixture.detectChanges();
    expect(sheet()).toContain('/effort ultracode on');
    closeWhy(term.fixture);

    // Owned sessions report it by themselves; ended ones say so on Info.
    const owned = render(ownedHaiku({ ultracode: undefined, ultracodeSource: undefined }));
    expect(button(owned.el, 'Ultracode')).toBeUndefined();
    const ended = render(session({ live: false, status: 'ended' }));
    expect(button(ended.el, 'Ultracode')).toBeUndefined();
  });

  it("doesn't claim a request ran fast when an owned session only reports fast mode on", () => {
    const { fixture, el } = render(
      session({ owned: true, surface: 'mother-claude', fast: true, control: ownedCaps() }),
    );
    const fast = button(el, 'Fast');
    expect(fast?.getAttribute('title')).toBe(
      'Fast mode: switched on, or the latest request ran in it',
    );
    fast?.click();
    fixture.detectChanges();
    expect(sheet()).toContain('reports its faster output mode as switched on');
    closeWhy(fixture);
  });

  it("shows thinking's share when its setting is unknown, and nothing when unused", () => {
    const shown = button(render(session({})).el, 'Thinking 25%');
    expect(shown).toBeDefined();
    expect(shown?.getAttribute('aria-label')).toBe(
      "Thinking: 3.1k tokens, 25% of the output; the setting isn't reported",
    );
    const none = render(session({ usage: { input: 0, output: 5, cacheRead: 0, cacheWrite: 0 } }));
    expect(button(none.el, 'Thinking')).toBeUndefined();
  });

  it('shows the thinking used, not "off", when the session thought since launch', () => {
    const startedAt = Date.now() - 30 * M;
    const off = { thinking: { mode: 'off' as const, source: 'launch' as const }, startedAt };
    const { fixture, el } = render(session({ ...off, lastThinkingAt: Date.now() - 2 * M }));
    expect(button(el, 'Thinking off')).toBeUndefined();
    const chip = button(el, 'Thinking 25%');
    expect(chip?.getAttribute('title')).toBe(
      'Thinking: Off (at launch), but used since (3.1k tokens, 25% of the output)',
    );
    chip?.click();
    fixture.detectChanges();
    expect(sheet()).toContain('it was switched on in the session itself');
    expect(sheet()).not.toContain('Thinking is off.');
    closeWhy(fixture);
    // Thinking from before this process (a resumed session) doesn't count.
    const before = render(session({ ...off, lastThinkingAt: startedAt - 10 * M }));
    expect(button(before.el, 'Thinking off')).toBeDefined();
  });
});
