import { Injectable, computed, inject, signal } from '@angular/core';

import { ApiError, ApiService, describeError } from './api.service';
import { RealtimeService } from './realtime.service';
import { MachineRun, RunGrant } from './runs';
import { MachineTask, taskEnded } from './tasks';

/**
 * Runs and tasks on other computers (docs/PLAN-MACHINES.md Phase 4) for
 * every screen that shows them — the Machines page's Runs card, its tasks
 * and its standing grants: `GET /api/machines/runs`, `/api/machines/tasks`
 * and `/api/machines/run-grants`, refetched on each `{t:'machines', what:
 * 'runs' | 'tasks' | 'runGrants'}` frame (and when a computer comes or
 * goes) while a screen watches.
 */
@Injectable({ providedIn: 'root' })
export class RunsService {
  private readonly api = inject(ApiService);
  private readonly realtime = inject(RealtimeService);

  readonly runs = signal<MachineRun[]>([]);
  readonly tasks = signal<MachineTask[]>([]);
  readonly grants = signal<RunGrant[]>([]);
  /** The last load's failure ('' when it worked). */
  readonly error = signal('');
  readonly running = computed(() =>
    this.runs().filter((r) => ['sending', 'queued', 'preparing', 'running'].includes(r.state)),
  );
  readonly tasksRunning = computed(() => this.tasks().filter((t) => !taskEnded(t.state)));

  private watchers = 0;
  private loading: Promise<void> | null = null;
  private again = false;

  constructor() {
    this.realtime.machines$.subscribe((f) => {
      if (
        this.watchers > 0 &&
        ['runs', 'tasks', 'runGrants', 'peers', 'enabled'].includes(f.what)
      ) {
        void this.load();
      }
    });
    this.realtime.refetch$.subscribe(() => {
      if (this.watchers > 0) void this.load();
    });
  }

  /** A screen shows runs: loads them, and returns its release. */
  watch(): () => void {
    this.watchers += 1;
    void this.load();
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.watchers -= 1;
    };
  }

  /** Every list (one load at a time; a request meanwhile asks again). */
  load(): Promise<void> {
    if (this.loading) {
      this.again = true;
      return this.loading;
    }
    this.loading = this.fetch().finally(() => {
      this.loading = null;
      if (this.again) {
        this.again = false;
        void this.load();
      }
    });
    return this.loading;
  }

  private async fetch(): Promise<void> {
    try {
      const [runs, grants, tasks] = await Promise.all([
        this.api.getRuns(),
        this.api.getRunGrants(),
        // A server that hands no tasks over yet has no such route.
        this.api.getTasks().catch((e: unknown) => {
          if (e instanceof ApiError && e.status === 404) return { tasks: [] };
          throw e;
        }),
      ]);
      this.runs.set(runs.runs ?? []);
      this.grants.set(grants.grants ?? []);
      this.tasks.set(tasks.tasks ?? []);
      this.error.set('');
    } catch (e) {
      this.error.set(describeError(e).text);
    }
  }

  /** One run's answer (a cancel, a refresh) over the list's copy. */
  applyRun(r: MachineRun | null | undefined): void {
    if (!r) return;
    this.runs.update((list) => list.map((x) => (x.id === r.id ? { ...x, ...r } : x)));
  }

  /** One task's answer (a cancel, a refresh) over the list's copy. */
  applyTask(t: MachineTask | null | undefined): void {
    if (!t) return;
    this.tasks.update((list) => list.map((x) => (x.id === t.id ? { ...x, ...t } : x)));
  }

  /** A task removed. */
  dropTask(id: string): void {
    this.tasks.update((list) => list.filter((x) => x.id !== id));
  }

  applyGrants(list: RunGrant[] | null | undefined): void {
    if (list) this.grants.set(list);
  }
}
