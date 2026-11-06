import { NgTemplateOutlet } from '@angular/common';
import { Component, DestroyRef, computed, inject, signal } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { FormsModule } from '@angular/forms';
import { ActivatedRoute, Router, RouterLink } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import { pathWithin } from '../../core/paths';
import { RealtimeService } from '../../core/realtime.service';
import {
  PRIORITY_LABELS,
  ROLE_STATE_TONE,
  RoleView,
  STATUS_LABELS,
  TASK_STATUSES,
  TaskStatus,
  TeamTask,
  TeamView,
  columns,
  roleStatusLine,
  upsertTask,
} from '../../core/teams';
import { ViewportService } from '../../core/viewport.service';
import { IconComponent } from '../../shared/icon.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';
import { NewTaskSheetComponent } from './new-task-sheet.component';
import { TaskSheetComponent } from './task-sheet.component';

/** One team: its roles (state, session, model, budget, why one waits) and
 * its board. A phone follows it, adds tasks, reprioritises and cancels. */
@Component({
  selector: 'mc-team',
  imports: [
    FormsModule,
    NgTemplateOutlet,
    RouterLink,
    IconComponent,
    TimeAgoComponent,
    NewTaskSheetComponent,
    TaskSheetComponent,
  ],
  template: `
    <div class="page">
      <a class="back muted small" routerLink="/teams"><mc-icon name="back" [size]="14" /> Teams</a>
      @if (team(); as t) {
        <div class="page-head">
          <h1>{{ t.label }}</h1>
          @if (t.stopped) {
            <span class="badge">stopped</span>
          }
          <span class="muted small">{{ t.templateName }}</span>
          <span class="spacer"></span>
          @if (t.stopped) {
            <button class="btn" type="button" [disabled]="busy()" (click)="resume()">
              <mc-icon name="play" [size]="14" /> Resume
            </button>
            <button class="btn ghost" type="button" [disabled]="busy()" (click)="remove()">
              Remove
            </button>
          } @else {
            <button class="btn" type="button" [disabled]="busy()" (click)="stop()">
              <mc-icon name="stop" [size]="14" /> Stop the team
            </button>
          }
        </div>
        <div class="mono small path">{{ t.project }}</div>
        @if (t.brief) {
          <details class="brief">
            <summary class="muted small">The lead's brief</summary>
            <p class="small">{{ t.brief }}</p>
          </details>
        }
        @if (error()) {
          <p class="notice error" role="alert">{{ error() }}</p>
        }

        <h2>Roles</h2>
        <div class="roles">
          @for (r of t.roles; track r.id) {
            <div class="card role">
              <div class="row">
                <strong>{{ r.title }}</strong>
                @if (r.lead) {
                  <span class="badge accent">lead</span>
                }
                <span [class]="'badge ' + tone(r)">{{ statusLine(r) }}</span>
              </div>
              @if (r.sessionId) {
                <a class="mono small" [routerLink]="['/session', r.sessionId]">{{
                  r.sessionName
                }}</a>
              } @else {
                <span class="mono small faint">{{ r.sessionName }}</span>
              }
              @if (r.state === 'waiting' && r.waitingSince) {
                <span class="small muted">Waiting <mc-time [ms]="r.waitingSince" /></span>
              }
              <div class="small facts">
                <span>{{ r.model || 'default model' }}{{ r.effort ? ' @ ' + r.effort : '' }}</span>
                <span>{{ r.permissionMode }}</span>
                @if (r.budgetUsd) {
                  <span
                    [title]="
                      r.budgetApplied
                        ? 'Applied (--max-budget-usd)'
                        : 'Not applied: the account runs on its plan'
                    "
                    >\${{ r.budgetUsd }}{{ r.budgetApplied ? '' : ' (plan)' }}</span
                  >
                }
                @if (r.readOnly) {
                  <span>read-only</span>
                }
                @if (r.foreign) {
                  <span>foreign session</span>
                }
              </div>
              @if (r.worktree) {
                <div class="mono small faint wrap">worktree {{ r.worktreePath }}</div>
              }
              @if (r.paths.length && !r.readOnly) {
                <div class="small faint wrap">owns {{ r.paths.join(', ') }}</div>
              }
              <div class="small muted">
                @if (r.currentTask) {
                  On task #{{ r.currentTask }} ·
                }
                {{ r.queuedTasks }} waiting
                @if (r.guardDenials) {
                  · {{ r.guardDenials }} guard denials
                }
              </div>
              @if (!t.stopped) {
                <div class="row actions">
                  @if (r.state === 'running' || r.state === 'idle') {
                    @if (r.foreign) {
                      <button class="btn small" type="button" (click)="release(r)">
                        Release role
                      </button>
                    } @else {
                      <button class="btn small" type="button" (click)="stopRole(r)">
                        <mc-icon name="stop" [size]="13" /> Stop
                      </button>
                    }
                  } @else if (r.state !== 'starting') {
                    <button class="btn small" type="button" (click)="startRole(r)">
                      <mc-icon name="play" [size]="13" /> {{ r.sessionId ? 'Resume' : 'Start' }}
                    </button>
                    @if (foreignCandidates().length) {
                      <select
                        class="take"
                        [attr.aria-label]="'Give the ' + r.title + ' role to a running session'"
                        [ngModel]="''"
                        (ngModelChange)="take(r, $event)"
                      >
                        <option value="" disabled>Take role…</option>
                        @for (s of foreignCandidates(); track s.id) {
                          <option [value]="s.id">{{ s.name || s.id.slice(0, 8) }}</option>
                        }
                      </select>
                    }
                  }
                </div>
              }
            </div>
          }
        </div>

        <div class="row board-head">
          <h2>Board</h2>
          <span class="spacer"></span>
          <button class="btn small primary" type="button" (click)="newOpen.set(true)">
            <mc-icon name="plus" [size]="14" /> New task
          </button>
        </div>
        @if (viewport.phone()) {
          <label class="field status-pick">
            <span class="field-label">Status</span>
            <select [ngModel]="status()" (ngModelChange)="status.set($event)">
              @for (s of statuses; track s) {
                <option [value]="s">{{ statusLabel(s) }} ({{ byStatus()[s].length }})</option>
              }
            </select>
          </label>
          <div class="stack">
            @for (task of byStatus()[status()]; track task.id) {
              <ng-container *ngTemplateOutlet="card; context: { $implicit: task }"></ng-container>
            } @empty {
              <p class="muted small">Nothing here.</p>
            }
          </div>
        } @else {
          <div class="board">
            @for (s of statuses; track s) {
              <section class="column" [attr.aria-label]="statusLabel(s)">
                <h3 class="small">
                  {{ statusLabel(s) }} <span class="faint">{{ byStatus()[s].length }}</span>
                </h3>
                @for (task of byStatus()[s]; track task.id) {
                  <ng-container
                    *ngTemplateOutlet="card; context: { $implicit: task }"
                  ></ng-container>
                }
              </section>
            }
          </div>
        }
        <ng-template #card let-task>
          <button class="card task" type="button" (click)="selected.set(task)">
            <span class="small"
              ><span class="faint">#{{ task.id }}</span> {{ task.title }}</span
            >
            <span class="row chips">
              @if (task.role) {
                <span class="badge">{{ task.role }}</span>
              }
              <span class="badge" [class.warn]="task.priority <= 1">P{{ task.priority }}</span>
              @if (task.blocked) {
                <span class="badge warn">blocked</span>
              }
              @if (task.escalations) {
                <span class="badge info">escalated {{ task.escalations }}</span>
              }
            </span>
          </button>
        </ng-template>
      } @else if (loading()) {
        <p class="muted">Loading…</p>
      } @else {
        <p class="notice error">{{ error() || 'No such team.' }}</p>
      }
    </div>

    @if (team(); as t) {
      <mc-new-task-sheet
        [open]="newOpen()"
        [team]="t"
        (closed)="newOpen.set(false)"
        (created)="created($event)"
      />
      <mc-task-sheet
        [open]="!!selected()"
        [team]="t"
        [task]="selected()"
        (closed)="selected.set(null)"
        (changed)="changed($event)"
      />
    }
  `,
  styles: [
    `
      .back {
        display: inline-flex;
        align-items: center;
        gap: 0.25rem;
        margin-bottom: 0.4rem;
        text-decoration: none;
      }
      .path {
        color: var(--muted);
        overflow-wrap: anywhere;
        margin: -0.4rem 0 0.75rem;
      }
      .brief p {
        white-space: pre-wrap;
      }
      .roles {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(17rem, 1fr));
        gap: 0.6rem;
      }
      .role {
        display: flex;
        flex-direction: column;
        gap: 0.3rem;
        min-width: 0;
      }
      .card + .card {
        margin-top: 0;
      }
      .facts {
        display: flex;
        flex-wrap: wrap;
        gap: 0.2rem 0.6rem;
        color: var(--muted);
      }
      .wrap {
        overflow-wrap: anywhere;
      }
      .actions {
        margin-top: 0.2rem;
      }
      .take {
        max-width: 11rem;
      }
      .board-head {
        margin-top: 1.25rem;
      }
      .board-head h2 {
        margin: 0;
      }
      .board {
        display: grid;
        grid-auto-flow: column;
        grid-auto-columns: minmax(13rem, 1fr);
        gap: 0.6rem;
        overflow-x: auto;
        padding-bottom: 0.5rem;
      }
      .column {
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
        min-width: 0;
      }
      .column h3 {
        margin: 0.25rem 0;
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.04em;
      }
      .status-pick {
        margin: 0.5rem 0;
      }
      .task {
        display: flex;
        flex-direction: column;
        gap: 0.35rem;
        text-align: left;
        width: 100%;
        cursor: pointer;
        color: inherit;
        font: inherit;
        overflow-wrap: anywhere;
      }
      .task:hover {
        border-color: var(--accent);
      }
      .chips {
        gap: 0.25rem;
      }
    `,
  ],
})
export class TeamComponent {
  private api = inject(ApiService);
  private realtime = inject(RealtimeService);
  private router = inject(Router);
  protected readonly viewport = inject(ViewportService);

