import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { useLanguage } from '../core/i18n/i18n';
import { FakeBackend, asDesktop, flush, me, useFakeServer } from '../core/mock/fake-server.testing';
import { setActiveMock } from '../core/mock/mock-api';
import { GrantOffer, PendingItem } from '../core/models';
import { RealtimeService } from '../core/realtime.service';
import { PendingCardComponent, pendingTitle, runDetail } from './pending-card.component';

const MACHINE = '0123456789abcdef0123456789abcdef';

/** What Mother Claude can remember of a run's answer (the server's offer). */
const runOffer: GrantOffer = {
  kind: 'run',
  patterns: 'command',
  suggested: ['cargo test'],
  session: 's1',
  sessionName: 'Windows build',
  project: '/Users/me/dev/app',
  projectName: 'app',
  machine: MACHINE,
  machineName: 'DESKTOP-PC',
};

/** A run approval as `machines::work::source::approve` raises it. */
const run = (over: Partial<PendingItem> = {}): PendingItem => ({
  id: 'req-run',
  sessionId: 's1',
  kind: 'permission',
  source: 'machine',
  toolName: 'machine_run',
  title: 'Run on DESKTOP-PC: cargo test --lib',
  detail: "In D:\\dev\\app on DESKTOP-PC (PowerShell 7), in a work folder at this session's code.",
  input: {
    machine: MACHINE,
    machineName: 'DESKTOP-PC',
    os: 'windows',
    shell: 'pwsh',
    shellLabel: 'PowerShell 7',
    checkout: 'D:\\dev\\app',
    folder: 'C:\\Users\\me\\AppData\\Roaming\\mc\\machines\\work\\wk_0123456789abcdef',
    cwd: 'src-tauri',
    command: 'cargo test --lib',
    timeoutSecs: 1800,
    artifacts: 'target/release/*.exe',
    project: '/Users/me/dev/app',
    projectName: 'app',
  },
  suggestions: [],
  dangerous: false,
  answerable: true,
  createdAt: Date.now(),
  grant: runOffer,
  ...over,
});

/** A task handed to a session there (`machines::work::tasks::source`). */
const task = (over: Partial<PendingItem> = {}): PendingItem => ({
  ...run(),
  id: 'req-task',
  toolName: 'machine_task',
  title: 'Hand a task to DESKTOP-PC: Fix the installer.',
  detail: "In its own worktree of D:\\dev\\app on DESKTOP-PC, at this session's code.",
  input: {
    kind: 'task',
    machine: MACHINE,
    machineName: 'DESKTOP-PC',
    os: 'windows',
    checkout: 'D:\\dev\\app',
    cwd: '',
    task: 'Fix the installer.',
    firstLine: 'Fix the installer.',
    permissionMode: 'acceptEdits',
    project: '/Users/me/dev/app',
    projectName: 'app',
  },
  grant: { ...runOffer, kind: 'task', patterns: undefined, suggested: undefined },
  ...over,
});

