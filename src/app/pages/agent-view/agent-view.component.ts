import { Component, DestroyRef, computed, effect, inject, signal, untracked } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { ActivatedRoute, RouterLink } from '@angular/router';
import { map } from 'rxjs';

import { ApiService, describeError } from '../../core/api.service';
import { AgentInfo, UiEvent } from '../../core/models';
import { NoticeService } from '../../core/notice.service';
import { RealtimeService } from '../../core/realtime.service';
import { TranscriptStore } from '../../core/transcript-store';
import { ActivityLineComponent } from '../../shared/activity-line.component';
import { agentKindIcon, agentKindLabel, agentStatusTone } from '../../shared/agents';
import { ChipComponent } from '../../shared/chip.component';
import { IconComponent } from '../../shared/icon.component';
import { renderMarkdown } from '../../shared/markdown';
import { TimeAgoComponent } from '../../shared/time-ago.component';
import { formatTokens, sessionName } from '../../shared/util';
import { TranscriptComponent } from '../session-detail/transcript.component';

/** Running agents' transcripts also refresh this often, in case the live
 * `sub{agent}` stream isn't available. */
const SAFETY_POLL_MS = 15_000;

/**
 * One agent — a subagent, workflow agent, background shell or monitor —
 * with its own transcript, live while it runs (`sub{session, agent}`).
 * Header: type, label, status, elapsed, tokens and tool uses, a breadcrumb up
 * to the parent session (and parent agent), and Stop where allowed. When the
 * agent is done, the footer shows its result as the parent received it (the
 * tool result, or the task notification).
 */
