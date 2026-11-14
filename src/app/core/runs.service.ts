import { Injectable, computed, inject, signal } from '@angular/core';

import { ApiError, ApiService, describeError } from './api.service';
import { RealtimeService } from './realtime.service';
import { MachineRun } from './runs';
import { HostedRole, MachineTask, taskEnded } from './tasks';

/**
 * Runs and tasks on other computers (docs/PLAN-MACHINES.md Phase 4) for
 * every screen that shows them — the Machines page's Runs card, its tasks
 * and the team roles it runs for other computers: `GET /api/machines/runs`,
 * `/api/machines/tasks` and `/api/machines/roles`, refetched on each
 * `{t:'machines', what: 'runs' | 'tasks' | 'roles'}` frame (and when a
 * computer comes or goes) while a screen watches. The commands allowed
 * without asking are permissions (`PermissionRulesService`).
 */
@Injectable({ providedIn: 'root' })
export class RunsService {
  private readonly api = inject(ApiService);
  private readonly realtime = inject(RealtimeService);

  readonly runs = signal<MachineRun[]>([]);
  readonly tasks = signal<MachineTask[]>([]);
  /** Team roles this computer runs for other computers' leads. */
  readonly roles = signal<HostedRole[]>([]);
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
      if (this.watchers > 0 && ['runs', 'tasks', 'roles', 'peers', 'enabled'].includes(f.what)) {
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
      const [runs, tasks, roles] = await Promise.all([
        this.api.getRuns(),
        // A server that hands no tasks over yet has no such route.
        this.api.getTasks().catch((e: unknown) => {
          if (e instanceof ApiError && e.status === 404) return { tasks: [] };
          throw e;
        }),
        // Nor one that runs no team roles for others.
        this.api.getHostedRoles().catch((e: unknown) => {
          if (e instanceof ApiError && e.status === 404) return { roles: [] };
          throw e;
        }),
      ]);
      this.runs.set(runs.runs ?? []);
      this.tasks.set(tasks.tasks ?? []);
      this.roles.set(roles.roles ?? []);
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
}
