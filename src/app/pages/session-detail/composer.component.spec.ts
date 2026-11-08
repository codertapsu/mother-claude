import { WritableSignal } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { ControlCaps, SessionDetail } from '../../core/models';
import { RealtimeService } from '../../core/realtime.service';
import { ComposerComponent, remainderAfterSend } from './composer.component';
import { SessionContext } from './session-context';

interface Reply {
  status: number;
  body: unknown;
}

interface ComposerInternals {
  text: WritableSignal<string>;
  palette: WritableSignal<'slash' | 'files' | null>;
  submit(how: 'send'): Promise<void>;
}

describe('remainderAfterSend', () => {
  it('keeps what was typed after the sent text', () => {
    expect(remainderAfterSend('hello', 'hello')).toBe('');
    expect(remainderAfterSend('hello\nand more', 'hello')).toBe('and more');
    expect(remainderAfterSend('rewritten', 'hello')).toBe('rewritten');
  });
});

describe('ComposerComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<ComposerComponent>;
  let cmp: ComposerInternals;
  let ta: HTMLTextAreaElement;

  const type = (value: string) => {
    ta.value = value;
    ta.setSelectionRange(value.length, value.length);
    ta.dispatchEvent(new Event('input'));
  };

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([]), SessionContext]);
    const ctx = TestBed.inject(SessionContext);
    ctx.id.set('s1');
    ctx.detail.set({
      id: 's1',
      live: true,
      status: 'idle',
      pending: [],
      control: { message: 'direct', slashCommands: true, limits: [] } as unknown as ControlCaps,
    } as unknown as SessionDetail);
    // Live: sending is allowed.
    const rt = TestBed.inject(RealtimeService) as unknown as {
      connectionS: WritableSignal<string>;
    };
    rt.connectionS.set('open');
    fixture = TestBed.createComponent(ComposerComponent);
    fixture.detectChanges();
    cmp = fixture.componentInstance as unknown as ComposerInternals;
    ta = (fixture.nativeElement as HTMLElement).querySelector('textarea')!;
  });

  afterEach(() => setActiveMock(null));

  it('keeps text typed while a send is in flight', async () => {
    let answer!: (r: Reply) => void;
    backend.routes.set('/sessions/s1/message', () => new Promise<Reply>((r) => (answer = r)));
    type('first message');
    const sent = cmp.submit('send');
    await flush();
    type('first message\nsecond thought');
    answer({ status: 200, body: { delivery: 'sent', messageId: 'm1' } });
    await sent;
    expect(cmp.text()).toBe('second thought');
  });

  it('ignores a /commands answer that arrives after the text moved on', async () => {
    let answer!: (r: Reply) => void;
    backend.routes.set('/sessions/s1/commands', () => new Promise<Reply>((r) => (answer = r)));
    type('/');
    await flush();
    type('');
    type('hello');
    answer({ status: 200, body: { commands: [{ name: 'clear' }] } });
    await flush();
    expect(cmp.palette()).toBeNull();
  });

  it('sends to a session that resumes by itself as to an idle one', () => {
    const ctx = TestBed.inject(SessionContext);
    const at = (status: string, kind: string) =>
      ctx.detail.set({
        id: 's1',
        live: true,
        status,
        activity: { kind, label: '' },
        pending: [],
        control: { message: 'direct', slashCommands: true, limits: [] } as unknown as ControlCaps,
      } as unknown as SessionDetail);
    // Between turns, its background work or a wakeup resumes it later:
    // not in a turn, so a message goes in now rather than in the queue.
    at('background', 'background');
    expect(ctx.busy()).toBeFalse();
    at('background', 'scheduled');
    expect(ctx.busy()).toBeFalse();
    at('working', 'tool');
    expect(ctx.busy()).toBeTrue();
    at('idle', 'idle');
    expect(ctx.busy()).toBeFalse();
  });

  it('sends while Claude works, with Send now for at once', async () => {
    const ctx = TestBed.inject(SessionContext);
    const detail = (queuedMessages: unknown[] = []) =>
      ctx.detail.set({
        id: 's1',
        live: true,
        status: 'working',
        activity: { kind: 'tool', label: 'Running a command' },
        pending: [],
        queuedMessages,
        control: {
          channel: 'owned',
          message: 'direct',
          interrupt: 'immediate',
          slashCommands: true,
          limits: [],
        } as unknown as ControlCaps,
      } as unknown as SessionDetail);
    detail();
    backend.routes.set('/sessions/s1/message', () => ({
      status: 200,
      body: { delivery: 'queued', messageId: 'q1' },
    }));
    backend.routes.set('/sessions/s1/queue/q1/send-now', () => ({
      status: 200,
      body: { ok: true, sendNow: 'stopped' },
    }));
    backend.routes.set('/sessions/s1', () => ({ status: 200, body: { id: 's1' } }));
    type('also check the tests');
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    const text = (e: Element | null) => (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
    // Not a queue: Claude reads it after its current step.
    expect(text(el.querySelector('.send'))).toBe('Send');
    expect(text(el.querySelector('.under .hint'))).toBe(
      'Claude reads it after its current step · Send now for at once',
    );
    const now = [...el.querySelectorAll<HTMLButtonElement>('.under .btn')].find(
      (b) => text(b) === 'Send now',
    );
    expect(now).toBeDefined();
    now!.click();
    await flush();
    expect(backend.calls).toContain('POST /sessions/s1/message');
    expect(backend.calls).toContain('POST /sessions/s1/queue/q1/send-now');

    // A message that waits says when Claude reads it, and offers Send now.
    detail([
      {
        id: 'q2',
        text: 'and the docs',
        createdAt: 1,
        via: 'owned',
        status: 'queued',
        cancellable: true,
      },
    ]);
    fixture.detectChanges();
    const row = el.querySelector('.queued li');
    expect(text(row?.querySelector('.qstate') ?? null)).toBe('after its current step');
    (row?.querySelector('[aria-label="Send now: and the docs"]') as HTMLButtonElement).click();
    await flush();
    expect(backend.calls).toContain('POST /sessions/s1/queue/q2/send-now');
  });

  it('opens the palette when the answer is still current', async () => {
    backend.routes.set('/sessions/s1/commands', () => ({
      status: 200,
      body: { commands: [{ name: 'clear' }] },
    }));
    type('/');
    await flush();
    expect(cmp.palette()).toBe('slash');
  });
});
