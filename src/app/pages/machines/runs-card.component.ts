import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  inject,
  input,
  signal,
} from '@angular/core';

import { RouterLink } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import { formatBytes } from '../../core/data-jobs';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { MachineRun, runEnded, runStateLabel, runTone, shellName } from '../../core/runs';
import { RunsService } from '../../core/runs.service';
import { ChipComponent } from '../../shared/chip.component';
import { IconComponent } from '../../shared/icon.component';
import { RelativeTimePipe } from '../../shared/time.pipes';
import { Tone, formatDuration } from '../../shared/util';
import { RolesListComponent } from './roles-list.component';
import { TasksListComponent } from './tasks-list.component';

/** Runs listed before *Show all*. */
const SHOWN = 8;
/** How often an open, running run's output is read again. */
const REFRESH_MS = 3000;

/**
 * Runs on Computers › Activity (docs/PLAN-MACHINES.md Phase 4): the commands
 * this computer's sessions ran on another computer, at their own code, and
 * the ones other computers ran here — where, which project, the command,
 * how it stands, its exit code and time, its output's end (read again
 * while it runs), and *Cancel* while it runs. Then the tasks handed to a
 * session on another computer, and taken from one (`mc-machines-tasks`),
 * and the team roles this computer runs for another's lead
 * (`mc-machines-roles`).
 * Below, a line to the commands and tasks allowed without asking: they are
 * permissions, on *Permissions* (`?on=others`).
 */
@Component({
  selector: 'mc-machines-runs',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    ChipComponent,
    IconComponent,
    RelativeTimePipe,
    RolesListComponent,
    RouterLink,
    TasksListComponent,
    TPipe,
  ],
  template: `
    <section class="card runs" [attr.aria-label]="'Runs' | t">
      @if (shows('commands')) {
        <div class="chead">
          <h2 class="first">{{ 'Runs' | t }}</h2>
        </div>
        <p class="muted small">
          {{
            'Commands your sessions run on another computer, at their own code, and the ones other computers run here. Each is kept a day.'
              | t
          }}
        </p>
        @if (error()) {
          <p class="notice error small" role="alert">{{ error() }}</p>
        }
        @if (loadError()) {
          <p class="notice error small" role="alert">{{ loadError() }}</p>
        }
        <ul class="list">
          @for (r of shown(); track r.id) {
            <li [attr.data-state]="r.state" [attr.data-direction]="r.direction">
              <div class="row">
                <mc-icon [name]="r.direction === 'out' ? 'upload' : 'download'" [size]="15" />
                <strong class="name">{{ where(r) }}</strong>
                @if (r.projectName) {
                  <span class="muted small">{{ r.projectName }}</span>
                }
                <span class="spacer"></span>
                <mc-chip [tone]="tone(r)">{{ label(r) }}</mc-chip>
              </div>
              <code class="mono cmd">{{ r.command }}</code>
              <span class="muted small meta">
                @if (r.exitCode !== null && r.exitCode !== undefined) {
                  <span>{{ 'exit code {n}' | t: { n: r.exitCode } }}</span>
                }
                @if (r.durationMs) {
                  <span>{{ duration(r.durationMs) }}</span>
                }
                @if (r.outputSize) {
                  <span>{{ size(r.outputSize) }}</span>
                }
                @if (shell(r); as s) {
                  <span>{{ s }}</span>
                }
                <span>{{ r.createdAt | relTime }}</span>
              </span>
              @if (r.unreachable && !ended(r)) {
                <p class="small detail warn">
                  {{
                    '{name} can’t be reached right now: Mother Claude keeps asking.'
                      | t: { name: r.machineName }
                  }}
                </p>
              }
              @if (r.error) {
                <p class="small detail">{{ r.error }}</p>
              }
              <div class="row acts">
                @if (r.tail || r.outputSize) {
                  <button
                    type="button"
                    class="btn ghost small"
                    [attr.aria-expanded]="open() === r.id"
                    (click)="toggle(r)"
                  >
                    {{ open() === r.id ? ('Hide output' | t) : ('Output' | t) }}
                  </button>
                }
                @if (!ended(r)) {
                  <button
                    type="button"
                    class="btn small danger"
                    [disabled]="busy() === r.id"
                    (click)="cancel(r)"
                  >
                    {{ busy() === r.id ? ('Cancelling…' | t) : ('Cancel' | t) }}
                  </button>
                }
              </div>
              @if (open() === r.id) {
                <pre
                  class="out"
                  tabindex="0"
                  [attr.aria-label]="'Output of {command}' | t: { command: r.command }"
                  >{{ outputOf(r) || ('No output yet.' | t) }}</pre>
                @if (r.artifacts.length) {
                  <p class="small">{{ 'Files it made:' | t }}</p>
                  <ul class="files small">
                    @for (a of r.artifacts; track a.path) {
                      <li>
                        <span class="mono">{{ a.path }}</span>
                        <span class="muted"> {{ size(a.size) }}</span>
                      </li>
                    }
                  </ul>
                }
              }
            </li>
          } @empty {
            <li class="muted small">{{ 'No run yet.' | t }}</li>
          }
        </ul>
        @if (runs.runs().length > shown().length) {
          <button type="button" class="btn ghost small" (click)="all.set(true)">
            {{ 'Show all {n}' | t: { n: runs.runs().length } }}
          </button>
        }
      }

      @if (shows('tasks')) {
        <mc-machines-tasks />
      }
      @if (shows('roles')) {
        <mc-machines-roles />
        @if (part() === 'roles' && !runs.roles().length) {
          <p class="muted small">{{ 'No team role runs here for another computer.' | t }}</p>
        }
      }

      @if (shows('commands')) {
        <p class="small allowed">
          <mc-icon name="shield" [size]="15" />
          <span>
            {{
              'Commands and tasks your sessions may run on another computer without asking are kept in Permissions.'
                | t
            }}
            <a routerLink="/permissions" [queryParams]="{ on: 'others' }">{{
              'Open Permissions' | t
            }}</a>
          </span>
        </p>
      }
    </section>
  `,
  styles: [
    `
      .list {
        list-style: none;
        margin: 0.3rem 0;
        padding: 0;
      }
      .list > li {
        display: flex;
        flex-direction: column;
        gap: 0.2rem;
        padding: 0.5rem 0;
        border-bottom: 1px solid var(--border);
      }
      .list > li:last-child {
        border-bottom: 0;
      }
      .row {
        flex-wrap: wrap;
      }
      .cmd {
        overflow-wrap: anywhere;
        font-size: 0.8rem;
      }
      .meta {
        display: flex;
        flex-wrap: wrap;
        gap: 0.2rem 0.6rem;
      }
      .detail {
        margin: 0;
      }
      .warn,
      li[data-state='failed'] .detail,
      li[data-state='error'] .detail,
      li[data-state='lost'] .detail {
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
        font-size: 0.75rem;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
      }
      .files {
        margin: 0;
        padding-left: 1rem;
      }
      .acts {
        margin-top: 0.1rem;
      }
      .allowed {
        display: flex;
        align-items: flex-start;
        gap: 0.45rem;
        margin: 0.9rem 0 0;
        padding-top: 0.75rem;
        border-top: 1px solid var(--border);
        color: var(--muted);
      }
      .allowed mc-icon {
        flex: 0 0 auto;
        margin-top: 0.1rem;
        color: var(--accent-text);
      }
    `,
  ],
})
export class RunsCardComponent {
  /** The part shown (Computers › Activity's chips): all of it, the
   * commands (with the line to the ones allowed without asking), the tasks
   * or the team roles. */
  readonly part = input<'all' | 'commands' | 'tasks' | 'roles'>('all');