  private readonly id = inject(ActivatedRoute).snapshot.paramMap.get('id') ?? '';
  protected readonly team = signal<TeamView | null>(null);
  protected readonly tasks = signal<TeamTask[]>([]);
  protected readonly loading = signal(true);
  protected readonly busy = signal(false);
  protected readonly error = signal('');
  protected readonly newOpen = signal(false);
  protected readonly selected = signal<TeamTask | null>(null);
  protected readonly status = signal<TaskStatus>('todo');
  protected readonly statuses = TASK_STATUSES;
  protected readonly priorities = PRIORITY_LABELS;

  protected readonly byStatus = computed(() => columns(this.tasks()));
  /** Live sessions Mother Claude does not run, in this team's project. */
  protected readonly foreignCandidates = computed(() => {
    const t = this.team();
    if (!t) return [];
    const taken = new Set(t.roles.map((r) => r.sessionId).filter(Boolean));
    return this.realtime
      .sessions()
      .filter((s) => s.live && !s.owned && !taken.has(s.id) && pathWithin(s.cwd, t.project));
  });

  constructor() {
    void this.load();
    const destroy = inject(DestroyRef);
    this.realtime.teams$.pipe(takeUntilDestroyed(destroy)).subscribe((f) => {
      if (f.team?.id === this.id) this.team.set(f.team);
      if (f.task && f.teamId === this.id) {
        const task = f.task;
        this.tasks.update((list) => upsertTask(list, task));
        if (this.selected()?.id === task.id) this.selected.set(task);
      }
      if (f.removed === this.id) void this.router.navigate(['/teams']);
    });
    this.realtime.refetch$.pipe(takeUntilDestroyed(destroy)).subscribe(() => void this.load());
  }

