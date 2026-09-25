import { Component, DestroyRef, computed, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import { NoticeService } from '../../core/notice.service';
import { AgentInfo, BackgroundTask, FleetSession, WorkflowRun } from '../../core/models';
import { RealtimeService } from '../../core/realtime.service';
import { ActivityLineComponent } from '../../shared/activity-line.component';
import { agentKindIcon, agentKindLabel, orderAgentTree } from '../../shared/agents';
import { ChipComponent } from '../../shared/chip.component';
import { IconComponent } from '../../shared/icon.component';
import { ElapsedPipe } from '../../shared/time.pipes';
import { formatTokens, sessionName } from '../../shared/util';

interface FleetGroup {
  sessionId: string;
  name: string;
  agents: (AgentInfo & { level: number })[];
  workflows: WorkflowRun[];
  tasks: BackgroundTask[];
}

/**
 * The fleet: every running agent (subagents, workflow agents, background
 * shells, monitors) grouped by session, with live activity, elapsed time and
 * tokens, plus each session's running workflow runs and background tasks.
 * Tap one to open its transcript; stop it where the session allows.
 */
@Component({
  selector: 'mc-agents',
  imports: [RouterLink, ActivityLineComponent, ChipComponent, IconComponent, ElapsedPipe],
  template: `
    <div class="page">
      <div class="page-head">
        <h1>Agents</h1>
        @if (total()) {
          <span class="muted">{{ total() }} running</span>
        }
      </div>

      @if (error()) {
        <p class="notice error">
          {{ error() }}
          <button class="btn ghost small" type="button" (click)="load()">Retry</button>
        </p>
      }

      @for (g of groups(); track g.sessionId) {
        <section class="group">
          <a class="ghead" [routerLink]="['/session', g.sessionId]">
            <span class="gname">{{ g.name }}</span>
            <span class="muted small">{{ g.agents.length }}</span>
            <mc-icon name="chevronRight" [size]="16" />
          </a>
          @for (w of g.workflows; track w.runId) {
            <div class="card wf">
              <mc-icon name="workflow" [size]="16" />
              <span class="wname">{{ w.name }}</span>
              <span class="muted small"
                >{{ w.agents.running }} running · {{ w.agents.done }} done{{
                  w.agents.failed ? ' · ' + w.agents.failed + ' failed' : ''
                }}</span
              >
            </div>
          }
          @for (t of g.tasks; track t.id) {
            <a
              class="card wf"
              [routerLink]="['/session', g.sessionId]"
              [queryParams]="{ tab: 'agents' }"
            >
              <mc-icon [name]="t.kind === 'monitor' ? 'activity' : 'terminal'" [size]="16" />
              <span class="wname mono">{{ t.label }}</span>
              @if (t.startedAt) {
                <span class="muted small">{{ t.startedAt | elapsed }}</span>
              }
            </a>
          }
          <div class="list card" [hidden]="!g.agents.length">
            @for (a of g.agents; track a.id) {
              <div class="agent" [style.--depth]="a.level">
                <a class="main" [routerLink]="['/session', a.sessionId, 'agent', a.id]">
                  <span class="kind" [title]="kindLabel(a)"
                    ><mc-icon [name]="kindIcon(a)" [size]="16"
                  /></span>
                  <span class="body">
                    <span class="title">
                      <span class="label">{{ a.label }}</span>
                      @if (a.type) {
                        <mc-chip>{{ a.type }}</mc-chip>
                      }
                      @if (a.phase) {
                        <mc-chip tone="accent">{{ a.phase }}</mc-chip>
                      }
                      @if (a.background) {
                        <mc-chip tone="blue">background</mc-chip>
                      }
                    </span>
                    @if (a.activity) {
                      <mc-activity-line [activity]="a.activity" />
                    }
                    <span class="stats">
                      @if (a.startedAt) {
                        <span
                          ><mc-icon name="clock" [size]="12" /> {{ a.startedAt | elapsed }}</span
                        >
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
                @if (a.canStop) {
                  <button
                    class="btn danger small"
                    type="button"
                    [disabled]="stopping().has(a.id)"
                    [attr.aria-label]="'Stop ' + a.label"
                    (click)="stop(a)"
                  >
                    <mc-icon name="stop" [size]="14" />
                    {{ stopping().has(a.id) ? 'Stopping…' : 'Stop' }}
                  </button>
                }
              </div>
            }
          </div>
        </section>
      } @empty {
        @if (loaded()) {
          <div class="card empty-state">
            <strong>No agents running</strong>
            Subagents, workflow agents and background shells show up here while they work.
          </div>
        } @else {
          <div class="skeleton" aria-label="Loading"><span></span><span></span></div>
        }
      }
    </div>
  `,
  styles: [
    `
      .group {
        margin-top: 1rem;
      }
      .wf {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        margin-bottom: 0.4rem;
        color: var(--text);
        text-decoration: none;
      }
      .wf mc-icon {
        color: var(--accent-text);
      }
      .wname {
        flex: 1;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        font-weight: 500;
      }
      .skeleton {
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
        margin-top: 1rem;
      }
      .skeleton span {
        height: 6rem;
        border-radius: var(--radius);
        background: var(--panel-2);
        animation: mc-shimmer 1.2s ease-in-out infinite alternate;
      }
      .ghead {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        min-height: var(--tap);
        color: var(--text);
        text-decoration: none;
      }
      .gname {
        font-weight: 600;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        min-width: 0;
      }
      .ghead mc-icon {
        margin-left: auto;
        color: var(--faint);
      }
      .list {
        padding: 0 0.9rem;
      }
      .agent {
        display: flex;
        align-items: center;
        gap: 0.6rem;
        padding: 0.65rem 0;
        padding-left: calc(min(var(--depth, 0), 4) * 1rem);
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
        background: var(--green-soft);
        color: var(--green);
      }
      .body {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
      }
      .title {
        display: flex;
        align-items: center;
        gap: 0.35rem;
        flex-wrap: wrap;
        min-width: 0;
      }
      .label {
        font-weight: 500;
        overflow: hidden;
        text-overflow: ellipsis;
        min-width: 0;
      }
      .stats {
        display: flex;
        flex-wrap: wrap;
        gap: 0.75rem;
        font-size: 0.75rem;
        color: var(--faint);
        font-variant-numeric: tabular-nums;
      }
      .stats span {
        display: inline-flex;
        align-items: center;
        gap: 0.25rem;
      }
    `,
  ],
})
export class AgentsComponent {
  private realtime = inject(RealtimeService);
  private api = inject(ApiService);
  private notices = inject(NoticeService);

  private readonly fleet = signal<FleetSession[]>([]);
  protected readonly loaded = signal(false);
  protected readonly error = signal('');
  protected readonly stopping = signal<ReadonlySet<string>>(new Set());

  protected readonly groups = computed<FleetGroup[]>(() => {
    const byId = new Map(this.realtime.sessions().map((s) => [s.id, s]));
    return this.fleet()
      .map((f) => {
        const s = byId.get(f.sessionId);
        return {
          sessionId: f.sessionId,
          name: s ? sessionName(s) : f.name || f.sessionId.slice(0, 8),
          agents: orderAgentTree(f.agents ?? []),
          workflows: (f.workflows ?? []).filter((w) => w.status === 'running'),
          tasks: (f.tasks ?? []).filter((t) => t.status === 'running'),
        };
      })
      .filter((g) => g.agents.length || g.workflows.length || g.tasks.length);
  });
  protected readonly total = computed(() =>
    this.groups().reduce((n, g) => n + g.agents.length + g.tasks.length, 0),
  );

  protected readonly tokens = formatTokens;

  constructor() {
    const destroyRef = inject(DestroyRef);
    const release = this.realtime.subscribeFleet();
    const subs = [
      this.realtime.fleet$.subscribe((f) => {
        this.fleet.set(f.sessions ?? []);
        this.loaded.set(true);
      }),
      this.realtime.refetch$.subscribe(() => void this.load()),
    ];
    destroyRef.onDestroy(() => {
      release();
      subs.forEach((s) => s.unsubscribe());
    });
    void this.load();
  }

  protected readonly kindIcon = agentKindIcon;
  protected readonly kindLabel = agentKindLabel;

  protected async stop(a: AgentInfo): Promise<void> {
    this.stopping.update((prev) => new Set(prev).add(a.id));
    try {
      await this.api.stopTask(a.sessionId, a.taskId ?? a.id);
      this.notices.success(`Asked “${a.label}” to stop.`);
    } catch (e) {
      this.notices.error(e, undefined, 'Could not stop');
    } finally {
      this.stopping.update((prev) => {
        const next = new Set(prev);
        next.delete(a.id);
        return next;
      });
    }
  }

  protected async load(): Promise<void> {
    try {
      const fleet = await this.api.getFleet();
      this.fleet.set(fleet.sessions ?? []);
      this.error.set('');
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.loaded.set(true);
    }
  }
}
