import { Injectable, inject, signal } from '@angular/core';

import { ApiService } from './api.service';
import { JOB_POLL_MS, normalizeJob } from './data-jobs';
import { tr } from './i18n/i18n';
import { JobView } from './models';

/** Answers in a row that may fail before following a job gives up. */
const MAX_FAILURES = 4;

/**
 * Following data jobs (moves, copies, backups, restores, pulls): ask
 * `GET /data/jobs/{id}` every {@link pollMs} while the job runs. Any signed-in
 * client may read a job; starting and undoing them is the desktop's.
 *
 * {@link recent} is `GET /data/jobs` as last read ({@link loadRecent}), each
 * job followed here replacing its copy: the Inbox finds the ones that failed
 * in it.
 */
@Injectable({ providedIn: 'root' })
export class DataJobsService {
  private api = inject(ApiService);

  /** How often a running job is asked about (ms; specs shorten it). */
  pollMs = JOB_POLL_MS;

  private readonly recentS = signal<readonly JobView[]>([]);
  /** The jobs the server keeps (newest last read), with what followed them. */
  readonly recent = this.recentS.asReadonly();

  /** Read `GET /data/jobs` again (a failure keeps what was read). */
  async loadRecent(): Promise<void> {
    try {
      const { jobs } = await this.api.listDataJobs();
      const list = (Array.isArray(jobs) ? jobs : [])
        .map((j) => normalizeJob(j))
        .filter((j): j is JobView => !!j);
      this.recentS.set(list);
    } catch {
      /* the last list stays */
    }
  }

  /** A job's newer view replaces (or joins) the recent list. */
  private note(job: JobView): void {
    this.recentS.update((list) =>
      list.some((j) => j.id === job.id)
        ? list.map((j) => (j.id === job.id ? job : j))
        : [...list, job],
    );
  }

  /**
   * Follow a job until it no longer runs: `onUpdate` gets every view (the
   * first one too, when `first` is a view). Resolves to the last view, or
   * null when it was never read (aborted first). Rejects with the read's
   * error when the job can't be read several times in a row.
   */
  async follow(
    first: JobView | string,
    onUpdate: (job: JobView) => void,
    signal?: AbortSignal,
  ): Promise<JobView | null> {
    let view = typeof first === 'string' ? null : normalizeJob(first);
    const id = typeof first === 'string' ? first : (view?.id ?? '');
    if (!id) return view;
    if (view) {
      this.note(view);
      onUpdate(view);
    }
    let failures = 0;
    while (!signal?.aborted && (!view || view.state === 'running')) {
      // A bare id is read at once; a running view after a pause.
      if (view || failures) await sleep(this.pollMs, signal);
      if (signal?.aborted) break;
      try {
        const next = normalizeJob(await this.api.getDataJob(id));
        if (!next) throw new Error(tr('The server sent a job this app could not read.'));
        failures = 0;
        view = next;
        this.note(next);
        onUpdate(next);
      } catch (e) {
        failures += 1;
        if (failures >= MAX_FAILURES) throw e;
      }
    }
    return view;
  }
}

/** A pause that ends early when `signal` aborts. */
function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal?.aborted) {
      resolve();
      return;
    }
    const done = () => {
      clearTimeout(timer);
      signal?.removeEventListener('abort', done);
      resolve();
    };
    const timer = setTimeout(done, ms);
    signal?.addEventListener('abort', done, { once: true });
  });
}
