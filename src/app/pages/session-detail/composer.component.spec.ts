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
