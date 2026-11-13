import { Component, DestroyRef, computed, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { NoticeService } from '../../core/notice.service';
import {
  AccountRef,
  AgentInfo,
  BackgroundTask,
  FleetSession,
  WorkflowRun,
} from '../../core/models';
import { RealtimeService } from '../../core/realtime.service';
import { ObservedEvent } from '../../core/teams';
import { AccountChipComponent } from '../../shared/account-chip.component';
import { ActivityLineComponent } from '../../shared/activity-line.component';
import {
  agentKindIcon,
  agentKindLabel,
  isTaskAgent,
  orderAgentTree,
  taskFromAgent,
} from '../../shared/agents';
import { ChipComponent } from '../../shared/chip.component';
import { EmptyStateComponent } from '../../shared/empty-state.component';
import { IconComponent } from '../../shared/icon.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';
import { ElapsedPipe } from '../../shared/time.pipes';
import { formatTokens, sessionName } from '../../shared/util';
import { TaskOutputComponent } from '../session-detail/task-output.component';
import { SessionsHeadComponent } from '../sessions/sessions-head.component';

/** While Limited (no socket, so no `fleet` frames), refresh every this many
 * poll ticks (2 s each). */
const FLEET_POLL_TICKS = 4;
/** What Claude Code's own agent teams reported, the latest first. */
const TEAMMATE_EVENTS = 20;

interface FleetGroup {
  sessionId: string;
  name: string;
  /** The session's Claude account (only while there are several). */
  account?: AccountRef;
  /** Subagents, workflow agents, teammates: each has a transcript. */
  agents: (AgentInfo & { level: number })[];
  workflows: WorkflowRun[];
  /** Background shells and monitors: output only, no transcript. */
  shells: AgentInfo[];
}

/**
 * *Sessions › Agents*, the fleet: every running agent (subagents, workflow
 * agents, background shells, monitors) grouped by session, with live
 * activity, elapsed time and tokens, plus each session's running workflow
 * runs. Tap an agent to open its transcript, or a shell/monitor for its
 * output; stop it where the session allows. Each session names its Claude
 * account while there are several. *Teammates* lists what Claude Code's own
 * agent teams reported through the hooks (interactive sessions: Mother
 * Claude observes them, it does not run them, docs/TEAMS.md "Hooks").
 */
@Component({
  selector: 'mc-agents',
  imports: [
    RouterLink,
    AccountChipComponent,
    ActivityLineComponent,
    ChipComponent,
    EmptyStateComponent,
    IconComponent,
    ElapsedPipe,
    SessionsHeadComponent,
    TaskOutputComponent,
    TimeAgoComponent,
    TPipe,
  ],
  template: `
    <div class="page narrow">
      <mc-sessions-head />

      @if (error()) {
        <p class="notice error">
          {{ error() }}
          <button class="btn ghost small" type="button" (click)="load()">{{ 'Retry' | t }}</button>
        </p>
      }

      @for (g of groups(); track g.sessionId) {
        <section class="group">
          <a class="ghead" [routerLink]="['/session', g.sessionId]">
            <span class="gname">{{ g.name }}</span>
            @if (g.account; as acc) {
              <mc-account-chip class="gacct" [account]="acc" />
            }
            <span class="muted small">{{ g.agents.length + g.shells.length }}</span>
            <mc-icon name="chevronRight" [size]="16" />
          </a>
          @for (w of g.workflows; track w.runId) {
            <div class="card wf">
              <mc-icon name="workflow" [size]="16" />
              <span class="wname">{{ w.name }}</span>
              <span class="muted small">{{ workflowCounts(w) }}</span>
            </div>
          }
          @for (t of g.shells; track t.id) {
            <div class="card wf shell">
              <button
                class="open"
                type="button"
                [attr.aria-label]="'Output of {name}' | t: { name: t.label }"
                (click)="openOutput(t)"
              >
                <mc-icon [name]="t.kind === 'monitor' ? 'activity' : 'terminal'" [size]="16" />
                <span class="wname mono">{{ t.label }}</span>
                @if (t.startedAt) {
                  <span class="muted small">{{ t.startedAt | elapsed }}</span>
                }
              </button>
              @if (t.canStop) {
                <button
                  class="btn danger small"
                  type="button"
                  [disabled]="stopping().has(t.id)"
                  [attr.aria-label]="'Stop {name}' | t: { name: t.label }"
                  (click)="stop(t)"
                >
                  <mc-icon name="stop" [size]="14" />
                  {{ stopping().has(t.id) ? ('Stopping…' | t) : ('Stop' | t) }}
                </button>
              }
            </div>
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
                        <mc-chip tone="blue">{{ 'background' | t }}</mc-chip>
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
                        <span>{{ '{n} tokens' | t: { n: tokens(a.tokens) } }}</span>
                      }
                      @if (a.toolUses) {
                        <span>{{
                          a.toolUses === 1
                            ? ('{n} tool use' | t: { n: a.toolUses })
                            : ('{n} tool uses' | t: { n: a.toolUses })
                        }}</span>
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
                    [attr.aria-label]="'Stop {name}' | t: { name: a.label }"
                    (click)="stop(a)"
                  >
                    <mc-icon name="stop" [size]="14" />
                    {{ stopping().has(a.id) ? ('Stopping…' | t) : ('Stop' | t) }}
                  </button>
                }
              </div>
            }
          </div>
        </section>
      } @empty {
        @if (loaded()) {
          <mc-empty-state
            icon="agents"
            [heading]="'No agents running' | t"
            [text]="
              'Subagents, workflow agents and background shells show up here while they work.' | t
            "
            help="sessions"
          />
        } @else {
          <div class="skeleton" [attr.aria-label]="'Loading' | t"><span></span><span></span></div>
        }
      }

      @if (teammates().length) {
        <section class="teammates" [attr.aria-label]="'Teammates' | t">
          <h2>{{ 'Teammates' | t }}</h2>
          <p class="muted small">
            {{
              "What Claude Code's own agent teams reported through the hooks (interactive sessions; Mother Claude observes them, it does not run them)."
                | t
            }}
          </p>
          <div class="card list">
            @for (e of teammates(); track $index) {
              <div class="list-row event">
                <span class="badge">{{ e.event }}</span>
                <span class="grow small"
                  >{{ e.teammate || e.toModel || e.source || '' }}
                  @if (e.subject) {
                    · {{ e.subject }}
                  }
                </span>
                <mc-time class="faint small" [ms]="e.at" />
              </div>
            }
          </div>
        </section>
      }
    </div>

    <mc-task-output
      [sessionId]="output()?.sessionId ?? ''"
      [task]="output()?.task ?? null"
      (closed)="output.set(null)"
    />
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
      .shell {
        padding-top: 0.25rem;
        padding-bottom: 0.25rem;
      }
      .open {
        flex: 1;
        min-width: 0;
        min-height: var(--tap);
        display: flex;
        align-items: center;
        gap: 0.5rem;
        padding: 0;
        border: 0;
        background: none;
        color: inherit;
        font: inherit;
        text-align: left;
        cursor: pointer;
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
      .gacct {
        flex: 0 1 auto;
        max-width: 45%;
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
      .teammates {
        margin-top: 1.5rem;
      }
      .teammates .list {
        padding-top: 0;
        padding-bottom: 0;
      }
      .event {
        flex-wrap: wrap;
        gap: 0.4rem 0.75rem;
      }
      .event .grow {
        flex: 1 1 16rem;
        min-width: 0;
        overflow-wrap: anywhere;
      }
    `,
  ],
})
export class AgentsComponent {
  private realtime = inject(RealtimeService);
  private api = inject(ApiService);
  private notices = inject(NoticeService);

