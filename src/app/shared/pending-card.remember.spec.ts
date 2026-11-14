import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { hostOsSignal } from '../core/i18n/i18n';
import {
  FakeBackend,
  RouteReply,
  asDesktop,
  flush,
  me,
  useFakeServer,
} from '../core/mock/fake-server.testing';
import { setActiveMock } from '../core/mock/mock-api';
import {
  PEER_ID,
  goLive,
  pairedMachines,
  remoteList,
  remoteSession,
} from '../core/mock/remote.testing';
import { GrantOffer, PendingItem } from '../core/models';
import { RealtimeService } from '../core/realtime.service';
import { PendingCardComponent } from './pending-card.component';
import { RememberPanelComponent } from './remember-panel.component';

/** What Mother Claude can remember of a Bash command in `shop`. */
const offer: GrantOffer = {
  kind: 'tool',
  tool: 'Bash',
  patterns: 'command',
  suggested: ['cargo test'],
  session: 's1',
  sessionName: 'tests',
  project: '/Users/me/dev/shop',
  projectName: 'shop',
};

const bash = (over: Partial<PendingItem> = {}): PendingItem => ({
  id: 'req-bash',
  sessionId: 's1',
  kind: 'permission',
  source: 'owned',
  toolName: 'Bash',
  title: 'Allow Bash?',
  input: { command: 'cargo test --lib' },
  suggestions: [
    {
      index: 0,
      label: 'Always allow “cargo test:*” in this project',
      kind: 'addRules',
      destination: 'localSettings',
      standing: true,
      raw: {},
    },
  ],
  dangerous: false,
  answerable: true,
  createdAt: Date.now(),
  grant: offer,
  ...over,
});

