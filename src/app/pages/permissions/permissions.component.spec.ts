import { Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';

import { AuthService } from '../../core/auth.service';
import { hostOsSignal } from '../../core/i18n/i18n';
import {
  FakeBackend,
  RouteReply,
  asDesktop,
  flush,
  me,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { PermissionRule, PermissionsView } from '../../core/permission-rules';
import { RealtimeService } from '../../core/realtime.service';
import { PermissionsComponent, groupRules } from './permissions.component';

@Component({ template: '' })
class BlankComponent {}

const NOW = Date.now();
const M = 60_000;
const D = 24 * 60 * M;
const PEER = 'f'.repeat(32);

const rule = (id: string, over: Partial<PermissionRule>): PermissionRule => ({
  id,
  kind: 'tool',
  tool: 'Bash',
  patterns: [],
  scope: 'project',
  createdAt: NOW - D,
  createdBy: 'Mac',
  uses: 0,
  ...over,
});

/** One of each: this computer's tools in a project, a session and
 * everywhere; a run and a task on DESKTOP-PC. */
const RULES: PermissionRule[] = [
  rule('pr_0000000000a1', {
    patterns: ['npm test', 'npm run lint'],
    patternKind: 'command',
    project: '/Users/me/dev/api-server',
    projectName: 'api-server',
    createdAt: NOW - 3 * D,
    usedAt: NOW - 43 * M,
    uses: 9,
  }),
  rule('pr_0000000000a2', {
    tool: 'Edit',
    patterns: ['docs/**'],
    patternKind: 'path',
    scope: 'session',
    session: 's-auth',
    sessionName: 'auth-refactor',
    project: '/Users/me/dev/api-server',
    projectName: 'api-server',
    createdBy: "Marcus's iPhone",
    usedAt: NOW - 31 * M,
    uses: 1,
  }),
  rule('pr_0000000000a3', {
    tool: 'WebFetch',
    patterns: ['docs.rs', '*.github.com'],
    patternKind: 'domain',
    scope: 'computer',
  }),
  rule('pr_0000000000a4', {
    kind: 'run',
    tool: undefined,
    patterns: ['cargo test'],
    patternKind: 'command',
    project: '/Users/me/dev/mother-claude',
    projectName: 'mother-claude',
    machine: PEER,
    machineName: 'DESKTOP-PC',
    uses: 14,
    usedAt: NOW - 2 * M,
  }),
  rule('pr_0000000000a5', {
    kind: 'task',
    tool: undefined,
    scope: 'computer',
    machine: PEER,
    machineName: 'DESKTOP-PC',
  }),
];

describe('PermissionsComponent', () => {
  let backend: FakeBackend;
  let harness: RouterTestingHarness;
  let view: PermissionsView;
  let patchReply: ((body: unknown) => RouteReply) | null;

  beforeEach(async () => {
    backend = new FakeBackend();
    view = { rules: RULES.map((r) => ({ ...r })), paused: false };
    patchReply = null;
    backend.routes.set('/permissions', (method, body) => {
      if (method === 'PATCH') {
        if (patchReply) return patchReply(body);
        view = { ...view, paused: (body as { paused: boolean }).paused };
      }
      return { status: 200, body: view };
    });
    for (const r of RULES) {
      backend.routes.set(`/permissions/${r.id}`, (method, body) => {
        if (method === 'DELETE') {
          view = { ...view, rules: view.rules.filter((x) => x.id !== r.id) };
          return { status: 200, body: view };
        }
        if (patchReply) return patchReply(body);
        const saved = { ...view.rules.find((x) => x.id === r.id)!, ...(body as object) };
        view = { ...view, rules: view.rules.map((x) => (x.id === r.id ? saved : x)) };
        return { status: 200, body: saved };
      });
    }
    useFakeServer(backend, [
      provideRouter([
        { path: 'permissions', component: PermissionsComponent },
        { path: '**', component: BlankComponent },
      ]),
    ]);
    hostOsSignal.set('macos');
    asDesktop({ hostOs: 'macos' });
    harness = await RouterTestingHarness.create();
  });

  afterEach(() => {
    TestBed.inject(RealtimeService).stop();
    hostOsSignal.set(undefined);
    setActiveMock(null);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const settle = async () => {
    for (let i = 0; i < 4; i++) {
      await flush();
      harness.detectChanges();
    }
  };
  const open = async (url = '/permissions') => {
    await harness.navigateByUrl(url);
    await settle();
    return harness.routeNativeElement as HTMLElement;
  };
  const button = (el: Element | null | undefined, label: string) =>
    [...(el?.querySelectorAll<HTMLButtonElement>('button') ?? [])].find((b) =>
      text(b).startsWith(label),
    );
  const row = (el: HTMLElement, id: string) => el.querySelector<HTMLElement>(`[data-id="${id}"]`)!;
  const sheet = () => document.querySelector<HTMLDialogElement>('mc-rule-sheet dialog')!;

  it('groups them by computer, then where they apply, each in words', async () => {
    const el = await open();
    expect(text(el.querySelector('mc-page-head .lede'))).toContain(
      'What Claude may do without asking you. Anything else — and anything dangerous — still asks.',
    );
    const groups = [...el.querySelectorAll('section.where')];
    expect(groups.map((g) => text(g.querySelector('h2')))).toEqual([
      'On this Mac',
      'On DESKTOP-PC',
    ]);
    expect([...groups[0].querySelectorAll('h3')].map(text)).toEqual([
      'Everywhere on this Mac',
      'In api-server (every session)',
      'In the session auth-refactor',
    ]);
    expect([...groups[1].querySelectorAll('h3')].map(text)).toEqual([
      'From every project',
      'From mother-claude',
    ]);
    expect(text(row(el, 'pr_0000000000a3').querySelector('.what'))).toBe(
      'Fetching pages on docs.rs *.github.com',
    );
    expect([...row(el, 'pr_0000000000a1').querySelectorAll('.pat')].map((p) => text(p))).toEqual([
      'npm test',
      'npm run lint',
    ]);
    expect(text(row(el, 'pr_0000000000a1').querySelector('.meta'))).toBe(
      'Used 9 times, last 43m ago · added 3d ago by Mac',
    );
    expect(text(row(el, 'pr_0000000000a2').querySelector('.meta'))).toBe(
      "Used once, 31m ago · added 1d ago by Marcus's iPhone",
    );
    expect(text(row(el, 'pr_0000000000a3').querySelector('.meta'))).toContain('Not used yet');
    expect(text(row(el, 'pr_0000000000a4').querySelector('.what'))).toBe(
      'Commands on DESKTOP-PC that start with cargo test',
    );
    expect(text(row(el, 'pr_0000000000a5').querySelector('.what'))).toBe(
      'Any task handed to DESKTOP-PC',
    );
    expect(button(row(el, 'pr_0000000000a1'), 'Edit')?.getAttribute('aria-label')).toBe(
      'Edit Bash commands that start with npm test, npm run lint',
    );
  });

  it('shows one computer’s with the chips, from a link too', async () => {
    let el = await open('/permissions?on=others');
    const chips = () => [...el.querySelectorAll<HTMLButtonElement>('.chips button')];
    expect(chips().map((c) => [text(c), c.getAttribute('aria-pressed')])).toEqual([
      ['All', 'false'],
      ['This Mac', 'false'],
      ['Other computers', 'true'],
    ]);
    expect([...el.querySelectorAll('section.where h2')].map(text)).toEqual(['On DESKTOP-PC']);
    chips()[1].click();
    await settle();
    el = harness.routeNativeElement as HTMLElement;
    expect(TestBed.inject(Router).url).toBe('/permissions?on=here');
    expect([...el.querySelectorAll('section.where h2')].map(text)).toEqual(['On this Mac']);
  });

  it('asks for everything while paused, and keeps them', async () => {
    const el = await open();
    const toggle = el.querySelector<HTMLInputElement>('.pause input')!;
    expect(toggle.checked).toBe(false);
    toggle.click();
    await settle();
    expect(backend.bodyOf('PATCH /permissions')).toEqual({ paused: true });
    expect(text(el.querySelector('.pause'))).toContain('On: Claude asks before everything.');
    expect(text(el.querySelector('.notice.warn'))).toBe(
      'Paused: none of these is used until you switch Ask for everything off.',
    );
    expect(el.querySelectorAll('.rule').length).toBe(5);

    // A refusal leaves the switch as the server keeps it.
    patchReply = () => ({
      status: 500,
      body: { error: { code: 'internal', message: 'disk full' } },
    });
    toggle.click();
    await settle();
    expect(toggle.checked).toBe(true);
    expect(text(el.querySelector('[role="alert"]'))).toBe('The server failed: disk full');
  });

  it('removes one after a second tap', async () => {
    const el = await open();
    button(row(el, 'pr_0000000000a3'), 'Remove…')!.click();
    harness.detectChanges();
    expect(text(row(el, 'pr_0000000000a3').querySelector('.confirm'))).toBe(
      'Remove it? What it allowed asks again.',
    );
    button(row(el, 'pr_0000000000a3'), 'Keep it')!.click();
    harness.detectChanges();
    expect(row(el, 'pr_0000000000a3').querySelector('.confirm')).toBeNull();
    button(row(el, 'pr_0000000000a3'), 'Remove…')!.click();
    harness.detectChanges();
    button(row(el, 'pr_0000000000a3'), 'Remove')!.click();
    await settle();
    expect(backend.calls).toContain('DELETE /permissions/pr_0000000000a3');
    expect(el.querySelector('[data-id="pr_0000000000a3"]')).toBeNull();
    expect([...el.querySelectorAll('section.where')][0].querySelectorAll('h3').length).toBe(2);
  });

  it('edits the patterns and where it applies in a sheet', async () => {
    const el = await open();
    button(row(el, 'pr_0000000000a1'), 'Edit')!.click();
    await settle();
    const dlg = sheet();
    expect(dlg.open).toBe(true);
    expect(text(dlg.querySelector('.title'))).toBe('Edit permission');
    const radios = (name: string) =>
      [...dlg.querySelectorAll<HTMLInputElement>(`input[type="radio"][name$="-${name}"]`)].map(
        (r) => ({ label: text(r.closest('label')), checked: r.checked, disabled: r.disabled, r }),
      );
    expect(radios('what').map((x) => [x.label, x.checked])).toEqual([
      ['Any Bash command', false],
      ['Only commands that start with:', true],
    ]);
    // A project's: no session to keep it for; everywhere it may be.
    expect(radios('where').map((x) => [x.label, x.checked, x.disabled])).toEqual([
      ['In api-server (every session)', true, false],
      ['Everywhere on this Mac', false, false],
    ]);
    const inputs = () => [...dlg.querySelectorAll<HTMLInputElement>('.prow input')];
    expect(inputs().map((i) => i.value)).toEqual(['npm test', 'npm run lint']);
    dlg.querySelectorAll<HTMLButtonElement>('.prow button')[1].click();
    await settle();
    radios('where')[1].r.click();
    await settle();
    button(dlg, 'Save')!.click();
    await settle();
    expect(backend.bodyOf('PATCH /permissions/pr_0000000000a1')).toEqual({
      patterns: ['npm test'],
      scope: 'computer',
    });
    expect(sheet().open).toBe(false);
    // Moved: everywhere on this Mac now.
    const here = [...el.querySelectorAll('section.where')][0];
    expect(text(here.querySelector('h3'))).toBe('Everywhere on this Mac');
    expect(here.querySelectorAll('[data-id="pr_0000000000a1"]').length).toBe(1);
  });

  it('keeps allowing more for a project or everywhere to the dangerous right', async () => {
    TestBed.inject(AuthService).me.set(me({ hostOs: 'macos' }));
    const el = await open();
    button(row(el, 'pr_0000000000a1'), 'Edit')!.click();
    await settle();
    const dlg = sheet();
    const radio = (label: string) =>
      [...dlg.querySelectorAll<HTMLInputElement>('input[type="radio"]')].find(
        (r) => text(r.closest('label')) === label,
      )!;
    expect(radio('Any Bash command').disabled).toBe(true);
    expect(radio('Everywhere on this Mac').disabled).toBe(true);
    expect(radio('In api-server (every session)').disabled).toBe(false);
    expect(text(dlg.querySelector('.why'))).toBe(
      'To allow more for a project or everywhere, use the Mac, or a device allowed to take dangerous actions (Phone & devices).',
    );
    // What the server refuses is said as it came.
    patchReply = () => ({
      status: 422,
      body: { error: { code: 'invalid_pattern', message: '“a | b” can’t hold a pipe.' } },
    });
    const first = dlg.querySelector<HTMLInputElement>('.prow input')!;
    first.value = 'a | b';
    first.dispatchEvent(new Event('input'));
    await settle();
    button(dlg, 'Save')!.click();
    await settle();
    expect(text(dlg.querySelector('[role="alert"]'))).toBe('“a | b” can’t hold a pipe.');
    expect(sheet().open).toBe(true);
    button(dlg, 'Cancel')!.click();
    await settle();
    expect(sheet().open).toBe(false);
  });

  it('teaches how to make one when there is none', async () => {
    view = { rules: [], paused: false };
    const el = await open();
    expect(text(el.querySelector('.empty-state strong'))).toBe('No permissions yet');
    expect(text(el.querySelector('.empty-state .text'))).toBe(
      'When Claude asks, choose Allow for… to remember your answer.',
    );
    expect(el.querySelector('.empty-state a')?.getAttribute('href')).toBe('/help/permissions');
    expect(el.querySelector('.chips')).toBeNull();
  });

  it('reads them again when they change anywhere', async () => {
    const el = await open();
    view = { ...view, rules: view.rules.slice(0, 1) };
    (TestBed.inject(RealtimeService) as unknown as { dispatch(raw: string): void }).dispatch(
      JSON.stringify({ t: 'permissions' }),
    );
    await settle();
    expect(el.querySelectorAll('.rule').length).toBe(1);
    expect(el.querySelector('.chips')).toBeNull();
  });
});

describe('groupRules', () => {
  beforeEach(() => hostOsSignal.set('windows'));
  afterEach(() => hostOsSignal.set(undefined));

  it('puts this computer first, then the others by name; everywhere, projects, sessions', () => {
    const groups = groupRules([
      ...RULES,
      rule('pr_0000000000b1', {
        kind: 'run',
        tool: undefined,
        machine: 'a'.repeat(32),
        machineName: 'BUILD-BOX',
        scope: 'session',
        session: '0f1e2d3c-0000-4000-8000-000000000000',
      }),
      rule('pr_0000000000b2', { projectName: 'aardvark', project: '/x/aardvark' }),
    ]);
    expect(groups.map((g) => g.title)).toEqual(['On this PC', 'On BUILD-BOX', 'On DESKTOP-PC']);
    expect(groups[0].scopes.map((s) => s.title)).toEqual([
      'Everywhere on this PC',
      'In aardvark (every session)',
      'In api-server (every session)',
      'In the session auth-refactor',
    ]);
    // A session without a name: its id's start.
    expect(groups[1].scopes.map((s) => s.title)).toEqual(['From the session 0f1e2d3c']);
  });
});