  private readonly api = inject(ApiService);
  protected readonly runs = inject(RunsService);
  protected readonly loadError = this.runs.error;
  protected readonly busy = signal<string | null>(null);
  protected readonly error = signal('');
  protected readonly all = signal(false);
  /** The run whose output is open, and its whole record once read. */
  protected readonly open = signal<string | null>(null);
  private readonly detail = signal<MachineRun | null>(null);
  private timer?: ReturnType<typeof setInterval>;

  protected shows(part: 'commands' | 'tasks' | 'roles'): boolean {
    return this.part() === 'all' || this.part() === part;
  }

  protected readonly shown = computed(() =>
    this.all() ? this.runs.runs() : this.runs.runs().slice(0, SHOWN),
  );

  constructor() {
    const release = this.runs.watch();
    inject(DestroyRef).onDestroy(() => {
      release();
      clearInterval(this.timer);
    });
  }

  protected where(r: MachineRun): string {
    return r.direction === 'out'
      ? tr('On {name}', { name: r.machineName })
      : tr('From {name}, on this {computer}', { name: r.machineName, computer: computerNoun() });
  }

  protected label(r: MachineRun): string {
    return runStateLabel(r.state);
  }

  protected tone(r: MachineRun): Tone {
    return runTone(r.state);
  }

  protected ended(r: MachineRun): boolean {
    return runEnded(r.state);
  }

  protected shell(r: MachineRun): string {
    return r.shell ? shellName(r.shell) : '';
  }

  protected duration(ms: number): string {
    return formatDuration(ms);
  }

  protected size(n: number): string {
    return formatBytes(n);
  }

  /** The open run's output: its whole record's when read, else the list's
   * end of it. */
  protected outputOf(r: MachineRun): string {
    const d = this.detail();
    return d?.id === r.id ? d.tail : r.tail;
  }

  protected toggle(r: MachineRun): void {
    clearInterval(this.timer);
    if (this.open() === r.id) {
      this.open.set(null);
      this.detail.set(null);
      return;
    }
    this.open.set(r.id);
    this.detail.set(null);
    void this.read(r.id);
    if (!runEnded(r.state)) {
      this.timer = setInterval(() => {
        if (document.visibilityState === 'visible') void this.read(r.id);
      }, REFRESH_MS);
    }
  }

  private async read(id: string): Promise<void> {
    try {
      const d = await this.api.getRun(id);
      if (this.open() !== id) return;
      this.detail.set(d);
      this.runs.applyRun({ ...d, tail: d.tail.slice(-2048) });
      if (runEnded(d.state)) clearInterval(this.timer);
    } catch (e) {
      this.error.set(describeError(e).text);
    }
  }

  protected async cancel(r: MachineRun): Promise<void> {
    await this.withBusy(r.id, async () => {
      const d = await this.api.cancelRun(r.id);
      this.runs.applyRun({ ...d, tail: d.tail.slice(-2048) });
      if (this.open() === r.id) this.detail.set(d);
    });
  }

  private async withBusy(key: string, fn: () => Promise<void>): Promise<void> {
    this.busy.set(key);
    this.error.set('');
    try {
      await fn();
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(null);
    }
  }
}