  private readonly fleet = signal<FleetSession[]>([]);
  /** Claude Code's own agent teams, as the hooks reported them. */
  protected readonly teammates = signal<ObservedEvent[]>([]);
  protected readonly loaded = signal(false);
  protected readonly error = signal('');
  protected readonly stopping = signal<ReadonlySet<string>>(new Set());
  /** The shell whose output sheet is open. */
  protected readonly output = signal<{ sessionId: string; task: BackgroundTask } | null>(null);
  private loading = false;
  private tick = 0;

  protected readonly groups = computed<FleetGroup[]>(() => {
    const byId = new Map(this.realtime.sessions().map((s) => [s.id, s]));
    return this.fleet()
      .map((f) => {
        const s = byId.get(f.sessionId);
        const all = f.agents ?? [];
        const account = f.account ?? s?.account;
        return {
          sessionId: f.sessionId,
          name: s ? sessionName(s) : f.name || f.sessionId.slice(0, 8),
          ...(account ? { account } : {}),
          agents: orderAgentTree(all.filter((a) => !isTaskAgent(a))),
          workflows: (f.workflows ?? []).filter((w) => w.status === 'running'),
          shells: all.filter((a) => isTaskAgent(a) && a.status === 'running'),
        };
      })
      .filter((g) => g.agents.length || g.workflows.length || g.shells.length);
  });
  protected readonly tokens = formatTokens;

