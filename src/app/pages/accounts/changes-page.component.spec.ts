import { Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';

import { DataJobsService } from '../../core/data-jobs.service';
import {
  FakeBackend,
  asDesktop,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_WORK, buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { JobView } from '../../core/models';
import { RealtimeService } from '../../core/realtime.service';
import { ChangesPageComponent } from './changes-page.component';

@Component({ template: '' })
class BlankComponent {}

const NOW = Date.now();

const job = (id: string, kind: JobView['kind'], over: Partial<JobView> = {}): JobView => ({
  id,
  kind,
  state: 'done',
  startedAt: NOW - 60_000,
  finishedAt: NOW - 50_000,
  filesDone: 1,
  filesTotal: 1,
  bytesDone: 10,
  bytesTotal: 10,
  sessions: [],
  ...over,
});

/** Accounts › Recent changes (docs/UI-REDESIGN.md §6.6): every data job,
 * newest first, a chip at a time, live. */
describe('ChangesPageComponent: Accounts › Recent changes', () => {
  let backend: FakeBackend;
  let harness: RouterTestingHarness;
  let jobs: JobView[];

  beforeEach(async () => {
    backend = new FakeBackend();
    useFakeServer(backend, [
      provideRouter([
        { path: 'accounts/changes', component: ChangesPageComponent },
        { path: '**', component: BlankComponent },
      ]),
    ]);
    asDesktop();
    serveAccounts(backend, buildAccounts(NOW, true));
    jobs = [
      job('j_move', 'move', {
        startedAt: NOW - 3_000_000,
        from: 'default',
        to: MOCK_ACCOUNT_WORK,
        undoUntil: NOW + 86_400_000,
      }),
      job('j_backup', 'backup', { startedAt: NOW - 2_000_000, account: 'default' }),
      job('j_delete', 'delete', {
        startedAt: NOW - 1_000_000,
        account: MOCK_ACCOUNT_WORK,
        undoUntil: NOW + 86_400_000,
      }),
    ];
    backend.routes.set('/data/jobs', () => ({ status: 200, body: { jobs } }));
    harness = await RouterTestingHarness.create();
  });

  afterEach(() => {
    TestBed.inject(RealtimeService).stop();
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
  const open = async (url = '/accounts/changes') => {
    await harness.navigateByUrl(url);
    await settle();
    return harness.routeNativeElement as HTMLElement;
  };
  const titles = (el: HTMLElement) =>
    [...el.querySelectorAll('mc-data-job .jtitle')].map((t) => text(t));
  const chip = (el: HTMLElement, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('.chips button')].find((b) => text(b) === label)!;

  it('lists every data job newest first, with Undo while it is possible', async () => {
    const el = await open();
    expect(titles(el)).toEqual([
      'Delete from Work',
      'Backup of Default',
      'Move from Default to Work',
    ]);
    expect(el.querySelectorAll('mc-data-job .undo .btn').length).toBe(2);
    expect(text(el.querySelector('.section-head h2'))).toBe('Recent changes');
    expect(text(el.querySelector('.section-head .muted'))).toBe('Undo is possible for 7 days');
  });

  it('shows one kind with its chip, from the link that opened it too', async () => {
    let el = await open('/accounts/changes?show=moves');
    expect(chip(el, 'Moves and copies').getAttribute('aria-pressed')).toBe('true');
    expect(el.querySelectorAll('mc-data-job').length).toBe(1);
    chip(el, 'Backups').click();
    await settle();
    el = harness.routeNativeElement as HTMLElement;
    expect(TestBed.inject(Router).url).toBe('/accounts/changes?show=backups');
    expect(el.querySelectorAll('mc-data-job').length).toBe(1);
    chip(el, 'All').click();
    await settle();
    expect(TestBed.inject(Router).url).toBe('/accounts/changes');
    expect((harness.routeNativeElement as HTMLElement).querySelectorAll('mc-data-job').length).toBe(
      3,
    );
  });

  it('takes a job that starts elsewhere as its frame comes, and says when there is none', async () => {
    jobs = [];
    const el = await open();
    expect(text(el.querySelector('.empty-state strong'))).toBe('No change yet');
    jobs = [
      job('j_new', 'pull', {
        state: 'failed',
        error: 'The folder is offline.',
        account: 'default',
      }),
    ];
    (TestBed.inject(RealtimeService) as unknown as { dispatch(raw: string): void }).dispatch(
      JSON.stringify({ t: 'jobs', id: 'j_new', kind: 'pull', state: 'failed' }),
    );
    await new Promise((r) => setTimeout(r, 320));
    await settle();
    expect(
      TestBed.inject(DataJobsService)
        .recent()
        .map((j) => j.id),
    ).toEqual(['j_new']);
    expect(el.querySelectorAll('mc-data-job').length).toBe(1);
    expect(text(el.querySelector('mc-data-job'))).toContain('The folder is offline');
  });
});
