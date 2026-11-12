import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';

import {
  AccountsTable,
  FoldersView,
  MachineAsk,
  PeerFolders,
  RootsView,
} from '../../core/machines';
import {
  FakeBackend,
  asDesktop,
  flush,
  me,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { AuthService } from '../../core/auth.service';
import { AsksCardComponent } from './asks-card.component';
import { PEER_ID, SELF_ID, check, machinesView, peer, setupView } from './machines.testing';
import { ProjectOptionsComponent } from './project-options.component';
import { MachinesSetupComponent } from './setup.component';

@Component({ template: '' })
class BlankComponent {}

const HERE = '/Users/you/development/projects';
const THERE = 'D:\\development\\projects';

function accounts(over: Partial<AccountsTable> = {}): AccountsTable {
  return {
    machine: SELF_ID,
    machines: [
      { id: SELF_ID, name: 'MacBook', here: true, online: true, error: null },
      { id: PEER_ID, name: 'DESKTOP-PC', here: false, online: true, error: null },
    ],
    accounts: [
      {
        name: 'Default',
        default: true,
        cells: {
          [SELF_ID]: { state: 'signedIn', kind: 'default' },
          [PEER_ID]: { state: 'signedIn', kind: 'default' },
        },
        asks: {},
      },
      {
        name: 'Work',
        default: false,
        cells: {
          [SELF_ID]: { state: 'signedIn', kind: 'directory' },
          [PEER_ID]: { state: 'signedOut', kind: 'directory', name: 'work' },
        },
        asks: {},
      },
      {
        name: 'Client X',
        default: false,
        cells: { [SELF_ID]: { state: 'signedIn', kind: 'token' }, [PEER_ID]: { state: 'missing' } },
        asks: {},
      },
    ],
    ...over,
  };
}

function peerFolders(over: Partial<PeerFolders> = {}): PeerFolders {
  return {
    machine: PEER_ID,
    name: 'DESKTOP-PC',
    online: true,
    error: null,
    proposals: [],
    matched: [],
    mismatched: [],
    onlyHere: [],
    onlyThere: [],
    ...over,
  };
}

function folders(peers: PeerFolders[] = [peerFolders()]): FoldersView {
  return { machine: SELF_ID, repos: 3, noOrigin: [], peers };
}

function roots(over: Partial<RootsView> = {}): RootsView {
  return {
    machine: SELF_ID,
    machines: [
      { id: SELF_ID, name: 'MacBook', os: 'macos', here: true, paired: false },
      { id: PEER_ID, name: 'DESKTOP-PC', os: 'windows', here: false, paired: true },
    ],
    roots: [],
    max: 64,
    ...over,
  };
}

const text = (el: Element | null | undefined) =>
  (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const button = (el: Element, label: string) =>
  [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));

describe('Machines setup, steps 5 and 6', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<MachinesSetupComponent>;

  /** The wizard, left at `step`. */
  function serveSetup(step: number): void {
    backend.routes.set('/machines', () => ({
      status: 200,
      body: machinesView({ setupStep: step, peers: [peer()] }),
    }));
    backend.routes.set('/machines/setup/progress', () => ({
      status: 200,
      body: machinesView({ setupStep: step, peers: [peer()] }),
    }));
    backend.routes.set('/machines/setup', () => ({ status: 200, body: setupView() }));
    backend.routes.set('/machines/setup/check', () => ({ status: 200, body: setupView() }));
  }

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([{ path: '**', component: BlankComponent }])]);
    backend.routes.set('/machines/accounts', () => ({ status: 200, body: accounts() }));
    backend.routes.set('/machines/folders', () => ({ status: 200, body: folders() }));
    backend.routes.set('/roots', () => ({ status: 200, body: roots() }));
    backend.routes.set('/machines/setup/ask', (_m, body) => ({
      status: 200,
      body: { asked: true, ask: { id: 'ask_0123456789abcdef', ...(body as object) } },
    }));
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const settle = async () => {
    for (let i = 0; i < 5; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const render = async (step: number) => {
    serveSetup(step);
    asDesktop();
    fixture = TestBed.createComponent(MachinesSetupComponent);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };

  it('matches accounts by name on every computer, and asks the other to sign one in', async () => {
    const el = await render(5);
    expect(text(el.querySelector('.step-body h2'))).toBe('Accounts and folders');
    const table = el.querySelector('table.accounts')!;
    const heads = [...table.querySelectorAll('thead th')].map((th) => text(th));
    expect(heads).toEqual(['Account', 'This computer', 'DESKTOP-PC']);
    const rows = [...table.querySelectorAll('tbody tr')];
    expect(rows.map((r) => text(r.querySelector('th')))).toEqual(['Default', 'Work', 'Client X']);
    const work = rows[1];
    expect(text(work)).toContain('not signed in');
    expect(text(work)).toContain('(as work)');
    // Client X is not on DESKTOP-PC: it may be added there, asked.
    expect(button(rows[2], 'Add it on DESKTOP-PC…')).toBeDefined();

    button(work, 'Sign in on DESKTOP-PC…')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/setup/ask')).toEqual({
      machine: PEER_ID,
      check: 'signIn',
      account: 'work',
    });
    expect(text(el.querySelector('.notice.ok'))).toBe(
      'Asked DESKTOP-PC: someone there signs work in, in its browser.',
    );
  });

  it('says an asked sign-in waits for someone there, instead of asking again', async () => {
    const asked: MachineAsk = {
      id: 'ask_0123456789abcdef',
      from: SELF_ID,
      fromName: 'MacBook',
      kind: 'signIn',
      account: 'work',
      at: Date.now(),
      state: 'asked',
    };
    const view = accounts();
    view.accounts[1].asks = { [PEER_ID]: asked };
    backend.routes.set('/machines/accounts', () => ({ status: 200, body: view }));
    const el = await render(5);
    const work = el.querySelectorAll('table.accounts tbody tr')[1];
    expect(button(work, 'Sign in on DESKTOP-PC…')).toBeUndefined();
    expect(text(work)).toContain('Asked — waiting for someone at DESKTOP-PC.');
  });

  it('proposes a root from the repositories’ git remotes, and saves the one you use', async () => {
    backend.routes.set('/machines/folders', () => ({
      status: 200,
      body: folders([
        peerFolders({
          proposals: [
            {
              id: 'pr_0123456789ab',
              name: 'projects',
              here: HERE,
              there: THERE,
              repos: ['mother-claude', 'tray-tool', 'site', 'lib'].map((n) => ({
                name: n,
                origin: `https://github.com/you/${n}.git`,
                here: `${HERE}/${n}`,
                there: `${THERE}\\${n}`,
              })),
            },
          ],
        }),
      ]),
    }));
    backend.routes.set('/roots', (method, body) => {
      if (method === 'PUT') {
        const up = (body as { upsert: { name: string; paths: Record<string, string> }[] })
          .upsert[0];
        return {
          status: 200,
          body: roots({
            roots: [
              {
                id: 'rt_1111111111111111',
                name: up.name,
                paths: up.paths,
                updatedAt: 1,
                by: SELF_ID,
              },
            ],
          }),
        };
      }
      return { status: 200, body: roots() };
    });
    const el = await render(5);
    const peerBlock = el.querySelector('.peer')!;
    expect(text(peerBlock)).toContain('Found on both computers, by their git remote:');
    expect(text(peerBlock)).toContain('mother-claude, tray-tool, site and 1 more');
    button(peerBlock, 'Use')!.click();
    await settle();
    expect(backend.bodyOf('PUT /roots')).toEqual({
      upsert: [{ name: 'projects', paths: { [SELF_ID]: HERE, [PEER_ID]: THERE } }],
    });
    expect(text(el.querySelector('.notice.ok'))).toBe('Saved the folder projects.');
    expect(text(el.querySelector('.roots'))).toContain(THERE);
    // The folders are looked at again, fresh.
    expect(backend.urls).toContain('/machines/folders?fresh=true');
  });

  it('offers to clone a repository on the other computer (asked there), or here', async () => {
    backend.routes.set('/machines/folders', () => ({
      status: 200,
      body: folders([
        peerFolders({
          onlyHere: [
            {
              name: 'side-project',
              path: `${HERE}/side-project`,
              origin: 'https://github.com/you/side-project',
              root: 'rt_1111111111111111',
              rel: 'side-project',
              mapped: `${THERE}\\side-project`,
            },
            { name: 'loose', path: '/Users/you/loose', origin: 'https://github.com/you/loose' },
          ],
          onlyThere: [
            {
              name: 'win-tools',
              path: `${THERE}\\win-tools`,
              origin: 'https://github.com/you/win-tools',
              root: 'rt_1111111111111111',
              rel: 'win-tools',
              mapped: `${HERE}/win-tools`,
            },
          ],
        }),
      ]),
    }));
    backend.routes.set('/machines/clone', (_m, body) => ({
      status: 200,
      body: { cloned: true, folder: `${HERE}/${(body as { rel: string }).rel}` },
    }));
    const el = await render(5);
    const items = [...el.querySelectorAll('.peer .list li')];
    const side = items.find((li) => text(li).startsWith('side-project'))!;
    const loose = items.find((li) => text(li).startsWith('loose'))!;
    expect(text(loose)).toContain('Share a folder that holds it first.');
    expect(button(loose, 'Clone it on DESKTOP-PC')).toBeUndefined();

    button(side, 'Clone it on DESKTOP-PC')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/setup/ask')).toEqual({
      machine: PEER_ID,
      check: 'clone',
      origin: 'https://github.com/you/side-project',
      root: 'rt_1111111111111111',
      rel: 'side-project',
    });
    expect(text(el.querySelector('.notice.ok'))).toBe(
      'Asked DESKTOP-PC: it clones side-project once someone there confirms.',
    );

    const win = [...el.querySelectorAll('.peer .list li')].find((li) =>
      text(li).startsWith('win-tools'),
    )!;
    button(win, 'Clone it here')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/clone')).toEqual({
      origin: 'https://github.com/you/win-tools',
      root: 'rt_1111111111111111',
      rel: 'win-tools',
    });
    expect(text(el.querySelector('.notice.ok'))).toBe(`Cloned into ${HERE}/win-tools.`);
  });

  it('adds a root with a folder on each computer, and removes one on a second tap', async () => {
    let current = roots({
      roots: [
        {
          id: 'rt_2222222222222222',
          name: 'old',
          paths: { [SELF_ID]: '/Users/you/old' },
          updatedAt: 1,
          by: SELF_ID,
        },
      ],
    });
    backend.routes.set('/roots', (method, body) => {
      if (method === 'PUT') {
        const c = body as {
          upsert?: { name: string; paths: Record<string, string> }[];
          remove?: string[];
        };
        current = roots({
          roots: [
            ...current.roots.filter((r) => !(c.remove ?? []).includes(r.id)),
            ...(c.upsert ?? []).map((u) => ({
              id: 'rt_3333333333333333',
              ...u,
              updatedAt: 2,
              by: SELF_ID,
            })),
          ],
        });
      }
      return { status: 200, body: current };
    });
    const el = await render(5);
    button(el, 'Add a folder')!.click();
    await settle();
    const form = el.querySelector('form.editor')!;
    const inputs = [...form.querySelectorAll<HTMLInputElement>('input')];
    const type = async (input: HTMLInputElement, value: string) => {
      input.value = value;
      input.dispatchEvent(new Event('input'));
      await settle();
    };
    await type(inputs[0], 'projects');
    await type(inputs[1], HERE);
    await type(inputs[2], THERE);
    button(form, 'Save')!.click();
    await settle();
    expect(backend.bodyOf('PUT /roots')).toEqual({
      upsert: [{ name: 'projects', paths: { [SELF_ID]: HERE, [PEER_ID]: THERE } }],
    });

    const old = [...el.querySelectorAll('.roots li')].find((li) => text(li).startsWith('old'))!;
    button(old, 'Remove')!.click();
    await settle();
    expect(backend.calls.filter((c) => c === 'PUT /roots').length).toBe(1);
    button(old, 'Tap again to remove')!.click();
    await settle();
    expect(backend.bodyOf('PUT /roots')).toEqual({ remove: ['rt_2222222222222222'] });
    expect(text(el.querySelector('.roots'))).not.toContain('/Users/you/old');
  });

  it('chooses how code travels and where it lands, for every computer', async () => {
    backend.routes.set('/machines/settings', (method, body) => ({
      status: 200,
      body: method === 'PUT' ? { ...(body as object), projects: [] } : { projects: [] },
    }));
    const el = await render(6);
    expect(text(el.querySelector('.step-body h2'))).toBe('Options');
    // Each project's own options, code sync among them.
    expect(text(el.querySelector('.step-body'))).toContain(
      'Each project can have its own: how its code moves; a recipe — what to ask Claude to run after a move, and the files git ignores that go with it; copies of its sessions kept on another computer; and code sync — its code kept the same on two computers, both ways, once both say yes.',
    );
    expect(
      el.querySelector<HTMLInputElement>('input[name="transfer"][value="bundle"]')!.checked,
    ).toBe(true);
    expect(
      el.querySelector<HTMLInputElement>('input[name="apply"][value="checkout"]')!.checked,
    ).toBe(true);
    el.querySelector<HTMLInputElement>('input[name="transfer"][value="remote"]')!.click();
    await settle();
    el.querySelector<HTMLInputElement>('input[name="keepRemoteRefs"]')!.click();
    el.querySelector<HTMLInputElement>('input[name="apply"][value="worktree"]')!.click();
    await settle();
    button(el.querySelector('mc-machines-options')!, 'Save')!.click();
    await settle();
    expect(backend.bodyOf('PUT /machines/settings')).toEqual({
      relay: { mode: 'public', url: null },
      keepAwake: true,
      code: { transfer: 'remote', apply: 'worktree', keepRemoteRefs: true },
    });
  });

  it('shows the readiness test’s lines 3–6 and marks a warning', async () => {
    backend.routes.set('/machines/test', () => ({
      status: 200,
      body: {
        results: [
          {
            machine: PEER_ID,
            name: 'DESKTOP-PC',
            at: Date.now(),
            tests: [
              {
                id: 'connect',
                state: 'ok',
                detail: 'Direct, on the same network.',
                ms: 6,
                mibps: null,
              },
              {
                id: 'transfer',
                state: 'ok',
                detail: '20 MiB at 28.4 MiB/s.',
                ms: 704,
                mibps: 28.4,
              },
              {
                id: 'tools',
                state: 'ok',
                detail: 'git and Claude Code 2.1.330 on both.',
                ms: null,
                mibps: null,
              },
              { id: 'code', state: 'ok', detail: 'Both ways in 1.4 s.', ms: 1400, mibps: null },
              {
                id: 'accounts',
                state: 'warning',
                detail: 'Work: not signed in on DESKTOP-PC · Client X: not on DESKTOP-PC',
                ms: null,
                mibps: null,
              },
              {
                id: 'folders',
                state: 'ok',
                detail: '1 folder resolves on DESKTOP-PC · 14 of 14 projects map.',
                ms: null,
                mibps: null,
              },
            ],
          },
        ],
      },
    }));
    const el = await render(7);
    button(el, 'Run the test')!.click();
    await settle();
    const heads = [...el.querySelectorAll('table.tests thead th')].map((th) => text(th));
    expect(heads).toEqual([
      'Computer',
      'Connect',
      'Transfer',
      'git, Claude Code',
      'Code',
      'Accounts',
      'Folders',
    ]);
    const cells = [...el.querySelectorAll('table.tests tbody td')];
    expect(cells[4].getAttribute('data-state')).toBe('warning');
    expect(text(cells[4])).toContain('!');
    expect(text(cells[4])).toContain('Passed, with a warning');
    expect(text(cells[3])).toContain('Both ways in 1.4 s.');
  });

  it('keeps Turn on Computers on when only an account waits for a sign-in', async () => {
    const scoped = check({
      id: 'account:acc_0123456789abcdef',
      area: 'accounts',
      title: 'Work',
      state: 'blocker',
      detail: "Not signed in: its sessions can't run or move here until it is.",
      why: 'A session moves to the account of the same name, and runs only where it is signed in.',
      severity: 'required',
      fix: { kind: 'auto', label: 'Sign in…' },
      scope: 'account',
    });
    serveSetup(7);
    backend.routes.set('/machines/setup/check', () => ({
      status: 200,
      body: setupView({
        blockers: 1,
        gating: 0,
        self: { ...setupView().self, checks: [check(), scoped] },
      }),
    }));
    asDesktop();
    fixture = TestBed.createComponent(MachinesSetupComponent);
    fixture.detectChanges();
    await settle();
    const el = fixture.nativeElement as HTMLElement;
    expect(button(el, 'Turn on Computers')!.disabled).toBe(false);
    expect(text(el.querySelector('.turn-on'))).toContain(
      '1 account is not signed in somewhere: its sessions wait for it, the rest work.',
    );
  });
});

