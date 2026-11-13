import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';

import { CapacityView } from '../../core/capacity';
import { MachinesSettingsView, defaultCode } from '../../core/machines';
import {
  FakeBackend,
  asDesktop,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_WORK, buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { MockCapacity } from '../../core/mock/mock-capacity';
import { ProjectSettingsService } from '../../core/project-settings';
import { PEER_ID, SELF_ID, machinesView, peer } from '../machines/machines.testing';
import { ProjectSettingsSheetComponent } from './project-settings-sheet.component';

@Component({ template: '' })
class BlankComponent {}

const CWD = '/Users/marcus/dev/api';

function capacity(): CapacityView {
  const v = new MockCapacity({ desktop: true }).view();
  return {
    ...v,
    canManage: true,
    budgets: [],
    settings: {
      ...v.settings,
      projectCap: 4,
      projectCaps: { '/Users/marcus/dev/other': 2 },
      budgets: {
        ...v.settings.budgets,
        projectDailyUsd: null,
        projects: { '/Users/marcus/dev/other': 10 },
      },
    },
  };
}

const settingsView = (over: Partial<MachinesSettingsView> = {}): MachinesSettingsView => ({
  relay: { mode: 'public', url: null },
  keepAwake: true,
  code: defaultCode(),
  projects: [],
  ...over,
});

describe("ProjectSettingsSheetComponent: a project's settings in one sheet", () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<ProjectSettingsSheetComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [
      provideRouter([
        { path: 'accounts', component: BlankComponent },
        { path: 'computers/options', component: BlankComponent },
      ]),
    ]);
    serveAccounts(backend, buildAccounts(Date.now(), true));
    backend.routes.set('/accounts/defaults', (_m, body) => ({ status: 200, body }));
    backend.routes.set('/machines', () => ({
      status: 200,
      body: machinesView({ enabled: true, peers: [peer()] }),
    }));
    backend.routes.set('/machines/settings', (m) => ({
      status: 200,
      body:
        m === 'PUT'
          ? settingsView({
              projects: [
                {
                  key: CWD,
                  project: CWD,
                  root: null,
                  code: {},
                  recipe: null,
                  copies: [],
                  updatedAt: 1,
                },
              ],
            })
          : settingsView(),
    }));
    backend.routes.set('/machines/sync', () => ({
      status: 200,
      body: { pairs: [], watching: true },
    }));
    backend.routes.set('/roots', () => ({
      status: 200,
      body: {
        machine: SELF_ID,
        machines: [
          { id: SELF_ID, name: 'MacBook', os: 'macos', here: true, paired: false },
          { id: PEER_ID, name: 'DESKTOP-PC', os: 'windows', here: false, paired: true },
        ],
        roots: [
          {
            id: 'rt_dev',
            name: 'dev',
            paths: { [SELF_ID]: '/Users/marcus/dev', [PEER_ID]: 'D:\\dev' },
            updatedAt: 1,
            by: SELF_ID,
          },
        ],
        max: 16,
      },
    }));
    backend.routes.set('/capacity', () => ({ status: 200, body: capacity() }));
    backend.routes.set('/capacity/settings', () => ({ status: 200, body: capacity() }));
    backend.routes.set('/teams', () => ({ status: 200, body: { teams: [] } }));
  });

  afterEach(() => setActiveMock(null));

  /** Every request answered, and shown. */
  const settle = async () => {
    for (let i = 0; i < 4; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const open = async (cwd = CWD, name = 'api') => {
    fixture = TestBed.createComponent(ProjectSettingsSheetComponent);
    fixture.detectChanges();
    TestBed.inject(ProjectSettingsService).open({ cwd, name });
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  /** The limits' form, its fields bound (a form binds them a moment
   * later). */
  const editLimits = async (el: HTMLElement) => {
    button(el, 'Change the limits')!.click();
    await settle();
    return [...el.querySelectorAll<HTMLInputElement>('mc-project-limits input')];
  };
  const type = (input: HTMLInputElement, value: string) => {
    input.value = value;
    input.dispatchEvent(new Event('input'));
  };
  const text = (e: Element | null | undefined) =>
    (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const headings = (el: HTMLElement) =>
    [...el.querySelectorAll('section > h3')].map((h) => text(h));
  const button = (el: HTMLElement, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b) === label);

  it("gathers the project's settings, each in its own section", async () => {
    asDesktop();
    const el = await open();
    expect(el.querySelector('dialog')?.getAttribute('aria-label')).toBe('api — project settings');
    expect(headings(el)).toEqual([
      'Account',
      'On each computer',
      'Code sync',
      'When it moves',
      'Team limits',
    ]);
    // Its folder on each computer, through the shared folder.
    expect(text(el.querySelector('mc-project-folders'))).toContain('D:\\dev\\api');
    expect(text(el.querySelector('mc-project-folders'))).toContain(
      'Through the shared folder dev.',
    );
    // Code sync and how it moves: this project's alone.
    expect(text(el.querySelector('mc-machines-sync'))).toContain(
      'It is not kept in sync with another computer.',
    );
    expect(text(el.querySelector('mc-machines-project-options'))).toContain(
      'It follows the defaults:',
    );
    // Its limits: every project's until it has its own.
    const limits = text(el.querySelector('mc-project-limits'));
    expect(limits).toContain('Role sessions at once 4 (every project’s)');
    expect(limits).toContain('Daily budget no budget (every project’s)');
  });

  it('leaves out the computers while Computers is not in use', async () => {
    asDesktop();
    backend.routes.set('/machines', () => ({ status: 200, body: machinesView() }));
    const el = await open();
    expect(headings(el)).toEqual(['Account', 'Your other computers', 'Team limits']);
    expect(el.querySelector('mc-machines-project-options')).toBeNull();
  });

  it('gives the project an account of its own, and takes it back', async () => {
    asDesktop();
    const el = await open();
    const select = el.querySelector<HTMLSelectElement>('mc-project-account select')!;
    expect(select.options[0].textContent?.trim()).toBe('The default: Default');
    expect(text(el.querySelector('mc-project-account'))).toContain('The default for new sessions.');
    select.value = MOCK_ACCOUNT_WORK;
    select.dispatchEvent(new Event('change'));
    await flush();
    expect(backend.bodyOf('PUT /accounts/defaults')).toEqual({
      newSession: 'default',
      bridge: 'default',
      byProject: { '/Users/marcus/dev/analytics': MOCK_ACCOUNT_WORK, [CWD]: MOCK_ACCOUNT_WORK },
    });
  });

  it('names the rule of a folder above it', async () => {
    asDesktop();
    const el = await open('/Users/marcus/dev/analytics/web', 'web');
    const account = el.querySelector('mc-project-account')!;
    expect(account.querySelector('select')?.options[0].textContent?.trim()).toBe(
      'The default: Work',
    );
    expect(text(account)).toContain('From the rule for ~/dev/analytics.');
  });

  it("changes this project's limit and budget alone, keeping the others'", async () => {
    asDesktop();
    const el = await open();
    const [cap, usd] = await editLimits(el);
    type(cap, '3');
    type(usd, '15');
    fixture.detectChanges();
    button(el, 'Save')!.click();
    await flush();
    expect(backend.bodyOf('PUT /capacity/settings')).toEqual({
      projectCaps: { '/Users/marcus/dev/other': 2, [CWD]: 3 },
      budgets: { projects: { '/Users/marcus/dev/other': 10, [CWD]: 15 } },
    });
  });

  it('refuses a limit out of range, saying why', async () => {
    asDesktop();
    const el = await open();
    const [cap] = await editLimits(el);
    type(cap, '40');
    fixture.detectChanges();
    button(el, 'Save')!.click();
    await settle();
    expect(backend.calls).not.toContain('PUT /capacity/settings');
    expect(text(el.querySelector('mc-project-limits [role="alert"]'))).toBe(
      "A project's limit must be a whole number from 1 to 16.",
    );
  });

  it('sets its own move options from the sheet, for this project', async () => {
    asDesktop();
    const el = await open();
    const options = el.querySelector('mc-machines-project-options') as HTMLElement;
    button(options, 'Set its own options…')!.click();
    fixture.detectChanges();
    // No project to choose: it is this one.
    expect(options.querySelector('select[name="project"]')).toBeNull();
    button(options, 'Save')!.click();
    await flush();
    const body = backend.bodyOf('PUT /machines/settings') as {
      projects: { upsert: { project: string }[] };
    };
    expect(body.projects.upsert.map((p) => p.project)).toEqual([CWD]);
  });

  it('shows a phone what only the desktop changes, and lets it pick the account', async () => {
    const el = await open();
    expect(button(el, 'Set its own options…')).toBeUndefined();
    expect(text(el)).toContain('Changed on the desktop app.');
    expect(el.querySelector('mc-project-folders a')).toBeNull();
    expect(el.querySelector<HTMLSelectElement>('mc-project-account select')?.disabled).toBe(false);
  });

  it('closes when a link inside leaves for another page', async () => {
    asDesktop();
    await open();
    const svc = TestBed.inject(ProjectSettingsService);
    expect(svc.project()).not.toBeNull();
    await TestBed.inject(Router).navigateByUrl('/accounts');
    expect(svc.project()).toBeNull();
  });
});
