import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, useFakeServer } from '../core/mock/fake-server.testing';
import { ownedCaps } from '../core/mock/fixtures';
import { setActiveMock } from '../core/mock/mock-api';
import { SessionSummary } from '../core/models';
import { ViewportService } from '../core/viewport.service';
import { SessionCardComponent } from './session-card.component';

const base = (over: Partial<SessionSummary>): SessionSummary => ({
  id: 's1',
  name: 'auth',
  projectName: 'api',
  cwd: '/Users/me/dev/api',
  surface: 'vscode',
  owned: false,
  live: true,
  status: 'idle',
  activity: { kind: 'idle', label: 'Idle' },
  model: { id: 'claude-opus-5-5', label: 'Opus 5.5' },
  usage: { input: 1_000, output: 41_000, cacheRead: 0, cacheWrite: 0 },
  messageCount: 3,
  agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
  queued: 0,
  pending: [],
  control: { ...ownedCaps(), channel: 'hooks' },
  ...over,
});

describe('SessionCardComponent chips', () => {
  beforeEach(() => useFakeServer(new FakeBackend(), [provideRouter([])]));
  afterEach(() => setActiveMock(null));

  const chips = (s: SessionSummary) => {
    const fixture = TestBed.createComponent(SessionCardComponent);
    fixture.componentRef.setInput('session', s);
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    return [...el.querySelectorAll('.chips mc-chip')].map((c) => ({
      text: (c.textContent ?? '').replace(/\s+/g, ' ').trim(),
      title: c.getAttribute('title') ?? '',
      tone: c.getAttribute('data-tone') ?? '',
    }));
  };

  it('shows effort, ultracode, thinking and fast after the model', () => {
    const list = chips(
      base({
        effort: 'xhigh',
        effortSource: 'observed',
        ultracode: 'on',
        ultracodeSource: 'transcript',
        ultracodeAt: Date.now() - 5 * 60_000,
        usage: { input: 1_000, output: 41_000, cacheRead: 0, cacheWrite: 0, thinking: 9_800 },
        fast: true,
      }),
    );
    const texts = list.map((c) => c.text);
    expect(texts.slice(0, 5)).toEqual([
      'Opus 5.5',
      'Effort X-high',
      'Ultracode',
      'Thinking 24%',
      'Fast',
    ]);
    const effort = list[1];
    expect(effort.title).toBe('Effort: Extra high (as sent with the latest request)');
    const ultra = list[2];
    expect(ultra.tone).toBe('accent');
    expect(ultra.title).toBe('Ultracode: On (as of the last message, 5m ago)');
    expect(list[3].title).toContain('9.8k tokens, 24% of the output');
  });

  it('says thinking is off; ultracode that is not running gets no chip', () => {
    const texts = chips(
      base({
        thinking: { mode: 'off', source: 'control' },
        ultracode: 'unavailable',
        ultracodeSource: 'applied',
      }),
    ).map((c) => c.text);
    expect(texts).toEqual(['Opus 5.5', 'Thinking off']);
  });

  it('adds nothing for values that are unknown, off or unused', () => {
    const texts = chips(
      base({ ultracode: 'off', thinking: { mode: 'adaptive', source: 'default' } }),
    ).map((c) => c.text);
    expect(texts).toEqual(['Opus 5.5']);
  });

  it('says fast mode may be switched on, not only seen, on an owned session', () => {
    const owned = chips(base({ owned: true, fast: true, control: ownedCaps() }));
    expect(owned.find((c) => c.text === 'Fast')?.title).toBe(
      'Fast mode: switched on, or the latest request ran in it',
    );
    const foreign = chips(base({ fast: true }));
    expect(foreign.find((c) => c.text === 'Fast')?.title).toBe(
      'Fast mode: the latest request ran in fast mode',
    );
  });

  it('shows the thinking used instead of a launch "off" the session has outgrown', () => {
    const now = Date.now();
    const off = {
      thinking: { mode: 'off' as const, source: 'launch' as const },
      startedAt: now - 30 * 60_000,
      usage: { input: 1_000, output: 41_000, cacheRead: 0, cacheWrite: 0, thinking: 9_800 },
    };
    const since = chips(base({ ...off, lastThinkingAt: now - 60_000 }));
    expect(since.map((c) => c.text)).toContain('Thinking 24%');
    expect(since.find((c) => c.text === 'Thinking 24%')?.title).toContain(
      'Off (at launch), but used since',
    );
    // Thinking from an earlier run of a resumed session says nothing.
    const earlier = chips(base({ ...off, lastThinkingAt: now - 60 * 60_000 }));
    expect(earlier.map((c) => c.text)).toContain('Thinking off');
  });

  it('names the Claude account only when the session carries one', () => {
    const render = (s: SessionSummary) => {
      const fixture = TestBed.createComponent(SessionCardComponent);
      fixture.componentRef.setInput('session', s);
      fixture.detectChanges();
      return fixture.nativeElement as HTMLElement;
    };
    // Only the Default exists: no account anywhere on the card.
    expect(render(base({})).querySelector('mc-account-chip')).toBeNull();
    const el = render(
      base({
        account: { id: 'acc_9b02d3e6c1f8', name: 'Marcus (Pro)', color: '#f59e0b', kind: 'token' },
      }),
    );
    const chip = el.querySelector('.chips mc-account-chip') as HTMLElement;
    expect((chip.textContent ?? '').replace(/\s+/g, ' ').trim()).toBe('Account Marcus (Pro)');
    expect(chip.getAttribute('title')).toBe('Claude account: Marcus (Pro) (token)');
    expect((chip.querySelector('.dot') as HTMLElement).style.background).toContain('245, 158, 11');
    // The setting chips are unchanged by it.
    expect(
      chips(
        base({ account: { id: 'default', name: 'Default', color: '#8b5cf6', kind: 'directory' } }),
      ).map((c) => c.text),
    ).toEqual(['Opus 5.5']);
  });

  it('packs the setting chips on the smallest screens', () => {
    TestBed.inject(ViewportService).narrow.set(true);
    const list = chips(
      base({
        effort: 'max',
        effortSource: 'observed',
        ultracode: 'on',
        ultracodeSource: 'transcript',
        ultracodeAt: Date.now() - 40 * 60_000,
        usage: { input: 1_000, output: 21_000, cacheRead: 0, cacheWrite: 0, thinking: 5_100 },
        fast: true,
      }),
    );
    // Screen readers still hear the words; the screen shows "Max · Ultracode · Fast" and "24%".
    expect(list.map((c) => c.text)).toEqual([
      'Opus 5.5',
      'Effort Max · Ultracode · Fast',
      'Thinking 24%',
    ]);
    const packed = list[1];
    expect(packed.title).toBe(
      'Effort: Max (as sent with the latest request) · Ultracode: On (as of the last message, 40m ago) · Fast mode: the latest request ran in fast mode',
    );
    expect(list[2].title).toContain('5.1k tokens, 24% of the output');
    // A single setting keeps its own look.
    const alone = chips(base({ ultracode: 'on', ultracodeSource: 'applied' }));
    expect(alone[1]).toEqual(jasmine.objectContaining({ text: 'Ultracode', tone: 'accent' }));
    expect(chips(base({})).map((c) => c.text)).toEqual(['Opus 5.5']);
  });
});
