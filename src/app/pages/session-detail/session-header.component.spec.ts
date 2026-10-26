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
    const navigate = spyOn(router, 'navigate').and.resolveTo(true);
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
});