  protected tone(r: RoleView): string {
    return ROLE_STATE_TONE[r.state] ?? '';
  }

  protected statusLine(r: RoleView): string {
    return roleStatusLine(r);
  }

  protected statusLabel(s: TaskStatus): string {
    return STATUS_LABELS[s];
  }

  protected async load(): Promise<void> {
    this.loading.set(true);
    try {
      const [team, tasks] = await Promise.all([
        this.api.getTeam(this.id),
        this.api.getTeamTasks(this.id),
      ]);
      this.team.set(team);
      this.tasks.set(tasks.tasks);
      this.error.set('');
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.loading.set(false);
    }
  }

  private async act(fn: () => Promise<TeamView | void>): Promise<void> {
    this.busy.set(true);
    this.error.set('');
    try {
      const view = await fn();
      if (view) this.team.set(view);
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(false);
    }
  }

  protected stop(): void {
    if (!confirm('Stop every role session of this team? Their conversations are kept.')) return;
    void this.act(() => this.api.stopTeam(this.id));
  }

  protected resume(): void {
    void this.act(() => this.api.resumeTeam(this.id));
  }

  protected remove(): void {
    if (!confirm('Remove this team? Its board stays with the project.')) return;
    void this.act(async () => {
      await this.api.removeTeam(this.id);
      await this.router.navigate(['/teams']);
    });
  }

  protected startRole(r: RoleView): void {
    void this.act(async () => {
      const out = await this.api.startTeamRole(this.id, r.id);
      if (out.state === 'waiting') {
        this.error.set(`The ${r.title} role waits: ${(out.reasons ?? []).join('; ')}`);
      }
      return this.api.getTeam(this.id);
    });
  }

  protected stopRole(r: RoleView): void {
    void this.act(() => this.api.stopTeamRole(this.id, r.id));
  }

  protected release(r: RoleView): void {
    void this.act(() => this.api.releaseTeamRole(this.id, r.id));
  }

  protected take(r: RoleView, sessionId: string): void {
    if (!sessionId) return;
    void this.act(() => this.api.takeTeamRole(this.id, r.id, sessionId));
  }

  protected created(task: TeamTask): void {
    this.newOpen.set(false);
    this.tasks.update((list) => upsertTask(list, task));
  }

  protected changed(task: TeamTask): void {
    this.tasks.update((list) => upsertTask(list, task));
    this.selected.set(task);
  }
}
