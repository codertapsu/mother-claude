import { WritableSignal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { useLanguage } from '../core/i18n/i18n';
import { FakeBackend, asDesktop, flush, useFakeServer } from '../core/mock/fake-server.testing';
import { setActiveMock } from '../core/mock/mock-api';
import { PendingItem, PermissionSuggestion } from '../core/models';
import { PendingCardComponent, pendingTitle, runDetail } from './pending-card.component';

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
    machine: '0123456789abcdef0123456789abcdef',
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
  suggestions: [
    {
      index: 0,
      label: 'Always allow `cargo test` on DESKTOP-PC for this project',
      kind: 'machineRunGrant',
      destination: 'machines',
      standing: true,
      raw: { type: 'machineRunGrant', prefix: 'cargo test' },
    },
  ],
  dangerous: false,
  answerable: true,
  createdAt: Date.now(),
  ...over,
});

describe('PendingCardComponent: a command to run on another computer', () => {
  let backend: FakeBackend;

  beforeEach(() => {
    backend = new FakeBackend();
    backend.routes.set('/sessions/s1/respond', () => ({ status: 200, body: { ok: true } }));
    useFakeServer(backend, [provideRouter([])]);
  });

  afterEach(() => setActiveMock(null));

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const buttons = (el: HTMLElement) => [...el.querySelectorAll<HTMLButtonElement>('button')];
  const render = (item: PendingItem) => {
    const fixture = TestBed.createComponent(PendingCardComponent);
    fixture.componentRef.setInput('item', item);
    fixture.detectChanges();
    return { fixture, el: fixture.nativeElement as HTMLElement };
  };

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

  it('offers to always allow the command only to whoever has the dangerous right', async () => {
    // A phone without it: allow once or deny, never "always".
    let { el, fixture } = render(run());
    let labels = buttons(el).map((b) => text(b));
    expect(labels).toContain('Allow once');
    expect(labels).not.toContain('Always…');
    fixture.destroy();

    // The desktop: "Always…" opens the standing grant, saved in Machines.
    asDesktop();
    ({ el, fixture } = render(run()));
    labels = buttons(el).map((b) => text(b));
    expect(labels).toContain('Always…');
    // (The buttons wait for a live connection; the card's own actions
    // are what they call.)
    const card = fixture.componentInstance as unknown as {
      panel: WritableSignal<string>;
      allowAlways(s: PermissionSuggestion): Promise<void>;
    };
    card.panel.set('always');
    fixture.detectChanges();
    const always = buttons(el).find((b) => text(b).startsWith('Always allow “cargo test”'));
    expect(always).toBeTruthy();
    expect(text(always)).toContain('Machines › Always allowed runs');
    await card.allowAlways(run().suggestions[0]);
    await flush();
    expect(backend.bodyOf('POST /sessions/s1/respond')).toEqual({
      requestId: 'req-run',
      decision: 'allow',
      suggestion: 0,
    });
  });

  it('marks a dangerous command and blocks Allow without the right', () => {
    const { el } = render(
      run({
        dangerous: true,
        dangerousReason: 'The command touches an MCP server configuration.',
        suggestions: [],
      }),
    );
    expect(text(el)).toContain('dangerous');
    expect(text(el)).toContain('touches an MCP server configuration');
    const allow = buttons(el).find((b) => text(b) === 'Allow once');
    expect(allow?.disabled).toBe(true);
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
