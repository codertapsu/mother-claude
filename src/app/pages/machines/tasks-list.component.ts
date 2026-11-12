import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { RunsService } from '../../core/runs.service';
import { sessionLink } from '../../core/session-links';
import { MachineTask, taskChanges, taskEnded, taskStateLabel, taskTone } from '../../core/tasks';
import { ChipComponent } from '../../shared/chip.component';
import { IconComponent } from '../../shared/icon.component';
import { RelativeTimePipe } from '../../shared/time.pipes';
import { Tone } from '../../shared/util';

/** Tasks listed before *Show all*. */
const SHOWN = 6;

/**
 * Tasks on the Machines page's Runs card (docs/PLAN-MACHINES.md Phase 4,
 * decision 6): what this computer's sessions handed to a Claude Code session
 * on another computer, and what other computers handed this one — where,
 * the project, the task's first line, how it stands, its session (opened on
 * the computer it runs on), the branch its work became, its summary; Cancel
 * while it works, Remove once it ended (its worktree there goes with it,
 * never its branch).
 */
@Component({
  selector: 'mc-machines-tasks',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ChipComponent, IconComponent, RelativeTimePipe, RouterLink, TPipe],
  template: `
    <h3>{{ 'Tasks' | t }}</h3>
    <p class="muted small">
      {{
        'Tasks your sessions handed to a Claude Code session on another computer, and the ones other computers handed this one. Each is kept 7 days after it last ended, with its worktree there.'
          | t
      }}
    </p>
    @if (error()) {
      <p class="notice error small" role="alert">{{ error() }}</p>
    }
    @if (notice()) {
      <p class="notice small" role="status">{{ notice() }}</p>
    }
    <ul class="items tasks">
      @for (t of shown(); track t.id) {
        <li [attr.data-state]="t.state" [attr.data-direction]="t.direction">
          <div class="row">
            <mc-icon [name]="t.direction === 'out' ? 'upload' : 'download'" [size]="15" />
            <strong class="name">{{ where(t) }}</strong>
            @if (t.projectName) {
              <span class="muted small">{{ t.projectName }}</span>
            }
            <span class="spacer"></span>
            <mc-chip [tone]="tone(t)">{{ label(t) }}</mc-chip>
          </div>
          <p class="first">{{ t.firstLine || t.task }}</p>
          <span class="muted small meta">
            @if (t.branchName) {
              <span class="mono branch">{{ t.branchName }}</span>
              <span>{{ changes(t) }}</span>
            } @else if (ended(t) && t.results === 0 && t.state === 'done') {
              <span>{{ 'Changed nothing: no branch' | t }}</span>
            }
            @if (t.turns > 1) {
              <span>{{ '{n} turns' | t: { n: t.turns } }}</span>
            }
            @if (t.dangerous) {
              <span class="warn">{{ 'Without approvals' | t }}</span>
            }
            <span>{{ t.createdAt | relTime }}</span>
          </span>
          @if (t.unreachable && !ended(t)) {
            <p class="small detail warn">
              {{
                '{name} can’t be reached right now: Mother Claude keeps asking.'
                  | t: { name: t.machineName }
              }}
            </p>
          }
          @if (t.error) {
            <p class="small detail">{{ t.error }}</p>
          }
          @if (t.idleStoppedAt && t.state === 'done') {
            <p class="small detail muted">
              {{ 'Its session stopped after sitting idle: a message to it resumes it.' | t }}
            </p>
          }
          @if (t.branchNote) {
            <p class="small detail warn">{{ t.branchNote }}</p>
          }
          <div class="row acts">
            @if (t.taskSession) {
              <a class="btn ghost small" [routerLink]="link(t)">
                {{ 'Open its session' | t }}
              </a>
            }
            @if (t.summary) {
              <button
                type="button"
                class="btn ghost small"
                [attr.aria-expanded]="open() === t.id"
                (click)="toggle(t)"
              >
                {{ open() === t.id ? ('Hide summary' | t) : ('Summary' | t) }}
              </button>
            }
            @if (!ended(t)) {
              <button
                type="button"
                class="btn small danger"
                [disabled]="busy() === t.id"
                (click)="cancel(t)"
              >
                {{ busy() === t.id ? ('Cancelling…' | t) : ('Cancel' | t) }}
              </button>
            } @else {
              <button
                type="button"
                class="btn ghost small"
                [class.danger]="armed() === t.id"
                [disabled]="busy() === t.id"
                [attr.aria-label]="'Remove the task {task}' | t: { task: t.firstLine }"
                (click)="remove(t)"
              >
                {{ armed() === t.id ? ('Tap again to remove' | t) : ('Remove' | t) }}
              </button>
            }
          </div>
          @if (open() === t.id && t.summary) {
            <pre class="out" tabindex="0" [attr.aria-label]="'Its summary' | t">{{
              t.summary
            }}</pre>
          }
        </li>
      } @empty {
        <li class="muted small">{{ 'No task yet.' | t }}</li>
      }
    </ul>
    @if (runs.tasks().length > shown().length) {
      <button type="button" class="btn ghost small" (click)="all.set(true)">
        {{ 'Show all {n}' | t: { n: runs.tasks().length } }}
      </button>
    }
  `,
  styles: [
    `
      :host {
        display: block;
      }
      h3 {
        margin: 0.9rem 0 0.2rem;
        font-size: 0.95rem;
      }
      .items {
        list-style: none;
        margin: 0.3rem 0;
        padding: 0;
      }
      .items > li {
        display: flex;
        flex-direction: column;
        gap: 0.2rem;
        padding: 0.5rem 0;
        border-bottom: 1px solid var(--border);
      }
      .items > li:last-child {
        border-bottom: 0;
      }
      .row {
        flex-wrap: wrap;
      }
      .first {
        margin: 0;
        overflow-wrap: anywhere;
      }
      .meta {
        display: flex;
        flex-wrap: wrap;
        gap: 0.2rem 0.6rem;
      }
      .branch {
        overflow-wrap: anywhere;
      }
      .detail {
        margin: 0;
      }
      .warn,
      li[data-state='error'] .detail,
      li[data-state='lost'] .detail,
      li[data-state='stopped'] .detail {
        color: var(--amber);
      }
      .out {
        margin: 0.2rem 0 0;
        max-height: 18rem;
        overflow: auto;
        padding: 0.5rem 0.6rem;
        background: var(--code-bg);
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        font-size: 0.8rem;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
      }
      .acts {
        margin-top: 0.1rem;
      }
    `,
  ],
})
export class TasksListComponent {
  private readonly api = inject(ApiService);
  protected readonly runs = inject(RunsService);
  protected readonly busy = signal<string | null>(null);
  protected readonly armed = signal<string | null>(null);
  protected readonly error = signal('');
  protected readonly notice = signal('');
  protected readonly all = signal(false);
  /** The task whose summary is open. */
  protected readonly open = signal<string | null>(null);

