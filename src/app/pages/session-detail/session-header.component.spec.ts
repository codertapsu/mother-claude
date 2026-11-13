import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';

import {
  FakeBackend,
  asDesktop,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { buildAccounts, ownedCaps } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { ListDetailService } from '../../core/list-detail';
import { JobView, SessionDetail } from '../../core/models';
import { DeleteDataSheetComponent } from '../../shared/delete-data-sheet.component';
import { SessionContext } from './session-context';
import { SessionHeaderComponent } from './session-header.component';

const detail = {
  id: 's1',
  name: 'auth',
  title: 'Fix the login',
  projectName: 'api',
  cwd: '/Users/me/dev/api',
  surface: 'cli',
  owned: false,
  live: false,
  status: 'ended',
  activity: { kind: 'idle', label: 'Idle' },
  usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
  messageCount: 3,
  agents: { running: 0, total: 0, workflowsRunning: 0, backgroundRunning: 0 },
  queued: 0,
  pending: [],
  control: ownedCaps(),
} as unknown as SessionDetail;

describe('SessionHeaderComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<SessionHeaderComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([]), SessionContext]);
    serveAccounts(backend, buildAccounts(Date.now(), false));
    asDesktop();
  });

  afterEach(() => setActiveMock(null));

  const render = async () => {
    const ctx = TestBed.inject(SessionContext);
    ctx.id.set('s1');
    ctx.detail.set(detail);
    fixture = TestBed.createComponent(SessionHeaderComponent);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const button = (el: HTMLElement, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => (b.textContent ?? '').replace(/\s+/g, ' ').trim() === label,
    );

  const text = (e: Element | null | undefined) =>
    (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
  /** The menu as its groups: each heading with what is under it. */
  const groups = (el: HTMLElement) => {
    const out: [string, string[]][] = [];
    for (const node of el.querySelectorAll('.menu > *')) {
      if (node.matches('h2.group')) out.push([text(node), []]);
      else if (node.matches('.item')) out.at(-1)?.[1].push(text(node.querySelector('span')));
      else if (node.matches('.lifecycle')) {
        out.at(-1)?.[1].push(...[...node.querySelectorAll('button')].map((b) => text(b)));
      }
    }
    return out;
  };
  const openMenu = (el: HTMLElement) => {
    el.querySelector<HTMLButtonElement>('[aria-label="Session menu"]')?.click();
    fixture.detectChanges();
  };

  it('groups its menu: Continue, Organize, This session, Its process', async () => {
    const ctx = TestBed.inject(SessionContext);
    const el = await render();
    ctx.detail.set({
      ...detail,
      live: true,
      status: 'idle',
      control: {
        ...ownedCaps(),
        channel: 'hooks',
        terminal: true,
        lifecycle: { stop: true, respawn: true, rm: true },
      },
    } as SessionDetail);
    fixture.detectChanges();
    openMenu(el);
    expect(groups(el)).toEqual([
      ['Continue', ['Continue here, as a new session']],
      ['Organize', ['Rename…', 'Delete…']],
      ['This session', ['Terminal view', 'Session info', 'claude --resume s1…']],
      ['Its process', ['Stop', 'Restart', 'Remove']],
    ]);
    // An ended one is continued as it is; a session Mother Claude runs has
    // nothing to continue and no process of a background job.
    ctx.detail.set(detail);
    fixture.detectChanges();
    expect(groups(el)[0]).toEqual(['Continue', ['Continue here']]);
    ctx.detail.set({
      ...detail,
      owned: true,
      control: { ...ownedCaps(), lifecycle: { stop: false, respawn: false, rm: false } },
    } as SessionDetail);
    fixture.detectChanges();
    expect(groups(el).map(([g]) => g)).toEqual(['Organize', 'This session']);
  });

  it('continues a session here as a new one, with launch options, from its menu', async () => {
    backend.routes.set('/sessions/s1/continue', () => ({ status: 200, body: { id: 's2' } }));
    const ctx = TestBed.inject(SessionContext);
    const el = await render();
    ctx.detail.set({ ...detail, live: true, status: 'working' } as SessionDetail);
    fixture.detectChanges();
    openMenu(el);
    button(el, 'Continue here, as a new session')?.click();
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    // Nothing started yet: a sheet says what happens, with the options.
    expect(backend.calls).not.toContain('POST /sessions/s1/continue');
    const sheet = el.querySelector('.continue')!;
    expect(text(sheet)).toContain('It keeps running where it is');
    expect(sheet.querySelector('mc-launch-options')).not.toBeNull();
    vi.spyOn(TestBed.inject(Router), 'navigate').mockResolvedValue(true);
    const go = sheet.querySelector<HTMLButtonElement>('.btn.primary')!;
    expect(text(go)).toBe('Continue here, as a new session');
    go.click();
    await flush();
    expect(backend.calls).toContain('POST /sessions/s1/continue');
    expect(backend.bodyOf('POST /sessions/s1/continue')).toEqual({ fork: true });
  });

  it('deletes the conversation from its menu, and leaves the page once it is gone', async () => {
    backend.routes.set('/data/delete', () => ({
      status: 200,
      body: { plan: { sessions: [{ id: 's1', files: 3, bytes: 2048 }], skipped: [] } },
    }));
    const el = await render();
    el.querySelector<HTMLButtonElement>('[aria-label="Session menu"]')?.click();
    fixture.detectChanges();
    button(el, 'Delete…')?.click();
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    // The only account: the delete names it.
    expect(backend.bodyOf('POST /data/delete')).toEqual({
      from: 'default',
      items: { sessions: ['s1'] },
      undo: true,
      dryRun: true,
    });
    expect(button(el, 'Delete 1 conversation')).toBeDefined();

    const router = TestBed.inject(Router);
    const navigate = vi.spyOn(router, 'navigate').mockResolvedValue(true);
    const sheet = fixture.debugElement.query(
      (d) => d.componentInstance instanceof DeleteDataSheetComponent,
    ).componentInstance as DeleteDataSheetComponent;
    const done = {
      id: 'j',
      kind: 'delete',
      state: 'done',
      startedAt: 1,
      filesDone: 3,
      filesTotal: 3,
      bytesDone: 2048,
      bytesTotal: 2048,
      sessions: ['s1'],
      result: { deleted: ['s1'], skipped: [] },
    } as JobView;
    // Undone in the dialog: the conversation is still there.
    sheet.closed.emit([{ ...done, state: 'undone' }]);
    expect(navigate).not.toHaveBeenCalled();
    sheet.closed.emit([done]);
    expect(navigate).toHaveBeenCalledWith(['/']);
  });

  it('closes with ✕ beside its list on a wide screen, back to that list', async () => {
    const ld = TestBed.inject(ListDetailService);
    ld.showList('inbox', '/inbox');
    ld.showDetail({ id: 's1', machine: null, agent: null }, false);
    const el = await render();
    // Narrower: back to Sessions, as before.
    expect(el.querySelector('a.back')?.getAttribute('aria-label')).toBe('Back to sessions');
    ld.wide.set(true);
    fixture.detectChanges();
    expect(el.querySelector('a.back')).toBeNull();
    const close = el.querySelector<HTMLButtonElement>('button.back')!;
    expect(close.getAttribute('aria-label')).toBe('Close');
    const go = vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true);
    close.click();
    expect(go).toHaveBeenCalledWith('/inbox');
  });
});
