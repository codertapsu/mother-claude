import { Injectable, inject } from '@angular/core';

import { ApiService } from './api.service';
import { JOB_POLL_MS, normalizeJob } from './data-jobs';
import { JobView } from './models';

/** Answers in a row that may fail before following a job gives up. */
const MAX_FAILURES = 4;

/**
 * Following data jobs (moves, copies, backups, restores, pulls): ask
 * `GET /data/jobs/{id}` every {@link pollMs} while the job runs. Any signed-in
 * client may read a job; starting and undoing them is the desktop's.
 */
@Injectable({ providedIn: 'root' })
export class DataJobsService {
  private api = inject(ApiService);

  /** How often a running job is asked about (ms; specs shorten it). */
  pollMs = JOB_POLL_MS;

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
    if (view) onUpdate(view);
    let failures = 0;
    while (!signal?.aborted && (!view || view.state === 'running')) {
      // A bare id is read at once; a running view after a pause.
      if (view || failures) await sleep(this.pollMs, signal);
      if (signal?.aborted) break;
      try {
        const next = normalizeJob(await this.api.getDataJob(id));
        if (!next) throw new Error('The server sent a job this app could not read.');
        failures = 0;
        view = next;
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