  protected readonly shown = computed(() =>
    this.all() ? this.runs.tasks() : this.runs.tasks().slice(0, SHOWN),
  );

  protected where(t: MachineTask): string {
    return t.direction === 'out'
      ? tr('To {name}', { name: t.machineName })
      : tr('From {name}, on this {computer}', { name: t.machineName, computer: computerNoun() });
  }

  protected label(t: MachineTask): string {
    return taskStateLabel(t.state);
  }

  protected tone(t: MachineTask): Tone {
    return taskTone(t.state);
  }

  protected ended(t: MachineTask): boolean {
    return taskEnded(t.state);
  }

  protected changes(t: MachineTask): string {
    return taskChanges(t);
  }

  /** Its session's page: on the computer it runs on. */
  protected link(t: MachineTask): string[] {
    return sessionLink(t.taskSession ?? '', t.direction === 'out' ? t.machine : null);
  }

  protected toggle(t: MachineTask): void {
    this.open.set(this.open() === t.id ? null : t.id);
  }

  protected async cancel(t: MachineTask): Promise<void> {
    await this.withBusy(t.id, async () => {
      const d = await this.api.cancelTask(t.id);
      this.runs.applyTask({ ...d, task: d.task.slice(0, 500) });
    });
  }

  /** Twice: its worktree there goes with it. */
  protected async remove(t: MachineTask): Promise<void> {
    if (this.armed() !== t.id) {
      this.armed.set(t.id);
      return;
    }
    this.armed.set(null);
    await this.withBusy(t.id, async () => {
      const v = await this.api.removeTask(t.id);
      this.runs.dropTask(t.id);
      this.notice.set(
        v.there === 'unreachable'
          ? tr('Removed here. {name} removes its worktree once it is 7 days old.', {
              name: t.machineName,
            })
          : '',
      );
    });
  }

  private async withBusy(key: string, fn: () => Promise<void>): Promise<void> {
    this.busy.set(key);
    this.error.set('');
    this.notice.set('');
    try {
      await fn();
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(null);
    }
  }
}
