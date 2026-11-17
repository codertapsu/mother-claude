import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { useLanguage } from '../core/i18n/i18n';
import { FakeBackend, asDesktop, flush, me, useFakeServer } from '../core/mock/fake-server.testing';
import { setActiveMock } from '../core/mock/mock-api';
import { GrantOffer, PendingItem } from '../core/models';
import { RealtimeService } from '../core/realtime.service';
import { sessionTalkDetail, sessionTalkInput } from '../core/session-talk';
import { PendingCardComponent, pendingTitle } from './pending-card.component';

const OTHER = 'f6a178a5-8c73-41cc-a6b9-908469891531';

/** What Mother Claude can remember of the answer: talking to sessions. */
const offer: GrantOffer = {
  kind: 'sessions',
  session: 's1',
  sessionName: 'Docs writer',
  project: '/Users/me/dev/app',
  projectName: 'app',
};

/** A message to another session, as `mcp::sessions::approve` raises it. */
const send = (over: Partial<PendingItem> = {}): PendingItem => ({
  id: 'req-send',
  sessionId: 's1',
  kind: 'permission',
  source: 'session',
  toolName: 'session_send',
  title: 'Message the session “Video dubbing” on DESKTOP-PC',
  detail: '“Docs writer” sends: Please check the French subtitles.',
  input: {
    action: 'send',
    session: OTHER,
    sessionName: 'Video dubbing',
    sessionProject: 'multilingual-dubbed-video',
    folder: 'D:\\development\\projects\\multilingual-dubbed-video',
    computer: 'DESKTOP-PC',
    machine: '0123456789abcdef0123456789abcdef',
    here: false,
    live: true,
    status: 'idle',
    permissionMode: 'default',
    message: 'Please check the French subtitles.',
    from: 'Docs writer',
    fromSession: 's1',
    project: '/Users/me/dev/app',
    projectName: 'app',
  },
  suggestions: [],
  dangerous: false,
  answerable: true,
  createdAt: Date.now(),
  grant: offer,
  ...over,
});