describe('ProjectOptionsComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<ProjectOptionsComponent>;
  const settle = async () => {
    for (let i = 0; i < 5; i++) {
      await flush();
      fixture.detectChanges();
    }
  };

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    backend.routes.set('/projects', () => ({
      status: 200,
      body: { projects: [{ path: `${HERE}/app`, name: 'app' }] },
    }));
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  it('sets a project’s own code options and recipe, and goes back to the defaults', async () => {
    asDesktop();
    let projects: unknown[] = [];
    backend.routes.set('/machines/settings', (method, body) => {
      const change = (body as { projects?: { upsert?: { project: string }[]; remove?: string[] } })
        ?.projects;
      if (method === 'PUT' && change?.upsert) {
        projects = change.upsert.map((p) => ({
          key: 'rt_1111111111111111/app',
          root: null,
          updatedAt: 1,
          ...p,
        }));
      }
      if (method === 'PUT' && change?.remove) projects = [];
      return {
        status: 200,
        body: { relay: { mode: 'public', url: null }, keepAwake: true, projects },
      };
    });
    fixture = TestBed.createComponent(ProjectOptionsComponent);
    fixture.detectChanges();
    await settle();
    const el = fixture.nativeElement as HTMLElement;
    expect(text(el)).toContain('No project has its own options: every one follows the defaults.');
    button(el, 'Set a project’s own options…')!.click();
    await settle();
    const select = (name: string, value: string) => {
      const s = el.querySelector<HTMLSelectElement>(`select[name="${name}"]`)!;
      s.value = value;
      s.dispatchEvent(new Event('change'));
    };
    const area = (name: string, value: string) => {
      const a = el.querySelector<HTMLTextAreaElement>(`textarea[name="${name}"]`)!;
      a.value = value;
      a.dispatchEvent(new Event('input'));
    };
    select('project', `${HERE}/app`);
    select('transfer', 'remote');
    await settle();
    area('windows', 'npm ci\n\n  npm run build  ');
    area('carry', '.env.local');
    await settle();
    expect(text(el.querySelector('form'))).toContain('risky');
    button(el, 'Save')!.click();
    await settle();
    expect(backend.bodyOf('PUT /machines/settings')).toEqual({
      projects: {
        upsert: [
          {
            project: `${HERE}/app`,
            code: { transfer: 'remote' },
            recipe: {
              afterMove: { macos: [], windows: ['npm ci', 'npm run build'], linux: [] },
              carryIgnored: ['.env.local'],
            },
          },
        ],
      },
    });
    expect(text(el.querySelector('.list'))).toContain(
      'Through your git remote · 2 commands after a move · carries 1 ignored file',
    );
    button(el, 'Use the defaults')!.click();
    await settle();
    expect(backend.bodyOf('PUT /machines/settings')).toEqual({
      projects: { remove: ['rt_1111111111111111/app'] },
    });
  });

  it('lets no one without the dangerous right start carrying ignored files', async () => {
    TestBed.inject(AuthService).me.set(me({ allowDangerous: false }));
    backend.routes.set('/machines/settings', () => ({
      status: 200,
      body: { relay: { mode: 'public', url: null }, keepAwake: true, projects: [] },
    }));
    fixture = TestBed.createComponent(ProjectOptionsComponent);
    fixture.detectChanges();
    await settle();
    const el = fixture.nativeElement as HTMLElement;
    button(el, 'Set a project’s own options…')!.click();
    await settle();
    expect(el.querySelector<HTMLTextAreaElement>('textarea[name="carry"]')!.disabled).toBe(true);
    expect(el.querySelector<HTMLTextAreaElement>('textarea[name="windows"]')!.disabled).toBe(false);
  });
});

