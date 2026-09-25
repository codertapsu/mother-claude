import { Component, DestroyRef, computed, effect, inject, signal, untracked } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { ActivatedRoute, RouterLink } from '@angular/router';
import { map } from 'rxjs';

import { ApiService, errorMessage } from '../../core/api.service';
import { AgentInfo } from '../../core/models';
import { RealtimeService } from '../../core/realtime.service';
import { TranscriptStore } from '../../core/transcript-store';
import { ActivityLineComponent } from '../../shared/activity-line.component';
import { ChipComponent } from '../../shared/chip.component';
import { EventListComponent } from '../../shared/event-list.component';
import { IconComponent } from '../../shared/icon.component';
import { ElapsedPipe } from '../../shared/time.pipes';
import { formatTokens, sessionName } from '../../shared/util';

/**
 * One agent (subagent, workflow agent, background shell or monitor): what it
 * is, what it's doing now, and its own transcript. Route
 * `session/:id/agent/:agentId`. Running agents refresh every few seconds.
 */
@Component({
  selector: 'mc-agent-view',
  imports: [
    RouterLink,
    ActivityLineComponent,
    ChipComponent,
    EventListComponent,
    IconComponent,
    ElapsedPipe,
  ],
  template: `
    <div class="page narrow">
      <a class="back" [routerLink]="['/session', sessionId()]">
        <mc-icon name="back" [size]="16" /> {{ sessionLabel() }}
      </a>

      @if (agent(); as a) {
        <header class="card head">
          <h1>{{ a.label }}</h1>
          <div class="chips">
            <mc-chip
              [tone]="a.status === 'running' ? 'green' : a.status === 'failed' ? 'red' : 'muted'"
              >{{ a.status }}</mc-chip
            >
            @if (a.type) {
              <mc-chip>{{ a.type }}</mc-chip>
            }
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
              <mc-chip icon="clock">{{ a.startedAt | elapsed: a.endedAt }}</mc-chip>
            }
            @if (a.tokens) {
              <mc-chip>{{ tokens(a.tokens) }} tokens</mc-chip>
            }
            @if (a.toolUses) {
              <mc-chip>{{ a.toolUses }} tool uses</mc-chip>
            }
          </div>
          @if (a.activity && a.status === 'running') {
            <mc-activity-line [activity]="a.activity" />
          }
          @if (a.worktree) {
            <p class="muted small mono">worktree: {{ a.worktree }}</p>
          }
          @if (a.canStop && a.status === 'running') {
            <div class="row">
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
        </header>
      } @else if (loadError()) {
        <p class="notice error">{{ loadError() }}</p>
      }

      @if (notice()) {
        <p class="notice" role="status">{{ notice() }}</p>
      }

      <section class="card log">
        @if (store(); as t) {
          @if (t.hasEarlier()) {
            <button
              class="btn small ghost"
              type="button"
              [disabled]="t.loadingEarlier()"
              (click)="t.loadEarlier()"
            >
              {{ t.loadingEarlier() ? 'Loading…' : 'Load earlier' }}
            </button>
          }
          @if (t.loading()) {
            <p class="muted">Loading transcript…</p>
          } @else {
            <mc-event-list [events]="t.events()" />
          }
          @if (t.error()) {
            <p class="error-text small">{{ t.error() }}</p>
          }
        }
      </section>
    </div>
  `,
  styles: [
    `
      .back {
        display: inline-flex;
        align-items: center;
        gap: 0.3rem;
        min-height: var(--tap);
        color: var(--muted);
        text-decoration: none;
        margin-bottom: 0.25rem;
      }
      .head {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
        margin-bottom: 0.75rem;
      }
      .head h1 {
        margin: 0;
        font-size: 1.15rem;
      }
      .chips {
        display: flex;
        flex-wrap: wrap;
        gap: 0.3rem;
      }
      .log {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
      }
    `,
  ],
})
export class AgentViewComponent {
  private route = inject(ActivatedRoute);
  private api = inject(ApiService);
  private realtime = inject(RealtimeService);
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
  protected readonly store = signal<TranscriptStore | null>(null);
  protected readonly loadError = signal('');
  protected readonly notice = signal('');
  protected readonly stopping = signal(false);
  protected readonly sessionLabel = computed(() => {
    const s = this.realtime.sessions().find((x) => x.id === this.sessionId());
    return s ? sessionName(s) : 'Session';
  });
  protected readonly tokens = formatTokens;

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

    // (Re)open the transcript when the route changes.
    effect(() => {
      const id = this.sessionId();
      const agentId = this.agentId();
      untracked(() => {
        this.store()?.destroy();
        this.store.set(id && agentId ? this.realtime.openTranscript(id, { agentId }) : null);
        void this.loadAgents();
      });
    });

    // Only running agents change; stop polling once this one is done.
    effect(() => {
      const running = this.agent()?.status === 'running';
      const store = this.store();
      untracked(() => store?.setPoll(running ? 4000 : 0));
    });
  }

  protected async stop(a: AgentInfo): Promise<void> {
    this.stopping.set(true);
    try {
      await this.api.stopTask(a.sessionId, a.taskId ?? a.id);
      this.notice.set('Stop requested.');
    } catch (e) {
      this.notice.set(`Could not stop: ${errorMessage(e)}`);
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
      this.loadError.set(errorMessage(e));
    }
  }
}
