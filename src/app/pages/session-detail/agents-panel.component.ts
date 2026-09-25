import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { AgentInfo, BackgroundTask, WorkflowRun } from '../../core/models';
import { ActivityLineComponent } from '../../shared/activity-line.component';
import {
  agentKindIcon,
  agentKindLabel,
  agentStatusTone,
  orderAgentTree,
} from '../../shared/agents';
import { ChipComponent } from '../../shared/chip.component';
import { IconComponent } from '../../shared/icon.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';
import { formatDuration, formatTokens } from '../../shared/util';
import { SessionContext } from './session-context';
import { TaskOutputComponent } from './task-output.component';

/**
 * Everything the session has running on the side: its subagents as a tree
 * (nesting by parent), workflow runs with their phases and per-agent state,
 * and background tasks (shells, monitors) with their output tail. Stop
 * buttons where the session allows it; tap an agent for its transcript.
 */
@Component({
  selector: 'mc-agents-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    NgTemplateOutlet,
    RouterLink,
    ActivityLineComponent,
    ChipComponent,
    IconComponent,
    TaskOutputComponent,
    TimeAgoComponent,
  ],
  template: `
    @if (!tree().length && !runs().length && !tasks().length) {
      <div class="card empty-state">
        <strong>No agents</strong>
        Subagents, workflows and background shells this session starts show up here while they work.
      </div>
    }

    @for (w of runs(); track w.run.runId) {
      <section class="card run" [attr.aria-label]="'Workflow ' + w.run.name">
        <header class="rhead">
          <mc-icon name="workflow" [size]="18" />
          <strong class="rname">{{ w.run.name }}</strong>
          <mc-chip
            [tone]="
              w.run.status === 'running' ? 'green' : w.run.status === 'completed' ? 'muted' : 'red'
            "
            >{{ w.run.status }}</mc-chip
          >
        </header>
        <p class="rstats">
          <span>{{ w.run.agents.running }} running</span>
          <span>{{ w.run.agents.done }} done</span>
          @if (w.run.agents.failed) {
            <span class="bad">{{ w.run.agents.failed }} failed</span>
          }
          <span>of {{ w.run.agents.total }}</span>
          @if (w.run.startedAt && w.run.status === 'running') {
            <span><mc-time [ms]="w.run.startedAt" mode="elapsed" /></span>
          } @else if (w.run.durationMs) {
            <span>{{ duration(w.run.durationMs) }}</span>
          }
          @if (w.run.totalTokens) {
            <span>{{ tokens(w.run.totalTokens) }} tokens</span>
          }
        </p>
        @if (w.run.phases.length) {
          <ol class="phases">
            @for (p of w.run.phases; track $index) {
              <li [class.now]="w.current.has(p.title)" [class.past]="w.past.has(p.title)">
                <span class="pdot" aria-hidden="true"></span>
                <span>{{ p.title }}</span>
                @if (p.detail) {
                  <span class="muted small">{{ p.detail }}</span>
                }
                @if (w.current.has(p.title)) {
                  <span class="visually-hidden">(current phase)</span>
                }
              </li>
            }
          </ol>
        }
        <div class="list">
          @for (a of w.agents; track a.id) {
            <ng-container *ngTemplateOutlet="agentRow; context: { $implicit: a }" />
          }
        </div>
      </section>
    }

    @if (tree().length) {
      <section class="card" aria-label="Subagents">
        <h2 class="first">Agents</h2>
        <div class="list">
          @for (a of tree(); track a.id) {
            <ng-container *ngTemplateOutlet="agentRow; context: { $implicit: a }" />
          }
        </div>
      </section>
    }

    @if (tasks().length) {
      <section class="card" aria-label="Background tasks">
        <h2 class="first">Background tasks</h2>
        <div class="list">
          @for (t of tasks(); track t.id) {
            <div class="task">
              <mc-icon [name]="t.kind === 'monitor' ? 'activity' : 'terminal'" [size]="16" />
              <div class="tbody">
                <span class="tlabel mono">{{ t.label }}</span>
                <span class="tmeta">
                  <mc-chip [tone]="tone(t.status)">{{ t.status }}</mc-chip>
                  @if (t.startedAt) {
                    <mc-time [ms]="t.startedAt" [until]="t.endedAt" mode="elapsed" />
                  }
                </span>
              </div>
              @if (t.outputAvailable) {
                <button class="btn small" type="button" (click)="output.set(t)">Output</button>
              }
              @if (t.status === 'running' && ctx.caps()?.stopTask) {
                <button
                  class="btn small danger"
                  type="button"
                  [attr.aria-label]="'Stop ' + t.label"
                  (click)="ctx.stopTask(t.id, t.label)"
                >
                  <mc-icon name="stop" [size]="13" /> Stop
                </button>
              }
            </div>
          }
        </div>
      </section>
    }

    <mc-task-output [task]="output()" (closed)="output.set(null)" />

    <ng-template #agentRow let-a>
      <div class="agent" [style.--level]="a.level ?? 0">
        <a class="main" [routerLink]="['/session', ctx.id(), 'agent', a.id]">
          <span class="kind" [title]="kindLabel(a)" [attr.data-tone]="tone(a.status)"
            ><mc-icon [name]="kindIcon(a)" [size]="16"
          /></span>
          <span class="abody">
            <span class="atitle">
              <span class="alabel">{{ a.label }}</span>
              @if (a.type) {
                <mc-chip>{{ a.type }}</mc-chip>
              }
              @if (a.status !== 'running') {
                <mc-chip [tone]="tone(a.status)">{{ a.status }}</mc-chip>
              }
              @if (a.background) {
                <mc-chip tone="blue">background</mc-chip>
              }
            </span>
            @if (a.activity && a.status === 'running') {
              <mc-activity-line [activity]="a.activity" />
            }
            <span class="stats">
              @if (a.startedAt) {
                <span
                  ><mc-icon name="clock" [size]="12" />
                  <mc-time [ms]="a.startedAt" [until]="a.endedAt" mode="elapsed"
                /></span>
              }
              @if (a.tokens) {
                <span>{{ tokens(a.tokens) }} tokens</span>
              }
              @if (a.toolUses) {
                <span>{{ a.toolUses }} tool {{ a.toolUses === 1 ? 'use' : 'uses' }}</span>
              }
              @if (a.model) {
                <span>{{ a.model }}</span>
              }
            </span>
          </span>
        </a>
        @if (a.canStop && a.status === 'running') {
          <button
            class="btn danger small"
            type="button"
            [attr.aria-label]="'Stop ' + a.label"
            (click)="ctx.stopTask(a.taskId ?? a.id, a.label)"
          >
            <mc-icon name="stop" [size]="13" /> Stop
          </button>
        }
      </div>
    </ng-template>
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
        padding: 0.5rem 0 1rem;
      }
      h2.first {
        margin-top: 0;
      }
      .rhead {
        display: flex;
        align-items: center;
        gap: 0.5rem;
      }
      .rhead mc-icon {
        color: var(--accent-text);
      }
      .rname {
        flex: 1;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
      }
      .rstats {
        display: flex;
        flex-wrap: wrap;
        gap: 0.75rem;
        margin: 0.4rem 0;
        font-size: 0.8rem;
        color: var(--muted);
      }
      .rstats .bad {
        color: var(--red);
      }
      .phases {
        list-style: none;
        margin: 0.25rem 0 0.5rem;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
      }
      .phases li {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        font-size: 0.85rem;
        color: var(--muted);
      }
      .pdot {
        width: 0.6rem;
        height: 0.6rem;
        border-radius: 50%;
        border: 2px solid var(--border-strong);
        flex: 0 0 auto;
      }
      .phases li.past .pdot {
        background: var(--green);
        border-color: var(--green);
      }
      .phases li.now {
        color: var(--text);
        font-weight: 600;
      }
      .phases li.now .pdot {
        border-color: var(--accent-text);
        background: var(--accent-soft);
      }
      .list {
        display: flex;
        flex-direction: column;
      }
      .list > * + * {
        border-top: 1px solid var(--border);
      }
      .agent {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        padding: 0.6rem 0;
        padding-left: calc(min(var(--level, 0), 4) * 1.1rem);
      }
      .main {
        flex: 1;
        min-width: 0;
        display: flex;
        gap: 0.6rem;
        color: var(--text);
        text-decoration: none;
      }
      .kind {
        flex: 0 0 auto;
        width: 2rem;
        height: 2rem;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        border-radius: 0.5rem;
        background: var(--tone-soft);
        color: var(--tone);
      }
      .abody {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
      }
      .atitle {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.35rem;
      }
      .alabel {
        font-weight: 500;
        overflow-wrap: anywhere;
      }
      .stats {
        display: flex;
        flex-wrap: wrap;
        gap: 0.75rem;
        font-size: 0.75rem;
        color: var(--faint);
      }
      .stats span {
        display: inline-flex;
        align-items: center;
        gap: 0.25rem;
      }
      .task {
        display: flex;
        align-items: center;
        gap: 0.6rem;
        padding: 0.55rem 0;
      }
      .task > mc-icon {
        color: var(--muted);
      }
      .tbody {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.2rem;
      }
      .tlabel {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .tmeta {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        font-size: 0.75rem;
        color: var(--faint);
      }
    `,
  ],
})
export class AgentsPanelComponent {
  protected readonly ctx = inject(SessionContext);
  protected readonly output = signal<BackgroundTask | null>(null);