describe('AsksCardComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<AsksCardComponent>;
  const settle = async () => {
    for (let i = 0; i < 5; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const ask = (over: Partial<MachineAsk> = {}): MachineAsk => ({
    id: 'ask_0123456789abcdef',
    from: PEER_ID,
    fromName: 'DESKTOP-PC',
    kind: 'clone',
    title: 'side-project',
    origin: 'https://github.com/you/side-project',
    folder: `${HERE}/side-project`,
    at: Date.now(),
    state: 'asked',
    ...over,
  });

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([{ path: '**', component: BlankComponent }])]);
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  it('shows what another computer asked, and runs it only when confirmed here', async () => {
    asDesktop();
    backend.routes.set('/machines/asks/ask_0123456789abcdef', (_m, body) => ({
      status: 200,
      body: {
        ask: ask({
          state: (body as { answer: string }).answer === 'confirm' ? 'done' : 'declined',
        }),
        next: null,
      },
    }));
    fixture = TestBed.createComponent(AsksCardComponent);
    fixture.componentRef.setInput('asks', [ask()]);
    fixture.detectChanges();
    await settle();
    const el = fixture.nativeElement as HTMLElement;
    expect(text(el)).toContain('Clone side-project');
    expect(text(el)).toContain('From DESKTOP-PC');
    expect(text(el)).toContain(
      `Clone https://github.com/you/side-project into ${HERE}/side-project?`,
    );
    expect(backend.calls).toEqual([]);
    button(el, 'Clone it')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/asks/ask_0123456789abcdef')).toEqual({
      answer: 'confirm',
    });
  });

  it('says not now, and a sign-in goes on in Accounts', async () => {
    asDesktop();
    backend.routes.set('/machines/asks/ask_0123456789abcdef', (_m, body) => ({
      status: 200,
      body: {
        ask: null,
        next: (body as { answer: string }).answer === 'confirm' ? '/accounts' : null,
      },
    }));
    fixture = TestBed.createComponent(AsksCardComponent);
    fixture.componentRef.setInput('asks', [
      ask({ kind: 'signIn', account: 'Work', title: undefined }),
    ]);
    fixture.detectChanges();
    await settle();
    const el = fixture.nativeElement as HTMLElement;
    expect(text(el)).toContain('Sign in to Work');
    button(el, 'Not now')!.click();
    await settle();
    expect(backend.bodyOf('POST /machines/asks/ask_0123456789abcdef')).toEqual({
      answer: 'decline',
    });
    button(el, 'Sign in…')!.click();
    await settle();
    expect(TestBed.inject(Router).url).toBe('/accounts');
  });

  it('is shown to a phone, never answered from it', async () => {
    fixture = TestBed.createComponent(AsksCardComponent);
    fixture.componentRef.setInput('asks', [ask()]);
    fixture.componentRef.setInput('readonly', true);
    fixture.detectChanges();
    await settle();
    const el = fixture.nativeElement as HTMLElement;
    expect(button(el, 'Clone it')).toBeUndefined();
    expect(button(el, 'Not now')).toBeUndefined();
  });
});