describe('PendingCardComponent: a command to run on another computer', () => {
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
  /** Answers need a live connection (its `hello` says who this is: a
   * phone; `asDesktop()` after it for the desktop). */
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

  it('names the computer, the folder there and the command', () => {
    const { el } = render(run());
    expect(text(el.querySelector('.title'))).toBe('Run a command on DESKTOP-PC?');
    const body = text(el.querySelector('mc-machine-run-action'));
    expect(body).toContain('DESKTOP-PC');
    expect(body).toContain('Windows');
    expect(body).toContain('D:\\dev\\app › src-tauri');
    expect(body).toContain('cargo test --lib');
    expect(body).toContain('PowerShell 7');
    expect(body).toContain('30 min');
    expect(body).toContain('target/release/*.exe');
    expect(text(el)).toContain('on another computer');
  });

  it('remembers commands from this session for anyone, the project and every command with the dangerous right', async () => {
    // A phone without the dangerous right.
    await goLive();
    let { el, fixture } = render(run());
    expect(buttons(el).map((b) => text(b))).toEqual([
      'Allow once',
      'Allow for…',
      'Deny',
      'Deny with feedback…',
    ]);
    button(el, 'Allow for…')!.click();
    fixture.detectChanges();
    expect(choices(el)).toEqual([
      { label: 'Commands from this session', enabled: true },
      { label: 'Commands from app', enabled: false },
      { label: 'Every command on DESKTOP-PC', enabled: false },
    ]);
    expect(text(el.querySelector('mc-remember-panel'))).toContain(
      'To remember it for a project or everywhere',
    );
    button(el, 'Commands from this session')!.click();
    await flush();
    fixture.detectChanges();
    expect(backend.bodyOf('POST /sessions/s1/respond')).toEqual({
      requestId: 'req-run',
      decision: 'allow',
      remember: { scope: 'session' },
    });
    expect(text(el.querySelector('.done'))).toBe(
      'Allowed and remembered: Commands from this session',
    );
    fixture.destroy();

    // The desktop: commands that start with `cargo test`, from the project.
    asDesktop();
    ({ el, fixture } = render(run({ id: 'req-run-2' })));
    button(el, 'Allow for…')!.click();
    fixture.detectChanges();
    expect(choices(el).every((c) => c.enabled)).toBe(true);
    button(el, 'Only matching…')!.click();
    fixture.detectChanges();
    await fixture.whenStable();
    const rows = [...el.querySelectorAll<HTMLInputElement>('.prow input')];
    expect(rows.map((r) => r.value)).toEqual(['cargo test']);
    const checked = el.querySelector<HTMLInputElement>('.scopes input:checked');
    expect(text(checked?.closest('label'))).toBe('From app');
    button(el, 'Allow and remember')!.click();
    await flush();
    fixture.detectChanges();
    expect(backend.bodyOf('POST /sessions/s1/respond')).toEqual({
      requestId: 'req-run-2',
      decision: 'allow',
      remember: { scope: 'project', patterns: ['cargo test'] },
    });
  });

  it('remembers tasks too, without patterns', async () => {
    await goLive();
    asDesktop();
    const { el, fixture } = render(task());
    expect(text(el.querySelector('.title'))).toBe('Hand a task to DESKTOP-PC?');
    button(el, 'Allow for…')!.click();
    fixture.detectChanges();
    expect(choices(el).map((c) => c.label)).toEqual([
      'Tasks from this session',
      'Tasks from app',
      'Every task handed to DESKTOP-PC',
    ]);
    expect(button(el, 'Only matching…')).toBeUndefined();
    button(el, 'Every task handed to DESKTOP-PC')!.click();
    await flush();
    expect(backend.bodyOf('POST /sessions/s1/respond')).toEqual({
      requestId: 'req-task',
      decision: 'allow',
      remember: { scope: 'computer' },
    });
  });

  it('marks a dangerous command, blocks Allow without the right, and never remembers it', async () => {
    await goLive();
    const dangerous = run({
      dangerous: true,
      dangerousReason: 'The command touches an MCP server configuration.',
      grant: { ...runOffer, blocked: 'Dangerous commands always ask.' },
    });
    const phone = render(dangerous);
    let el = phone.el;
    expect(text(el)).toContain('dangerous');
    expect(text(el)).toContain('touches an MCP server configuration');
    expect(button(el, 'Allow once')?.disabled).toBe(true);
    expect(button(el, 'Allow for…')).toBeUndefined();
    phone.fixture.destroy();

    // The desktop may allow it once; nothing is remembered.
    asDesktop();
    el = render(dangerous).el;
    expect(button(el, 'Allow once')?.disabled).toBe(false);
    expect(button(el, 'Allow for…')).toBeUndefined();
    expect(button(el, 'Always…')).toBeUndefined();
    expect(text(el.querySelector('.once'))).toBe(
      'Can’t be remembered: Dangerous commands always ask.',
    );
  });

  it("reads the server's title and detail in the language shown", async () => {
    expect(pendingTitle('Run on DESKTOP-PC: cargo test')).toBe('Run on DESKTOP-PC: cargo test');
    try {
      await useLanguage('vi');
      expect(pendingTitle('Run on DESKTOP-PC: cargo test')).toBe(
        'Chạy trên DESKTOP-PC: cargo test',
      );
      expect(
        runDetail(
          "In D:\\dev\\app on DESKTOP-PC (PowerShell 7), in a work folder at this session's code.",
        ),
      ).toBe(
        'Trong D:\\dev\\app trên DESKTOP-PC (PowerShell 7), ở một thư mục làm việc với mã nguồn của phiên này.',
      );
    } finally {
      await useLanguage('en');
    }
    expect(runDetail('Something else.')).toBe('Something else.');
  });
});