@Component({
  selector: 'mc-agent-view',
  imports: [
    RouterLink,
    ActivityLineComponent,
    ChipComponent,
    IconComponent,
    TimeAgoComponent,
    TranscriptComponent,
  ],
  template: `
    <div class="av">
      <header class="bar">
        <a
          class="btn ghost icon-only"
          [routerLink]="parentLink()"
          [attr.aria-label]="'Back to ' + parentLabel()"
        >
          <mc-icon name="back" />
        </a>
        <nav class="crumbs" aria-label="Breadcrumb">
          <a [routerLink]="['/session', sessionId()]">{{ sessionLabel() }}</a>
          @if (parent(); as p) {
            <span aria-hidden="true">›</span>
            <a [routerLink]="['/session', sessionId(), 'agent', p.id]">{{ p.label }}</a>
          }
          <span aria-hidden="true">›</span>
          <span class="here" aria-current="page">{{ agent()?.label || 'Agent' }}</span>
        </nav>
      </header>

      <div #scroller class="scroll">
        <div class="inner">
          @if (agent(); as a) {
            <section class="card head">
              <div class="title">
                <span class="kind" [attr.data-tone]="tone(a.status)"
                  ><mc-icon [name]="kindIcon(a)" [size]="18"
                /></span>
                <h1>{{ a.label }}</h1>
              </div>
              <div class="chips">
                <mc-chip [tone]="tone(a.status)">{{ a.status }}</mc-chip>
                <mc-chip>{{ a.type || kindLabel(a) }}</mc-chip>
                @if (a.phase) {
                  <mc-chip tone="accent">{{ a.phase }}</mc-chip>
                }
                @if (a.background) {
                  <mc-chip tone="blue">background</mc-chip>
                }
                @if (a.model) {
                  <mc-chip>{{ a.model }}</mc-chip>
                }
                @if (a.startedAt) {
                  <mc-chip icon="clock"
                    ><mc-time [ms]="a.startedAt" [until]="a.endedAt" mode="elapsed"
                  /></mc-chip>
                }
                @if (a.tokens) {
                  <mc-chip>{{ tokens(a.tokens) }} tokens</mc-chip>
                }
                @if (a.toolUses) {
                  <mc-chip>{{ a.toolUses }} tool {{ a.toolUses === 1 ? 'use' : 'uses' }}</mc-chip>
                }
              </div>
              @if (a.activity && a.status === 'running') {
                <mc-activity-line [activity]="a.activity" />
              }
              @if (a.worktree) {
                <p class="muted small mono">worktree: {{ a.worktree }}</p>
              }
              @if (a.canStop && a.status === 'running') {
                <div>
                  <button
                    class="btn danger small"
                    type="button"
                    [disabled]="stopping()"
                    (click)="stop(a)"
                  >
                    <mc-icon name="stop" [size]="14" />
                    {{ stopping() ? 'Stopping…' : 'Stop this agent' }}
                  </button>
                </div>
              }
            </section>
          } @else if (loadError()) {
            <p class="notice error">{{ loadError() }}</p>
          }

          <mc-transcript
            [scroller]="scroller"
            [store]="store()"
            [sessionId]="sessionId()"
            [running]="agent()?.status === 'running'"
          />

          @if (agent() && agent()?.status !== 'running') {
            <section class="card result" aria-label="Result">
              <p class="rtitle">
                <mc-icon [name]="agent()?.status === 'failed' ? 'xCircle' : 'check'" [size]="16" />
                Agent {{ agent()?.status === 'failed' ? 'failed' : 'finished' }}
                @if (agent()?.endedAt) {
                  · <mc-time [ms]="agent()?.endedAt" />
                }
              </p>
              @if (resultHtml()) {
                <div class="md" [innerHTML]="resultHtml()"></div>
              } @else if (resultLoading()) {
                <p class="muted small">Looking for the result…</p>
              } @else {
                <p class="muted small">The result isn't in the session's transcript yet.</p>
              }
            </section>
          }
        </div>
      </div>
    </div>
  `,
  styles: [
    `
      .av {
        display: flex;
        flex-direction: column;
        height: 100dvh;
        background: var(--bg);
      }
      .bar {
        flex: 0 0 auto;
        display: flex;
        align-items: center;
        gap: 0.35rem;
        min-height: 48px;
        padding: 0 0.5rem;
        background: var(--panel);
        border-bottom: 1px solid var(--border);
      }
      .crumbs {
        flex: 1;
        min-width: 0;
        display: flex;
        align-items: center;
        gap: 0.35rem;
        font-size: 0.85rem;
        color: var(--faint);
        overflow: hidden;
        white-space: nowrap;
      }
      .crumbs a {
        color: var(--muted);
        text-decoration: none;
        overflow: hidden;
        text-overflow: ellipsis;
        max-width: 40%;
      }
      .here {
        color: var(--text);
        font-weight: 600;
        overflow: hidden;
        text-overflow: ellipsis;
      }
      .scroll {
        flex: 1;
        min-height: 0;
        overflow-y: auto;
        overscroll-behavior: contain;
        padding: 0.75rem max(0.85rem, env(safe-area-inset-right))
          max(1rem, env(safe-area-inset-bottom)) max(0.85rem, env(safe-area-inset-left));
      }
      .inner {
        max-width: 960px;
        margin: 0 auto;
        display: flex;
        flex-direction: column;
      }
      .head {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
      }
      .title {
        display: flex;
        align-items: center;
        gap: 0.6rem;
      }
      .title h1 {
        margin: 0;
        font-size: 1.1rem;
        min-width: 0;
        overflow-wrap: anywhere;
      }
      .kind {
        flex: 0 0 auto;
        width: 2.2rem;
        height: 2.2rem;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        border-radius: 0.55rem;
        background: var(--tone-soft);
        color: var(--tone);
      }
      .chips {
        display: flex;
        flex-wrap: wrap;
        gap: 0.3rem;
      }
      .result {
        margin-top: 0.5rem;
        border-color: var(--green-border);
      }
      .rtitle {
        display: flex;
        align-items: center;
        gap: 0.4rem;
        margin: 0 0 0.4rem;
        font-weight: 600;
      }
      @media (max-width: 767px) {
        .av {
          position: fixed;
          inset: 0;
          top: var(--vv-top, 0px);
          height: var(--vvh, 100dvh);
          z-index: 25;
          padding-top: env(safe-area-inset-top);
        }
      }
    `,
  ],
})
export class AgentViewComponent {
  private route = inject(ActivatedRoute);
  private api = inject(ApiService);
  private realtime = inject(RealtimeService);
  private notices = inject(NoticeService);
  private destroyRef = inject(DestroyRef);

  protected readonly sessionId = toSignal(this.route.paramMap.pipe(map((p) => p.get('id') ?? '')), {
    initialValue: this.route.snapshot.paramMap.get('id') ?? '',
  });
  protected readonly agentId = toSignal(
    this.route.paramMap.pipe(map((p) => p.get('agentId') ?? '')),
    { initialValue: this.route.snapshot.paramMap.get('agentId') ?? '' },
  );