  constructor() {
    const destroyRef = inject(DestroyRef);
    const release = this.realtime.subscribeFleet();
    const subs = [
      this.realtime.fleet$.subscribe((f) => {
        this.fleet.set(f.sessions ?? []);
        this.loaded.set(true);
      }),
      this.realtime.refetch$.subscribe(() => {
        void this.load();
        void this.loadTeammates();
      }),
      // Limited (polling): no fleet frames arrive, so refresh now and then.
      this.realtime.pollTick$.subscribe(() => {
        this.tick += 1;
        if (this.tick % FLEET_POLL_TICKS === 0) void this.load();
      }),
    ];
    destroyRef.onDestroy(() => {
      release();
      subs.forEach((s) => s.unsubscribe());
    });
    void this.load();
    void this.loadTeammates();
  }

  protected readonly kindIcon = agentKindIcon;
  protected readonly kindLabel = agentKindLabel;

  /** "2 running · 5 done · 1 failed". */
  protected workflowCounts(w: WorkflowRun): string {
    return [
      tr('{n} running', { n: w.agents.running }),
      tr('{n} done', { n: w.agents.done }),
      w.agents.failed ? tr('{n} failed', { n: w.agents.failed }) : '',
    ]
      .filter(Boolean)
      .join(' · ');
  }

  protected async stop(a: AgentInfo): Promise<void> {
    this.stopping.update((prev) => new Set(prev).add(a.id));
    try {
      await this.api.stopTask(a.sessionId, a.taskId ?? a.id);
      this.notices.success(tr('Asked “{name}” to stop.', { name: a.label }));
    } catch (e) {
      this.notices.error(e, undefined, tr('Could not stop'));
    } finally {
      this.stopping.update((prev) => {
        const next = new Set(prev);
        next.delete(a.id);
        return next;
      });
    }
  }

  /** A shell or monitor has no transcript: show its output instead. */
  protected openOutput(a: AgentInfo): void {
    this.output.set({ sessionId: a.sessionId, task: taskFromAgent(a) });
  }

  protected async load(): Promise<void> {
    if (this.loading) return; // a slow poll must not pile up requests
    this.loading = true;
    try {
      const fleet = await this.api.getFleet();
      this.fleet.set(fleet.sessions ?? []);
      this.error.set('');
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.loading = false;
      this.loaded.set(true);
    }
  }

  /** What agent teams in interactive sessions reported (an older server
   * has none to say). */
  private async loadTeammates(): Promise<void> {
    try {
      const { events } = await this.api.getTeamEvents();
      this.teammates.set((events ?? []).slice(0, TEAMMATE_EVENTS));
    } catch {
      // Only a convenience: the fleet above is the page.
    }
  }
}