/** *Allow for…* on an approval card: remembering the answer. */
describe('PendingCardComponent: Allow for…', () => {
  let backend: FakeBackend;
  let reply: RouteReply;
  let fixture: ComponentFixture<PendingCardComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    reply = { status: 200, body: {} };
    backend.routes.set('/sessions/s1/respond', () => reply);
    useFakeServer(backend, [provideRouter([])]);
    hostOsSignal.set('macos');
  });

  afterEach(() => {
    fixture?.destroy();
    TestBed.inject(RealtimeService).stop();
    hostOsSignal.set(undefined);
    setActiveMock(null);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const el = () => fixture.nativeElement as HTMLElement;
  const buttons = () => [...el().querySelectorAll<HTMLButtonElement>('button')];
  const button = (label: string) => buttons().find((b) => text(b).startsWith(label));
  const click = async (label: string) => {
    const b = button(label);
    expect(b, label).toBeTruthy();
    expect(b?.disabled, `${label} is enabled`).toBe(false);
    b!.click();
    await flush();
    fixture.detectChanges();
  };
  const choices = () =>
    [...el().querySelectorAll<HTMLButtonElement>('mc-remember-panel .choice')].map((b) => ({
      label: text(b.querySelector('.clabel')),
      hint: text(b.querySelector('.chint')),
      enabled: !b.disabled,
    }));
  const sent = () => backend.bodyOf('POST /sessions/s1/respond');
  /** A live card; `desktop`: this is the desktop app (the dangerous
   * right), else a phone without it. */
  const render = async (item: PendingItem, desktop = true) => {
    const rt = TestBed.inject(RealtimeService);
    await rt.start();
    backend.last.open();
    backend.last.frame({ t: 'hello', v: 2, me: me(), serverTime: Date.now() });
    if (desktop) asDesktop({ hostOs: 'macos' });
    fixture = TestBed.createComponent(PendingCardComponent);
    fixture.componentRef.setInput('item', item);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
  };

  it('offers Allow for… beside Allow once, worded for a Bash command in a project', async () => {
    await render(bash());
    expect(buttons().map((b) => text(b))).toEqual([
      'Allow once',
      'Allow for…',
      'Deny',
      'Deny with feedback…',
    ]);
    await click('Allow for…');
    expect(choices()).toEqual([
      {
        label: 'In this session',
        hint: 'Any Bash command, in this session only.',
        enabled: true,
      },
      {
        label: 'In shop (every session)',
        hint: 'Any Bash command, in every session of shop.',
        enabled: true,
      },
      {
        label: 'Everywhere on this Mac',
        hint: 'Any Bash command, in every project on this Mac.',
        enabled: true,
      },
    ]);
    const panel = el().querySelector('mc-remember-panel')!;
    expect(text(panel)).toContain('Dangerous actions always ask.');
    expect(panel.querySelector('a')?.getAttribute('href')).toBe('/permissions');
    expect(text(panel.querySelector('a'))).toBe('Change later in Permissions');
    // Claude Code's own suggestion, below.
    expect(text(panel.querySelector('.cc'))).toContain('Or save it in Claude Code’s own settings:');
    expect(text(panel.querySelector('.cc'))).toContain(
      'Always allow “cargo test:*” in this project',
    );
    // The first choice takes the focus.
    await fixture.whenStable();
    expect(document.activeElement).toBe(panel.querySelector('.choice'));

    await click('In shop (every session)');
    expect(sent()).toEqual({
      requestId: 'req-bash',
      decision: 'allow',
      remember: { scope: 'project' },
    });
    expect(text(el().querySelector('.done'))).toBe(
      'Allowed and remembered: In shop (every session)',
    );
  });

  it('sends each one-tap choice, and the Claude Code suggestion as before', async () => {
    for (const [label, scope] of [
      ['In this session', 'session'],
      ['Everywhere on this Mac', 'computer'],
    ] as const) {
      await render(bash({ id: `req-${scope}` }));
      await click('Allow for…');
      await click(label);
      expect(sent()).toEqual({ requestId: `req-${scope}`, decision: 'allow', remember: { scope } });
      fixture.destroy();
      TestBed.inject(RealtimeService).stop();
    }
    await render(bash({ id: 'req-cc' }));
    await click('Allow for…');
    await click('Always allow “cargo test:*”');
    expect(sent()).toEqual({ requestId: 'req-cc', decision: 'allow', suggestion: 0 });
  });

  it('narrows it to patterns: prefilled, rows added and removed, a scope chosen', async () => {
    await render(bash());
    await click('Allow for…');
    await click('Only matching…');
    await fixture.whenStable();
    const panel = el().querySelector('mc-remember-panel')!;
    expect(text(panel.querySelector('.ptitle'))).toBe('Only commands that start with:');
    expect(text(panel.querySelector('.hint'))).toContain('cargo test covers cargo test --lib');
    const inputs = () => [...panel.querySelectorAll<HTMLInputElement>('.prow input')];
    expect(inputs().map((i) => i.value)).toEqual(['cargo test']);
    expect(document.activeElement).toBe(inputs()[0]);
    // One row can't be removed; two can.
    expect(panel.querySelector<HTMLButtonElement>('.prow button')?.disabled).toBe(true);
    await click('Add a pattern');
    await click('Add a pattern');
    await fixture.whenStable();
    expect(inputs().length).toBe(3);
    const type = async (input: HTMLInputElement, value: string) => {
      input.value = value;
      input.dispatchEvent(new Event('input'));
      await flush();
      fixture.detectChanges();
    };
    await type(inputs()[1], '  npm run lint ');
    await type(inputs()[2], 'make');
    panel.querySelectorAll<HTMLButtonElement>('.prow button')[2].click();
    await flush();
    fixture.detectChanges();
    expect(inputs().map((i) => i.value)).toEqual(['cargo test', '  npm run lint ']);
    // The desktop starts from the project; everywhere instead.
    const radios = [...panel.querySelectorAll<HTMLInputElement>('.scopes input')];
    expect(radios.map((r) => text(r.closest('label')))).toEqual([
      'In this session',
      'In shop (every session)',
      'Everywhere on this Mac',
    ]);
    expect(radios[1].checked).toBe(true);
    radios[2].click();
    fixture.detectChanges();
    await click('Allow and remember');
    expect(sent()).toEqual({
      requestId: 'req-bash',
      decision: 'allow',
      remember: { scope: 'computer', patterns: ['cargo test', 'npm run lint'] },
    });
    expect(text(el().querySelector('.done'))).toBe(
      'Allowed and remembered: Everywhere on this Mac, matching cargo test, npm run lint',
    );
  });

  it('keeps a project or everywhere for the dangerous right; this session is anyone’s', async () => {
    await render(bash(), false);
    await click('Allow for…');
    expect(choices().map((c) => [c.label, c.enabled])).toEqual([
      ['In this session', true],
      ['In shop (every session)', false],
      ['Everywhere on this Mac', false],
    ]);
    expect(text(el().querySelector('mc-remember-panel .why'))).toBe(
      'To remember it for a project or everywhere, answer on the Mac, or let this device take dangerous actions (Settings › Phone & devices).',
    );
    // The suggestion saved to a settings file needs it too.
    const cc = el().querySelector<HTMLButtonElement>('.cc .suggestion');
    expect(cc?.disabled).toBe(true);
    await click('Only matching…');
    const radios = [...el().querySelectorAll<HTMLInputElement>('.scopes input')];
    expect(radios.map((r) => [r.checked, r.disabled])).toEqual([
      [true, false],
      [false, true],
      [false, true],
    ]);
    await click('Allow and remember');
    expect(sent()).toEqual({
      requestId: 'req-bash',
      decision: 'allow',
      remember: { scope: 'session', patterns: ['cargo test'] },
    });
  });

  it("shows the server's refusal, the patterns still there to fix", async () => {
    reply = {
      status: 422,
      body: {
        error: {
          code: 'invalid_pattern',
          message:
            '“cargo test && rm” can’t hold & | ; or a backtick: a command with one always asks.',
        },
      },
    };
    await render(bash());
    await click('Allow for…');
    await click('Only matching…');
    await click('Allow and remember');
    expect(text(el().querySelector('[role="alert"]'))).toContain('can’t hold');
    expect(el().querySelector('.prow input')).not.toBeNull();
    expect(el().querySelector('.done')).toBeNull();

    reply = { status: 403, body: { error: { code: 'dangerous_blocked', message: 'No.' } } };
    await click('Back');
    await click('Everywhere on this Mac');
    expect(text(el().querySelector('[role="alert"]'))).toContain(
      'or allow dangerous actions for this device',
    );
  });

  it('allows once only, saying why, when nothing can be remembered', async () => {
    await render(bash({ grant: { ...offer, blocked: 'Dangerous actions always ask.' } }));
    expect(buttons().map((b) => text(b))).toEqual(['Allow once', 'Deny', 'Deny with feedback…']);
    expect(text(el().querySelector('.once'))).toBe(
      'Can’t be remembered: Dangerous actions always ask.',
    );
  });

  it('keeps Always… for a request Mother Claude offers nothing for', async () => {
    await render(bash({ grant: undefined }));
    expect(button('Allow for…')).toBeUndefined();
    await click('Always…');
    expect(text(el())).toContain('Saved in .claude/settings.local.json');
  });

  it('opens with Shift+A, and Escape goes back a step at a time', async () => {
    await render(bash());
    const host = el();
    host.focus();
    host.dispatchEvent(new KeyboardEvent('keydown', { key: 'A', shiftKey: true, bubbles: true }));
    fixture.detectChanges();
    await fixture.whenStable();
    expect(el().querySelector('mc-remember-panel')).not.toBeNull();
    await click('Only matching…');
    await fixture.whenStable();
    const escape = () => {
      (document.activeElement ?? host).dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }),
      );
      fixture.detectChanges();
    };
    escape();
    await fixture.whenStable();
    expect(el().querySelector('.prow')).toBeNull();
    expect(choices().length).toBe(3);
    escape();
    expect(el().querySelector('mc-remember-panel')).toBeNull();
    expect(document.activeElement).toBe(host);
  });
});