  private readonly agents = signal<AgentInfo[]>([]);
  protected readonly agent = computed(() => this.agents().find((a) => a.id === this.agentId()));
  protected readonly parent = computed(() => {
    const pid = this.agent()?.parentId;
    return pid ? this.agents().find((a) => a.id === pid) : undefined;
  });
  protected readonly store = signal<TranscriptStore | null>(null);
  protected readonly loadError = signal('');
  protected readonly stopping = signal(false);
  protected readonly resultText = signal('');
  protected readonly resultLoading = signal(false);
  protected readonly resultHtml = computed(() =>
    this.resultText() ? renderMarkdown(this.resultText()) : '',
  );
  protected readonly sessionLabel = computed(() => {
    const s = this.realtime.sessions().find((x) => x.id === this.sessionId());
    return s ? sessionName(s) : 'Session';
  });
  protected readonly parentLink = computed(() => {
    const p = this.parent();
    return p ? ['/session', this.sessionId(), 'agent', p.id] : ['/session', this.sessionId()];
  });
  protected readonly parentLabel = computed(() => this.parent()?.label ?? this.sessionLabel());
  protected readonly tokens = formatTokens;
  protected readonly kindIcon = agentKindIcon;
  protected readonly kindLabel = agentKindLabel;
  protected readonly tone = agentStatusTone;
  private resultFor = '';

  constructor() {
    const subs = [
      this.realtime.agents$.subscribe((f) => {
        if (f.session === this.sessionId()) this.agents.set(f.agents ?? []);
      }),
      this.realtime.refetch$.subscribe(() => void this.loadAgents()),
    ];
    this.destroyRef.onDestroy(() => {
      subs.forEach((s) => s.unsubscribe());
      this.store()?.destroy();
    });

    // (Re)open the agent's live transcript when the route changes.
    effect(() => {
      const id = this.sessionId();
      const agentId = this.agentId();
      untracked(() => {
        this.store()?.destroy();
        this.resultText.set('');
        this.resultFor = '';
        this.store.set(id && agentId ? this.realtime.openTranscript(id, { agentId }) : null);
        void this.loadAgents();
      });
    });

    // A safety refresh while it runs; the result once it's done.
    effect(() => {
      const a = this.agent();
      const store = this.store();
      untracked(() => {
        store?.setPoll(a?.status === 'running' ? SAFETY_POLL_MS : 0);
        if (a && a.status !== 'running' && this.resultFor !== a.id) void this.loadResult(a);
      });
    });
  }

  protected async stop(a: AgentInfo): Promise<void> {
    this.stopping.set(true);
    try {
      await this.api.stopTask(a.sessionId || this.sessionId(), a.taskId ?? a.id);
      this.notices.success(`Asked “${a.label}” to stop.`);
    } catch (e) {
      this.notices.error(e, undefined, 'Could not stop');
    } finally {
      this.stopping.set(false);
    }
  }

  private async loadAgents(): Promise<void> {
    const id = this.sessionId();
    if (!id) return;
    try {
      const res = await this.api.getSessionAgents(id);
      if (id !== this.sessionId()) return;
      this.agents.set(res.agents ?? []);
      this.loadError.set(this.agent() ? '' : 'This agent is no longer listed for the session.');
    } catch (e) {
      this.loadError.set(describeError(e).text);
    }
  }

  /** The parent's view of the result: the Agent call's tool_result, or the
   * background task's notification. */
  private async loadResult(a: AgentInfo): Promise<void> {
    this.resultFor = a.id;
    this.resultLoading.set(true);
    try {
      const page = await this.api.getTranscript(this.sessionId(), { limit: 300 });
      const events = page.events ?? [];
      const fromResult = a.toolUseId
        ? events.find((e: UiEvent) => e.kind === 'result' && e.result?.toolUseId === a.toolUseId)
        : undefined;
      const fromTask = events.find(
        (e: UiEvent) =>
          e.kind === 'task' && (e.task?.taskId === a.taskId || e.task?.taskId === a.id),
      );
      this.resultText.set(
        fromResult?.result?.text ?? fromTask?.task?.summary ?? fromTask?.text ?? '',
      );
    } catch {
      this.resultText.set('');
    } finally {
      this.resultLoading.set(false);
    }
  }
}
