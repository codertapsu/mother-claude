import { ComponentFixture, TestBed } from '@angular/core/testing';
import { By } from '@angular/platform-browser';
import { provideRouter } from '@angular/router';

import { AuthService } from '../../core/auth.service';
import { desktopOs, revealLabel } from '../../core/delete-data';
import {
  FakeBackend,
  asDesktop,
  flush,
  me,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_WORK, buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { MoveDataSheetComponent } from '../../shared/move-data-sheet.component';
import { MIXED_ACCOUNTS, ProjectsViewComponent } from './projects-view.component';

const H = 3_600_000;

const projects = () => [
  {
    account: 'default',
    cwd: '/Users/me/dev/api',
    name: 'api',
    folder: '/Users/me/.claude/projects/-Users-me-dev-api',
    sessions: 12,
    live: 1,
    bytes: 4_404_019,
    lastActivityAt: Date.now() - 3 * H,
  },
  {
    account: MOCK_ACCOUNT_WORK,
    cwd: '/Users/me/dev/web',
    name: 'web',
    folder: '/Users/me/.claude-work/projects/-Users-me-dev-web',
    sessions: 1,
    live: 0,
    bytes: 2048,
    lastActivityAt: Date.now() - 60_000,
  },
  {
    account: 'default',
    cwd: '/Users/me/dev/web',
    name: 'web',
    folder: '/Users/me/.claude/projects/-Users-me-dev-web',
    sessions: 2,
    live: 0,
    bytes: 1024,
    lastActivityAt: Date.now() - 2 * H,
  },
];

describe('ProjectsViewComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<ProjectsViewComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    serveAccounts(backend, buildAccounts(Date.now(), true));
    backend.routes.set('/data/projects', () => ({ status: 200, body: { projects: projects() } }));
    backend.routes.set('/data/jobs', () => ({ status: 200, body: { jobs: [] } }));
  });

  afterEach(() => setActiveMock(null));

  const settle = async (times = 3) => {
    for (let i = 0; i < times; i++) {
      fixture.detectChanges();
      await flush();
    }
    fixture.detectChanges();
  };
  const render = async () => {
    fixture = TestBed.createComponent(ProjectsViewComponent);
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  /** Each row's name, account, folder, what it holds and where on disk. */
  const rows = (el: HTMLElement) =>
    [...el.querySelectorAll('.prow')].map((r) =>
      ['.pname', 'mc-account-chip', '.pcwd', '.pmeta', '.pfolder']
        .map((sel) => text(r.querySelector(sel)))
        .join(' | '),
    );
  const button = (el: Element, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));
  const type = (input: HTMLInputElement | HTMLSelectElement, value: string) => {
    input.value = value;
    input.dispatchEvent(new Event(input instanceof HTMLSelectElement ? 'change' : 'input'));
    fixture.detectChanges();
  };

  it('lists every project on disk, newest first, with what it holds', async () => {
    const el = await render();
    expect(rows(el)).toEqual([
      'web | Account Work | ~/dev/web | 1 conversation · 2.0 KB · 1m ago | ~/.claude-work/projects/-Users-me-dev-web',
      'web | Account Default | ~/dev/web | 2 conversations · 1.0 KB · 2h ago | ~/.claude/projects/-Users-me-dev-web',
      'api | Account Default | ~/dev/api | 12 conversations · 1 running · 4.2 MB · 3h ago | ~/.claude/projects/-Users-me-dev-api',
    ]);
    type(el.querySelector<HTMLInputElement>('input[type="search"]')!, 'API');
    expect(rows(el).length).toBe(1);
    type(el.querySelector<HTMLInputElement>('input[type="search"]')!, '');
    type(el.querySelector<HTMLSelectElement>('select[aria-label="Account"]')!, MOCK_ACCOUNT_WORK);
    expect(rows(el).map((r) => r.split(' | ').slice(0, 2).join(' | '))).toEqual([
      'web | Account Work',
    ]);
  });

  it("deletes the picked projects from each one's account, and moves one account's together", async () => {
    asDesktop();
    backend.routes.set('/data/delete', () => ({
      status: 200,
      body: { plan: { sessions: [{ id: 's1', files: 3, bytes: 300 }], skipped: [] } },
    }));
    const el = await render();
    button(el, 'Select')!.click();
    fixture.detectChanges();
    const boxes = () => [...el.querySelectorAll<HTMLInputElement>('.prow .pick input')];
    boxes()[0].click(); // web, Work
    boxes()[2].click(); // api, Default
    fixture.detectChanges();
    expect(text(el.querySelector('.selcount'))).toBe('2 selected');
    // Two accounts: one move cannot take both.
    expect(button(el, 'Move or copy 2')!.disabled).toBeTrue();
    expect(text(el.querySelector('.selbar .why'))).toBe(MIXED_ACCOUNTS);

    button(el, 'Delete 2')!.click();
    await settle();
    const asked = backend.calls
      .map((c, i) => (c === 'POST /data/delete' ? backend.bodies[i] : null))
      .filter(Boolean);
    expect(asked).toEqual([
      {
        from: MOCK_ACCOUNT_WORK,
        items: { projects: ['/Users/me/dev/web'] },
        undo: true,
        dryRun: true,
      },
      { from: 'default', items: { projects: ['/Users/me/dev/api'] }, undo: true, dryRun: true },
    ]);
    expect(text(el.querySelector('mc-delete-data-sheet .title'))).toBe('Delete 2 projects');
    el.querySelector<HTMLButtonElement>('mc-delete-data-sheet button[aria-label="Close"]')!.click();
    fixture.detectChanges();

    // One account's projects move together.
    boxes()[0].click();
    boxes()[1].click();
    fixture.detectChanges();
    expect(el.querySelector('.selbar .why')).toBeNull();
    button(el, 'Move or copy 2')!.click();
    fixture.detectChanges();
    const sheet = fixture.debugElement.query(By.directive(MoveDataSheetComponent))
      .componentInstance as MoveDataSheetComponent;
    expect(sheet.start()).toEqual({
      from: { kind: 'account', account: 'default' },
      what: 'projects',
      projects: ['/Users/me/dev/web', '/Users/me/dev/api'],
    });
  });

  it("reveals a project's folder on the desktop, and copies its path", async () => {
    asDesktop();
    backend.routes.set('/data/projects/reveal', () => ({ status: 200, body: {} }));
    const el = await render();
    const menu = () => el.querySelector<HTMLButtonElement>('[aria-label="Project menu: api"]')!;
    menu().click();
    fixture.detectChanges();
    const sheet = () => el.querySelector('mc-sheet.menu-sheet')!;
    expect(text(sheet().querySelector('.where'))).toBe('~/.claude/projects/-Users-me-dev-api');
    button(sheet(), revealLabel(desktopOs(navigator.userAgent)))!.click();
    await settle();
    expect(backend.bodyOf('POST /data/projects/reveal')).toEqual({
      account: 'default',
      cwd: '/Users/me/dev/api',
    });
    expect(el.querySelector('mc-sheet.menu-sheet')).toBeNull();

    const copy = spyOn(navigator.clipboard, 'writeText').and.resolveTo();
    menu().click();
    fixture.detectChanges();
    button(sheet(), 'Copy path')!.click();
    await settle();
    expect(copy).toHaveBeenCalledWith('/Users/me/.claude/projects/-Users-me-dev-api');
  });

  it('on a phone copies the path but does not reveal, and a delete says what it needs', async () => {
    TestBed.inject(AuthService).me.set(me({ allowDangerous: false }));
    const el = await render();
    el.querySelector<HTMLButtonElement>('[aria-label="Project menu: api"]')!.click();
    fixture.detectChanges();
    const items = [...el.querySelectorAll('mc-sheet.menu-sheet .item')].map((b) => text(b));
    expect(items).toEqual(['Copy path', 'Move or copy…', 'Delete…']);
    button(el.querySelector('mc-sheet.menu-sheet')!, 'Delete…')!.click();
    await settle();
    expect(text(el.querySelector('mc-delete-data-sheet .notice.warn'))).toContain(
      'needs dangerous actions for this device',
    );
    expect(backend.calls).not.toContain('POST /data/delete');
  });

  it('keeps recent deletes, with their Undo, below the list', async () => {
    const job = (id: string, kind: string, startedAt: number) => ({
      id,
      kind,
      state: 'done',
      startedAt,
      account: MOCK_ACCOUNT_WORK,
      sessions: ['w1'],
      result: { deleted: ['w1'], skipped: [] },
      undoUntil: Date.now() + 86_400_000,
    });
    backend.routes.set('/data/jobs', () => ({
      status: 200,
      body: { jobs: [job('m', 'move', 3), job('d1', 'delete', 1), job('d2', 'delete', 2)] },
    }));
    const el = await render();
    const listed = [...el.querySelectorAll('[aria-label="Recently deleted"] mc-data-job')];
    expect(listed.map((j) => text(j.querySelector('.jtitle')))).toEqual([
      'Delete from Work',
      'Delete from Work',
    ]);
    expect(text(listed[0])).toContain('Deleted 1 conversation');
    expect(listed[0].querySelector('.undo')).not.toBeNull();
  });
});