  /** Agents that belong to no workflow run, as a tree. */
  protected readonly tree = computed(() =>
    orderAgentTree(
      this.ctx.agents().filter((a) => !a.workflowRunId || !this.runIds().has(a.workflowRunId)),
    ),
  );
  private readonly runIds = computed(() => new Set(this.ctx.workflows().map((w) => w.runId)));
  protected readonly runs = computed(() =>
    this.ctx.workflows().map((run) => {
      const agents = orderAgentTree(this.ctx.agents().filter((a) => a.workflowRunId === run.runId));
      const current = new Set(
        agents.filter((a) => a.status === 'running' && a.phase).map((a) => a.phase as string),
      );
      const titles = run.phases.map((p) => p.title);
      const firstNow = titles.findIndex((t) => current.has(t));
      const past = new Set(
        run.status === 'completed' ? titles : firstNow > 0 ? titles.slice(0, firstNow) : [],
      );
      return { run, agents, current, past } satisfies {
        run: WorkflowRun;
        agents: (AgentInfo & { level: number })[];
        current: Set<string>;
        past: Set<string>;
      };
    }),
  );
  protected readonly tasks = computed(() =>
    [...this.ctx.tasks()].sort(
      (a, b) =>
        Number(b.status === 'running') - Number(a.status === 'running') ||
        (b.startedAt ?? 0) - (a.startedAt ?? 0),
    ),
  );

  protected readonly tokens = formatTokens;
  protected readonly duration = formatDuration;
  protected readonly kindIcon = agentKindIcon;
  protected readonly kindLabel = agentKindLabel;
  protected readonly tone = agentStatusTone;
}
