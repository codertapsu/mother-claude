import { ComponentFixture, TestBed } from '@angular/core/testing';

import { FakeBackend, asDesktop, flush, useFakeServer } from '../core/mock/fake-server.testing';
import { setActiveMock } from '../core/mock/mock-api';
import { JobView } from '../core/models';
import { DataJobComponent } from './data-job.component';

const job = (over: Partial<JobView> = {}): JobView => ({
  id: 'job1',
  kind: 'move',
  state: 'done',
  startedAt: Date.now() - 60_000,
  finishedAt: Date.now() - 30_000,
  filesDone: 3,
  filesTotal: 3,
  bytesDone: 3072,
  bytesTotal: 3072,
  sessions: ['s1'],
  undoUntil: Date.now() + 86_400_000,
  ...over,
});

describe('DataJobComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<DataJobComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend);
  });

  afterEach(() => setActiveMock(null));

  const render = (j: JobView) => {
    fixture = TestBed.createComponent(DataJobComponent);
    fixture.componentRef.setInput('job', j);
    fixture.componentRef.setInput('label', 'Move to Work');
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();

  it('shows how far a running job is', () => {
    const el = render(
      job({ state: 'running', bytesDone: 1024, filesDone: 1, undoUntil: undefined }),
    );
    const bar = el.querySelector('[role="progressbar"]');
    expect(bar?.getAttribute('aria-valuenow')).toBe('33');
    expect(text(el)).toContain('1.0 KB of 3.0 KB');
    expect(el.querySelector('button')).toBeNull();
  });

  it('tells a phone that the undo is on the Mac', () => {
    const el = render(job());
    expect(text(el)).toContain('Moved 1 session.');
    expect(text(el)).toContain('It can be undone on the Mac until');
    expect(el.querySelector('button')).toBeNull();
  });

  it('titles a move into a storage with its label, and says what it did', async () => {
    backend.routes.set('/backup', () => ({
      status: 200,
      body: {
        settings: { folder: '/x' },
        storage: [
          { id: 'backup', label: 'iCloud Drive', path: '/x', primary: true, sets: [] },
          { id: 'stor_1', label: 'Google Drive', path: '/y', sets: [] },
        ],
      },
    }));
    fixture = TestBed.createComponent(DataJobComponent);
    fixture.componentRef.setInput(
      'job',
      job({
        from: 'default',
        toStorage: 'stor_1',
        set: 'Shared',
        result: { archived: ['s1', 's2'], skipped: [{ id: 's3', code: 'session_live' }] },
      }),
    );
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(backend.calls).toContain('GET /backup');
    expect(text(el.querySelector('.jtitle'))).toBe('Move to Google Drive · Shared');
    expect(text(el.querySelector('.sum'))).toBe('Archived 2 conversations · 1 skipped');
  });

  it("says why an undo was refused, in the server's words", async () => {
    asDesktop();
    backend.routes.set('/data/jobs/job1/undo', () => ({
      status: 409,
      body: {
        error: {
          code: 'undo_conflict',
          message: 'the copied conversation was continued since; nothing was undone',
        },
      },
    }));
    const changed = jasmine.createSpy('changed');
    const el = render(job());
    fixture.componentInstance.changed.subscribe(changed);
    const undo = () => el.querySelector('.undo .btn') as HTMLButtonElement;
    undo().click();
    fixture.detectChanges();
    expect(text(undo())).toBe('Tap again to undo');
    undo().click();
    await flush();
    fixture.detectChanges();
    expect(text(el.querySelector('[role="alert"]'))).toBe(
      'Not undone: The copied conversation was continued since; nothing was undone.',
    );
    expect(changed).not.toHaveBeenCalled();
  });
});
