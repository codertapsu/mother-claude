import { Component, computed, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { ApiService } from '../../core/api.service';
import { BridgeDefaults, BridgeStatus, LaunchDefaults, Session } from '../../core/models';
import { relativeTime } from '../../shared/util';

const EFFORT_LABELS: Record<string, string> = {
  low: 'Low',
  medium: 'Medium',
  high: 'High',
  xhigh: 'Extra High',
  max: 'Max',
};

const PERMISSION_MODES: { value: string; label: string; hint: string }[] = [
  { value: 'default', label: 'Ask me', hint: 'Every tool call needs approval' },
  { value: 'plan', label: 'Plan only', hint: 'Claude plans but changes nothing' },
  { value: 'acceptEdits', label: 'Accept edits', hint: 'File edits go through; other tools ask' },
  { value: 'dontAsk', label: "Don't ask", hint: 'No prompts — it just works' },
  { value: 'auto', label: 'Auto', hint: 'A classifier decides what needs you' },
];

/** New conversation on the first message, rather than joining an existing one. */
const FRESH = '';

/**
 * Start, configure and stop the HTTP API.
 *
 * Opening Mother Claude does not publish the API — starting it is a deliberate
 * act, because the port can drive Claude with your account. What you pick here
 * becomes the *defaults* for conversations the API creates; any request can
 * override them.
 */
@Component({
  selector: 'mc-bridge',
  imports: [FormsModule],
  template: `
    <h1>API</h1>
    <p class="lede">
      A local HTTP API for Claude on this machine — conversations, turns, live streams, tool
      approvals, images. It is off until you start it.
    </p>

    @if (error(); as e) {
      <div class="card err">{{ e }}</div>
    }

    @if (status(); as s) {
      <!-- ── Status ─────────────────────────────────────────────── -->
      <div class="card status">
        <div class="row head">
          <span class="badge" [class.working]="s.running" [class.stopped]="!s.running">
            {{ s.running ? 'running' : 'stopped' }}
          </span>
          @if (s.running && s.url) {
            <a class="mono url" [href]="s.url + '/docs/'" target="_blank" rel="noreferrer">{{
              s.url
            }}</a>
          } @else if (s.port) {
            <span class="mono muted">port {{ s.port }}</span>
          }
          <span class="spacer"></span>
          @if (s.running) {
            <button class="btn" type="button" [disabled]="busy()" (click)="stop()">Stop</button>
          }
        </div>

        <dl class="facts">
          <dt>Claude account</dt>
          <dd [class.warn]="!s.claudeAuthenticated">{{ s.claudeAccount }}</dd>
          <dt>Runtime</dt>
          <dd>
            {{ s.hostBuilt ? (s.hostRunning ? 'running' : 'ready') : 'not built' }}
            @if (!s.hostBuilt) {
              <span class="muted small">— run <code>npm run sidecar:build</code></span>
            }
          </dd>
          <dt>Authentication</dt>
          <dd>
            {{ s.requireToken ? 'bearer token required' : 'none — any local tool can call it' }}
          </dd>
          @if (s.running) {
            <dt>Started</dt>
            <dd>{{ rel(s.startedAt) }}</dd>
            <dt>In flight</dt>
            <dd>{{ s.activeOperations }} operation{{ s.activeOperations === 1 ? '' : 's' }}</dd>
          }
        </dl>

        @if (!s.claudeAuthenticated) {
          <p class="warn">
            Claude itself is not signed in, so every message would fail. Run
            <code>claude auth login</code> in a terminal, then reload this page.
          </p>
        }

        @if (s.running && s.url) {
          <div class="links">
            <a class="btn" [href]="s.docsUrl" target="_blank" rel="noreferrer">API reference</a>
            <a class="btn" [href]="s.consoleUrl" target="_blank" rel="noreferrer">Console</a>
          </div>
          <pre class="mono snippet">{{ snippet(s) }}</pre>
        }
      </div>

      <!-- ── Defaults ───────────────────────────────────────────── -->
      <h2>{{ s.running ? 'Defaults in use' : 'Defaults' }}</h2>
      <p class="muted">
        Applied to conversations the API creates. Every one can be overridden in a request payload.
        Leave a field on <em>Default</em> to inherit your own Claude settings.
      </p>

      <div class="card form">
        <label class="wide">
          <span class="lbl">Working directory</span>
          <input
            [ngModel]="cwd()"
            (ngModelChange)="cwd.set($event)"
            placeholder="your home directory"
            spellcheck="false"
          />
          <span class="hint">Where conversations run, and which files Claude can reach.</span>
        </label>

        <div class="row">
          <label>
            <span class="lbl">Model</span>
            <select [ngModel]="model()" (ngModelChange)="model.set($event)">
              <option value="">
                Default{{ launch()?.model ? ' — ' + launch()!.model : '' }}
              </option>
              @for (m of launch()?.models ?? []; track m.value) {
                <option [value]="m.value" [title]="m.description || ''">{{ m.label }}</option>
              }
            </select>
          </label>

          <label>
            <span class="lbl">Effort</span>
            <select [ngModel]="effort()" (ngModelChange)="effort.set($event)">
              <option value="">
                Default{{ launch()?.effort ? ' — ' + effortLabel(launch()!.effort!) : '' }}
              </option>
              @for (e of efforts(); track e) {
                <option [value]="e">{{ effortLabel(e) }}</option>
              }
            </select>
          </label>

          <label>
            <span class="lbl">Thinking</span>
            <select [ngModel]="thinking()" (ngModelChange)="thinking.set($event)">
              <option value="">Default</option>
              <option value="on">On</option>
              <option value="off">Off</option>
            </select>
          </label>
        </div>

        <label class="wide">
          <span class="lbl">Permissions</span>
          <select [ngModel]="permissionMode()" (ngModelChange)="permissionMode.set($event)">
            @for (p of permissionModes; track p.value) {
              <option [value]="p.value">{{ p.label }} — {{ p.hint }}</option>
            }
          </select>
          <span class="hint">
            Approvals reach you as prompt cards here and on your phone, and over
            <code>GET /requests</code>.
          </span>
        </label>

        <label class="check">
          <input
            type="checkbox"
            [ngModel]="isolate()"
            (ngModelChange)="isolate.set($event)"
          />
          <span
            >Ignore project settings — don't load <code>CLAUDE.md</code>, hooks or MCP servers from
            the working directory</span
          >
        </label>
      </div>

      <!-- ── Conversation ───────────────────────────────────────── -->
      <h2>Conversation</h2>
      <p class="muted">
        Where a message with no <code>thread_id</code> goes. Pass
        <code>createNewChat: true</code> in any request to branch off a new one — it is false by
        default so a client that only sends <code>{{ '{' }} "message": "…" {{ '}' }}</code> keeps
        one continuous conversation.
      </p>

      <div class="card form">
        <label class="wide">
          <span class="lbl">Messages join</span>
          <select [ngModel]="thread()" (ngModelChange)="thread.set($event)">
            <option [value]="FRESH">A new conversation, created by the first message</option>
            @if (liveThreads(s).length) {
              <optgroup label="Running now">
                @for (t of liveThreads(s); track t.threadId) {
                  <option [value]="t.threadId">
                    {{ t.title || shortId(t.threadId) }} — {{ t.cwd }}
                  </option>
                }
              </optgroup>
            }
            @if (pastSessions(s).length) {
              <optgroup label="Earlier conversations">
                @for (p of pastSessions(s); track p.id) {
                  <option [value]="p.id">
                    {{ p.title || p.projectName || shortId(p.id) }} — {{ rel(p.lastActivity) }}
                  </option>
                }
              </optgroup>
            }
          </select>
          <span class="hint">
            Picking an earlier conversation resumes it — same history, same working directory.
          </span>
        </label>

        @if (s.running && thread() !== (s.activeThread ?? FRESH)) {
          <button class="btn" type="button" [disabled]="busy()" (click)="applyThread()">
            Switch conversation
          </button>
        }
      </div>

      <!-- ── Action ─────────────────────────────────────────────── -->
      <div class="actions">
        <button class="btn primary" type="button" [disabled]="busy()" (click)="start()">
          {{ busy() ? 'Working…' : s.running ? 'Restart with these defaults' : 'Start the API' }}
        </button>
        @if (!s.running) {
          <span class="muted small">
            Starting publishes
            <span class="mono">{{ s.url || 'the /v1 routes' }}</span>
            to local tools.
          </span>
        }
      </div>

      @if (!s.enabled) {
        <div class="card muted">
          The API is disabled for this process (<code>MOTHER_CLAUDE_BRIDGE=0</code>).
        </div>
      }
    } @else if (!error()) {
      <p class="muted">Loading…</p>
    }
  `,
  styles: [
    `
      .lede {
        margin: -4px 0 18px;
        max-width: 46rem;
      }
      h2 {
        margin: 26px 0 4px;
        font-size: 0.85rem;
        text-transform: uppercase;
        letter-spacing: 0.06em;
        opacity: 0.7;
      }
      h2 + .muted {
        margin: 0 0 10px;
        max-width: 46rem;
      }
      .card.err {
        border-color: var(--error, #a63232);
        color: var(--error, #a63232);
      }
      .status .head {
        align-items: center;
        gap: 10px;
        margin-bottom: 12px;
      }
      .url {
        text-decoration: none;
      }
      .facts {
        display: grid;
        grid-template-columns: max-content 1fr;
        gap: 4px 16px;
        margin: 0;
        font-size: 0.9rem;
      }
      .facts dt {
        opacity: 0.6;
      }
      .facts dd {
        margin: 0;
      }
      .warn {
        color: var(--warn, #b8860b);
      }
      .links {
        display: flex;
        gap: 8px;
        margin-top: 14px;
      }
      .snippet {
        margin: 12px 0 0;
        padding: 10px 12px;
        border-radius: 8px;
        background: rgba(127, 127, 127, 0.1);
        font-size: 12px;
        line-height: 1.5;
        overflow-x: auto;
        white-space: pre;
      }
      .form {
        display: flex;
        flex-direction: column;
        gap: 14px;
      }
      .form .row {
        gap: 12px;
        flex-wrap: wrap;
      }
      label {
        display: flex;
        flex-direction: column;
        gap: 4px;
        min-width: 0;
      }
      label.wide {
        width: 100%;
      }
      label.check {
        flex-direction: row;
        align-items: flex-start;
        gap: 8px;
        font-size: 0.9rem;
      }
      label.check input {
        margin-top: 2px;
      }
      .lbl {
        font-size: 0.78rem;
        text-transform: uppercase;
        letter-spacing: 0.04em;
        opacity: 0.6;
      }
      .hint {
        font-size: 0.8rem;
        opacity: 0.6;
      }
      input,
      select {
        font: inherit;
        padding: 7px 9px;
        border-radius: 7px;
        border: 1px solid rgba(127, 127, 127, 0.35);
        background: transparent;
        color: inherit;
        min-width: 0;
      }
      .actions {
        display: flex;
        align-items: center;
        gap: 12px;
        margin: 18px 0 40px;
        flex-wrap: wrap;
      }
      @media (max-width: 560px) {
        .form .row {
          flex-direction: column;
          align-items: stretch;
        }
        .actions .btn {
          width: 100%;
        }
      }
    `,
  ],
})
export class BridgeComponent {
  private api = inject(ApiService);

  readonly FRESH = FRESH;
  readonly permissionModes = PERMISSION_MODES;

  readonly status = signal<BridgeStatus | null>(null);
  readonly launch = signal<LaunchDefaults | null>(null);
  readonly sessions = signal<Session[]>([]);
  readonly error = signal('');
  readonly busy = signal(false);

  // Form state, seeded from whatever the API is already configured with.
  readonly cwd = signal('');
  readonly model = signal('');
  readonly effort = signal('');
  readonly thinking = signal('');
  readonly permissionMode = signal('default');
  readonly isolate = signal(false);
  readonly thread = signal(FRESH);

  readonly efforts = computed(() => this.launch()?.efforts ?? Object.keys(EFFORT_LABELS));

  constructor() {
    void this.load();
  }

  private async load(): Promise<void> {
    try {
      const [status, launch, sessions] = await Promise.all([
        this.api.getBridge(),
        this.api.getDefaults().catch(() => null),
        this.api.listSessions().catch(() => [] as Session[]),
      ]);
      this.launch.set(launch);
      this.sessions.set(sessions);
      this.apply(status);
      this.error.set('');
    } catch (e) {
      this.error.set(e instanceof Error ? e.message : String(e));
    }
  }

  /** Adopt a status payload, seeding the form from the defaults it reports. */
  private apply(status: BridgeStatus): void {
    this.status.set(status);
    const d = status.defaults ?? {};
    this.cwd.set(d.cwd ?? '');
    this.model.set(d.model ?? '');
    this.effort.set(d.effort ?? '');
    this.thinking.set(d.thinking ?? '');
    this.permissionMode.set(d.permissionMode ?? 'default');
    this.isolate.set(Array.isArray(d.settingSources) && d.settingSources.length === 0);
    this.thread.set(status.activeThread ?? FRESH);
  }

  private defaults(): BridgeDefaults {
    return {
      cwd: this.cwd().trim() || undefined,
      model: this.model() || undefined,
      effort: this.effort() || undefined,
      thinking: (this.thinking() || undefined) as 'on' | 'off' | undefined,
      permissionMode: this.permissionMode() || undefined,
      // `[]` is meaningful — it isolates the conversation — so it must not be
      // collapsed to "unset" the way an empty string is.
      settingSources: this.isolate() ? [] : undefined,
      defaultThread: this.thread() || undefined,
    };
  }

  private async run(action: () => Promise<BridgeStatus>): Promise<void> {
    this.busy.set(true);
    this.error.set('');
    try {
      this.apply(await action());
    } catch (e) {
      this.error.set(this.explain(e));
    } finally {
      this.busy.set(false);
    }
  }

  /** The server answers with its JSON envelope; show the message, not the status. */
  private explain(e: unknown): string {
    const raw = e instanceof Error ? e.message : String(e);
    try {
      const body = JSON.parse(raw) as { error?: { message?: string; field?: string } };
      if (body.error?.message) {
        return body.error.field
          ? `${body.error.message} (${body.error.field})`
          : body.error.message;
      }
    } catch {
      // Not JSON — the raw text is the best we have.
    }
    return raw;
  }

  start(): Promise<void> {
    return this.run(() => this.api.startBridge(this.defaults()));
  }

  stop(): Promise<void> {
    return this.run(() => this.api.stopBridge());
  }

  applyThread(): Promise<void> {
    return this.run(() => this.api.setBridgeThread(this.thread() || null));
  }

  liveThreads(s: BridgeStatus) {
    return s.threads ?? [];
  }

  /** Recent sessions that are not already listed as running conversations. */
  pastSessions(s: BridgeStatus): Session[] {
    const live = new Set((s.threads ?? []).map((t) => t.threadId));
    return this.sessions()
      .filter((session) => !live.has(session.id) && session.messageCount > 0)
      .sort((a, b) => (b.lastActivity ?? 0) - (a.lastActivity ?? 0))
      .slice(0, 25);
  }

  /** The call to paste into a terminal, matching this listener's auth policy. */
  snippet(s: BridgeStatus): string {
    const url = s.url ?? '/v1';
    const auth = s.requireToken ? ' \\\n  -H "Authorization: Bearer <token>"' : '';
    return (
      `curl -sS ${url}/chat${auth} \\\n` +
      `  -H 'Content-Type: application/json' \\\n` +
      `  -d '{"message":"What changed in this repo today?"}'`
    );
  }

  effortLabel(value: string): string {
    return EFFORT_LABELS[value] ?? value;
  }

  shortId(id: string): string {
    return id.slice(0, 8);
  }

  rel(at?: number): string {
    return relativeTime(at);
  }
}