describe('PendingCardComponent: a session working with another', () => {
  let backend: FakeBackend;

  beforeEach(() => {
    backend = new FakeBackend();
    backend.routes.set('/sessions/s1/respond', () => ({ status: 200, body: { ok: true } }));
    useFakeServer(backend, [provideRouter([])]);
  });

  afterEach(() => {
    TestBed.inject(RealtimeService).stop();
    setActiveMock(null);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const buttons = (el: HTMLElement) => [...el.querySelectorAll<HTMLButtonElement>('button')];
  const button = (el: HTMLElement, label: string) =>
    buttons(el).find((b) => text(b).startsWith(label));
  const render = (item: PendingItem) => {
    const fixture = TestBed.createComponent(PendingCardComponent);
    fixture.componentRef.setInput('item', item);
    fixture.detectChanges();
    return { fixture, el: fixture.nativeElement as HTMLElement };
  };
  const goLive = async () => {
    const rt = TestBed.inject(RealtimeService);
    await rt.start();
    backend.last.open();
    backend.last.frame({ t: 'hello', v: 2, me: me(), serverTime: Date.now() });
    await flush();
  };
  const choices = (el: HTMLElement) =>
    [...el.querySelectorAll<HTMLButtonElement>('mc-remember-panel .choice')].map((b) => ({
      label: text(b.querySelector('.clabel')),
      enabled: !b.disabled,
    }));

  it('names the session, where it is, and the message', () => {
    const { el } = render(send());
    expect(text(el.querySelector('.title'))).toBe(
      'Message the session “Video dubbing” on DESKTOP-PC?',
    );
    const body = text(el.querySelector('mc-session-talk-action'));
    expect(body).toContain('Video dubbing');
    expect(body).toContain('running');
    expect(body).toContain(OTHER);
    expect(body).toContain('multilingual-dubbed-video');
    expect(body).toContain('DESKTOP-PC');
    expect(body).toContain('Please check the French subtitles.');
    expect(body).toContain('saying which session sent it');
    expect(text(el)).toContain('another session');
  });

  it('words a read, a question and a start', () => {
    let el = render(
      send({
        id: 'req-read',
        toolName: 'session_read',
        input: { ...(send().input as object), action: 'read', message: undefined, last: 5 },
      }),
    ).el;
    expect(text(el.querySelector('.title'))).toBe(
      'Read the session “Video dubbing” on DESKTOP-PC?',
    );
    expect(text(el.querySelector('mc-session-talk-action'))).toContain('Its last 5 messages');
    el = render(
      send({
        id: 'req-ask',
        input: { ...(send().input as object), action: 'ask', here: true, live: false },
      }),
    ).el;
    expect(text(el.querySelector('.title'))).toBe('Ask the session “Video dubbing”?');
    const body = text(el.querySelector('mc-session-talk-action'));
    expect(body).toContain('this session waits for the answer');
    expect(body).toContain('It is not running');
    el = render(
      send({
        id: 'req-start',
        input: {
          action: 'start',
          sessionProject: 'app',
          folder: '/Users/me/dev/app',
          computer: 'Studio',
          here: true,
          message: 'Write the release notes.',
        },
      }),
    ).el;
    expect(text(el.querySelector('.title'))).toBe('Start a session in app?');
    expect(text(el.querySelector('mc-session-talk-action'))).toContain('Write the release notes.');
  });

  it('remembers talking to sessions from this session for anyone, wider with the dangerous right', async () => {
    await goLive();
    let { el, fixture } = render(send());
    button(el, 'Allow for…')!.click();
    fixture.detectChanges();
    expect(choices(el)).toEqual([
      { label: 'From this session', enabled: true },
      { label: 'From app', enabled: false },
      // It applies where the asking session runs: this computer.
      { label: 'From every session on this computer', enabled: false },
    ]);
    expect(button(el, 'Only matching…')).toBeUndefined();
    button(el, 'From this session')!.click();
    await flush();
    fixture.detectChanges();
    expect(backend.bodyOf('POST /sessions/s1/respond')).toEqual({
      requestId: 'req-send',
      decision: 'allow',
      remember: { scope: 'session' },
    });
    fixture.destroy();

    asDesktop();
    ({ el, fixture } = render(send({ id: 'req-send-2' })));
    button(el, 'Allow for…')!.click();
    fixture.detectChanges();
    expect(choices(el).every((c) => c.enabled)).toBe(true);
  });

  it('marks a message to a session without prompts dangerous, never remembered', async () => {
    await goLive();
    asDesktop();
    const { el } = render(
      send({
        dangerous: true,
        dangerousReason: '“Video dubbing” runs without permission prompts (bypassPermissions).',
        input: { ...(send().input as object), permissionMode: 'bypassPermissions' },
        grant: {
          ...offer,
          blocked: 'Talking to a session that runs without permission prompts always asks.',
        },
      }),
    );
    expect(text(el)).toContain('dangerous');
    expect(text(el)).toContain('runs without permission prompts');
    expect(button(el, 'Allow once')?.disabled).toBe(false);
    expect(button(el, 'Allow for…')).toBeUndefined();
    expect(el.querySelector('mc-session-talk-action .warn')).not.toBeNull();
  });

  it("reads the server's title and detail in the language shown", async () => {
    expect(pendingTitle('Ask the session “Video dubbing”')).toBe(
      'Ask the session “Video dubbing”?',
    );
    expect(pendingTitle('Start a session in app on DESKTOP-PC')).toBe(
      'Start a session in app on DESKTOP-PC?',
    );
    expect(sessionTalkDetail('“Docs writer” wants to read its last 10 messages.')).toBe(
      '“Docs writer” wants to read its last 10 messages.',
    );
    try {
      await useLanguage('vi');
      expect(pendingTitle('Message the session “Video dubbing” on DESKTOP-PC')).toBe(
        'Nhắn cho phiên “Video dubbing” trên DESKTOP-PC?',
      );
      expect(sessionTalkDetail('“Docs writer” sends: hi')).toBe('“Docs writer” gửi: hi');
    } finally {
      await useLanguage('en');
    }
    expect(sessionTalkDetail('Something else.')).toBe('Something else.');
    expect(sessionTalkInput({ action: 'dance' })).toBeNull();
    expect(sessionTalkInput(null)).toBeNull();
  });
});
