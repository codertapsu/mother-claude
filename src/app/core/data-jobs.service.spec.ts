import { TestBed } from '@angular/core/testing';

import { DataJobsService } from './data-jobs.service';
import { FakeBackend, useFakeServer } from './mock/fake-server.testing';
import { setActiveMock } from './mock/mock-api';
import { JobView } from './models';

const job = (over: Partial<JobView> = {}): JobView => ({
  id: 'job1',
  kind: 'copy',
  state: 'running',
  startedAt: 1,
  filesDone: 0,
  filesTotal: 4,
  bytesDone: 0,
  bytesTotal: 400,
  sessions: [],
  ...over,
});

describe('DataJobsService', () => {
  let backend: FakeBackend;
  let jobs: DataJobsService;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend);
    jobs = TestBed.inject(DataJobsService);
    jobs.pollMs = 1;
  });

  afterEach(() => setActiveMock(null));

  it('follows a running job until it ends, passing every view on', async () => {
    const answers = [
      job({ bytesDone: 200, filesDone: 2 }),
      job({ state: 'done', bytesDone: 400, filesDone: 4, undoUntil: 99 }),
    ];
    backend.routes.set('/data/jobs/job1', () => ({ status: 200, body: answers.shift() }));
    const seen: string[] = [];
    const last = await jobs.follow(job(), (j) => seen.push(`${j.state}:${j.bytesDone}`));
    expect(seen).toEqual(['running:0', 'running:200', 'done:400']);
    expect(last?.state).toBe('done');
    expect(backend.calls.filter((c) => c === 'GET /data/jobs/job1').length).toBe(2);
  });

  it('reads a job named only by its id at once', async () => {
    backend.routes.set('/data/jobs/job2', () => ({
      status: 200,
      body: job({ id: 'job2', kind: 'restore', state: 'done' }),
    }));
    const last = await jobs.follow('job2', () => undefined);
    expect(last?.kind).toBe('restore');
    expect(backend.calls).toEqual(['GET /data/jobs/job2']);
  });

  it('stops when told to, and gives up on a job it cannot read', async () => {
    backend.routes.set('/data/jobs/job1', () => ({ status: 200, body: job() }));
    const ctrl = new AbortController();
    const following = jobs.follow(job(), () => ctrl.abort(), ctrl.signal);
    expect((await following)?.state).toBe('running');

    backend.routes.set('/data/jobs/gone', () => ({
      status: 404,
      body: { error: { code: 'job_not_found', message: 'No such data job.' } },
    }));
    await expectAsync(jobs.follow('gone', () => undefined)).toBeRejected();
    expect(backend.calls.filter((c) => c === 'GET /data/jobs/gone').length).toBe(4);
  });
});