describe('RememberPanelComponent: another computer’s request', () => {
  beforeEach(() => {
    TestBed.configureTestingModule({ providers: [provideRouter([])] });
    hostOsSignal.set('macos');
  });
  afterEach(() => hostOsSignal.set(undefined));

  it('remembers it on that computer, changed there later', () => {
    const fixture = TestBed.createComponent(RememberPanelComponent);
    fixture.componentRef.setInput('offer', { ...offer, project: undefined });
    fixture.componentRef.setInput('host', 'DESKTOP-PC');
    fixture.componentRef.setInput('canDangerous', false);
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    const text = (e: Element | null) => (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
    // No project folder: no project to choose.
    expect([...el.querySelectorAll('.clabel')].map(text)).toEqual([
      'In this session',
      'Everywhere on DESKTOP-PC',
    ]);
    expect(text(el.querySelector('.why'))).toBe(
      'To remember it for a project or everywhere, answer at DESKTOP-PC, or on a computer it allows dangerous actions.',
    );
    expect(text(el.querySelector('.foot'))).toBe(
      'Dangerous actions always ask. Change it later in Permissions on DESKTOP-PC.',
    );
    expect(el.querySelector('.foot a')).toBeNull();
  });
});

describe('PendingCardComponent: Allow for… on a session of another computer', () => {
  const PROXY = `/machines/${PEER_ID}/proxy`;
  let backend: FakeBackend;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    backend.routes.set(`${PROXY}/sessions/r1/respond`, () => ({ status: 200, body: {} }));
    hostOsSignal.set('macos');
  });

  afterEach(() => {
    TestBed.inject(RealtimeService).stop();
    hostOsSignal.set(undefined);
    setActiveMock(null);
  });

  it('says everywhere on that computer, as its offer names it, and answers through it', async () => {
    pairedMachines();
    const socket = await goLive(backend);
    remoteList(socket, PEER_ID, [remoteSession('r1', { status: 'waiting' })]);
    asDesktop({ hostOs: 'macos' });
    const fixture = TestBed.createComponent(PendingCardComponent);
    fixture.componentRef.setInput('item', {
      ...bash(),
      sessionId: 'r1',
      grant: { ...offer, session: 'r1', machineName: 'Studio PC' },
    });
    fixture.componentRef.setInput('machine', PEER_ID);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    const text = (e: Element | null) => (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
    const button = (label: string) =>
      [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));
    button('Allow for…')!.click();
    fixture.detectChanges();
    expect([...el.querySelectorAll('.clabel')].map(text)).toEqual([
      'In this session',
      'In shop (every session)',
      'Everywhere on Studio PC',
    ]);
    expect(text(el.querySelector('mc-remember-panel .foot'))).toBe(
      'Dangerous actions always ask. Change it later in Permissions on Studio PC.',
    );
    button('Everywhere on Studio PC')!.click();
    await flush();
    fixture.detectChanges();
    expect(backend.bodyOf(`POST ${PROXY}/sessions/r1/respond`)).toEqual({
      requestId: 'req-bash',
      decision: 'allow',
      remember: { scope: 'computer' },
    });
    expect(text(el.querySelector('.done'))).toBe('Allowed and remembered: Everywhere on Studio PC');
    fixture.destroy();
  });
});
