import { Component, computed, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiService } from '../../core/api.service';
import { RealtimeService } from '../../core/realtime.service';
import { BackgroundTask, Session } from '../../core/models';
import { relativeTime } from '../../shared/util';

interface ServicesPayload {
  mcpServers?: Record<string, unknown>;
  daemon?: { reachable?: boolean; raw?: string; error?: string };
  backgroundJobs?: { id: string; cwd: string; state: string }[];
}

interface SessionActivity {
  session: Session;
  running: BackgroundTask[];
  finished: BackgroundTask[];
}

const KIND_ICONS: Record<string, string> = {
  bash: '⚙',
  agent: '🤖',
  workflow: '🔀',
};

@Component({
  selector: 'mc-services',
  imports: [RouterLink],
  template: `
    <h1>Services</h1>
    @if (error()) {
      <div class="card muted">{{ error() }}</div>
    }

    <h2>Background activity</h2>
    @if (activity().length) {
      @for (a of activity(); track a.session.id) {
        <div class="card act">
          <a class="sess" [routerLink]="['/session', a.session.id]">
            <strong>{{ a.session.title || a.session.projectName || a.session.id }}</strong>
            <span class="muted mono small">{{ a.session.cwd }}</span>
          </a>
          @for (t of a.running; track t.id) {
            <div class="task">
              <span class="tico">{{ icon(t.kind) }}</span>
              <span class="tlabel">{{ t.label || t.id }}</span>
              <span class="spacer"></span>
              <span class="badge working">running</span>
              <span class="muted small">{{ rel(t.startedAt) }}</span>
            </div>
          }
          @for (t of a.finished; track t.id) {
            <div class="task done">
              <span class="tico">{{ icon(t.kind) }}</span>
              <span class="tlabel">{{ t.label || t.id }}</span>
              <span class="spacer"></span>
              <span
                class="badge"
                [class.completed]="t.status === 'completed'"
                [class.failed]="t.status === 'failed' || t.status === 'killed'"
                >{{ t.status }}</span
              >
              <span class="muted small">{{ rel(t.endedAt) }}</span>
            </div>
          }
        </div>
      }
    } @else {
      <p class="muted">
        No background tasks right now. When a session runs background commands, subagents, or
        workflows, they show up here with live status.
      </p>
    }

    @if (data(); as d) {
      <h2>Daemon</h2>
      <div class="card">
        <span
          class="badge"
          [class.working]="d.daemon?.reachable"
          [class.failed]="!d.daemon?.reachable"
        >
          {{ d.daemon?.reachable ? 'reachable' : 'unreachable' }}
        </span>
        <pre class="mono out">{{ d.daemon?.raw || d.daemon?.error || 'no output' }}</pre>
      </div>

      <h2>MCP servers</h2>
      @if (mcpNames(d).length) {
        @for (name of mcpNames(d); track name) {
          <div class="card mono">{{ name }}</div>
        }
      } @else {
        <p class="muted">None configured in ~/.claude.json.</p>
      }

      <h2>Background jobs (daemon)</h2>
      @if (d.backgroundJobs?.length) {
        @for (j of d.backgroundJobs; track j.id) {
          <div class="card mono">{{ j.id }} — {{ j.state }} — {{ j.cwd }}</div>
        }
      } @else {
        <p class="muted">No background jobs.</p>
      }
    } @else {
      <p class="muted">Loading…</p>
    }
  `,
  styles: [
    `
      .out {
        white-space: pre-wrap;
        margin: 0.5rem 0 0;
        font-size: 0.8rem;
      }
      .card {
        margin-bottom: 0.4rem;
      }
      .act .sess {
        display: flex;
        align-items: baseline;
        gap: 0.5rem;
        flex-wrap: wrap;
        color: var(--text);
        text-decoration: none;
        margin-bottom: 0.35rem;
        min-width: 0;
      }
      .act .sess:hover strong {
        color: var(--accent);
      }
      .task {
        display: flex;
        align-items: baseline;
        gap: 0.45rem;
        padding: 0.25rem 0;
        border-top: 1px solid var(--border);
        min-width: 0;
      }
      .task.done {
        opacity: 0.75;
      }
      .tico {
        flex: 0 0 auto;
      }
      .tlabel {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        min-width: 0;
      }
      .small {
        font-size: 0.75rem;
      }
      .badge.completed {
        color: var(--green);
      }
    `,
  ],
})
export class ServicesComponent {
  private api = inject(ApiService);
  private realtime = inject(RealtimeService);

  protected readonly data = signal<ServicesPayload | null>(null);
  protected readonly error = signal('');
  protected readonly rel = relativeTime;

  /** Live per-session background tasks: running first, then recent finished. */
  protected readonly activity = computed<SessionActivity[]>(() =>
    this.realtime
      .sessions()
      .filter((s) => s.tasks?.length)
      .map((s) => {
        const tasks = s.tasks ?? [];
        return {
          session: s,
          // A dead session can't finish its tasks — only live ones are "running".
          running: s.running ? tasks.filter((t) => t.status === 'running') : [],
          finished: tasks
            .filter((t) => t.status !== 'running')
            .slice(-5)
            .reverse(),
        };
      })
      .filter((a) => a.running.length || a.finished.length)
      .sort((a, b) => b.running.length - a.running.length),
  );

  constructor() {
    this.load();
  }

  protected icon(kind: string): string {
    return KIND_ICONS[kind] ?? '⚙';
  }

  protected mcpNames(d: ServicesPayload): string[] {
    return d.mcpServers ? Object.keys(d.mcpServers) : [];
  }

  private async load(): Promise<void> {
    try {
      this.data.set((await this.api.getServices()) as ServicesPayload);
    } catch (e) {
      this.error.set(`Could not load services: ${(e as Error).message}`);
    }
  }
}
