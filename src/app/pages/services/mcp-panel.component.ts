import { Component, DestroyRef, computed, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiService, errorMessage } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { McpCall, McpToolsStatus } from '../../core/models';
import { RealtimeService } from '../../core/realtime.service';
import { DesktopOnlyComponent } from '../../shared/desktop-only.component';
import { IconComponent } from '../../shared/icon.component';
import { RelativeTimePipe } from '../../shared/time.pipes';

/** How often the panel re-reads the server while the page is visible. */
const POLL_MS = 5000;

/**
 * Services › Claude Code tools: Mother Claude's MCP server, which every
 * Claude Code session on this computer reaches through the Mother Claude
 * plugin (docs/MCP.md). On/off (desktop only), the tool count, the last calls
 * (time, session, tool, ok — never arguments) and a hint when the installed
 * plugin is older than the one this app writes.
 */
@Component({
  selector: 'mc-mcp-panel',
  imports: [RouterLink, DesktopOnlyComponent, IconComponent, RelativeTimePipe, TPipe],
  template: `
    <h2>{{ 'Claude Code tools (MCP)' | t }}</h2>
    <div class="card">
      <p class="muted small first">
        {{
          'Mother Claude serves its own tools to every Claude Code session on this computer, through the Mother Claude plugin. The model sees them as'
            | t
        }}
        <code class="mono">{{ prefix() }}…</code>
      </p>
      @if (status(); as s) {
        @if (isDesktop()) {
          <label class="switch list-row">
            <input
              type="checkbox"
              [checked]="s.enabled"
              [disabled]="busy()"
              (change)="toggle($any($event.target).checked)"
            />
            <span class="switch-text">
              <span>{{ "Give sessions Mother Claude's tools" | t }}</span>
              <span class="muted small">
                @if (s.enabled) {
                  {{
                    'Sessions started from now on get them; running ones after /reload-plugins or a restart.'
                      | t
                  }}
                } @else {
                  {{ 'Off: sessions get no tools, and calls from running ones are refused.' | t }}
                }
              </span>
            </span>
          </label>
        } @else {
          <!-- A phone: the switch's value, and where it changes. -->
          <div class="list-row elsewhere">
            <span class="grow">{{ "Give sessions Mother Claude's tools" | t }}</span>
            <mc-desktop-only [value]="s.enabled ? ('On' | t) : ('Off' | t)" />
          </div>
        }
        <dl class="kv">
          <dt>{{ 'Tools' | t }}</dt>
          <dd>{{ s.toolCount }}</dd>
          <dt>{{ 'Sessions connected' | t }}</dt>
          <dd>
            {{ s.clients }} <span class="muted small">{{ '(last 15 min)' | t }}</span>
          </dd>
          <dt>{{ 'Sessions Mother Claude starts' | t }}</dt>
          <dd>{{ s.ownedSessions ? ('get the tools' | t) : ('get no tools' | t) }}</dd>
          @if (s.plugin; as p) {
            <dt>{{ 'Plugin' | t }}</dt>
            <dd>
              @if (p.installed) {
                {{ 'installed' | t }}
                @if (p.version) {
                  <span class="muted small">v{{ p.version }}</span>
                }
              } @else {
                {{ 'not installed' | t }}
              }
            </dd>
          }
        </dl>
        @if (hint(); as h) {
          <p class="notice warn hint">
            <mc-icon name="plug" [size]="14" />
            <span
              >{{ h }}
              <a routerLink="/settings/elsewhere">{{ 'Sessions started elsewhere' | t }}</a></span
            >
          </p>
        }

        <h3>{{ 'Last calls' | t }}</h3>
        @if (s.recentCalls.length) {
          <div class="list calls">
            @for (c of s.recentCalls; track $index) {
              <div class="list-row call">
                <span class="badge" [class.ok]="c.ok" [class.danger]="!c.ok">
                  {{ c.ok ? ('ok' | t) : ('error' | t) }}
                </span>
                <span class="mono small tool">{{ c.tool }}</span>
                <span class="small who">{{ sessionLabel(c) }}</span>
                <span class="muted small when">{{ c.at | relTime }}</span>
              </div>
            }
          </div>
        } @else {
          <p class="muted small">{{ 'No calls yet.' | t }}</p>
        }
      } @else if (!error()) {
        <p class="muted">{{ 'Loading…' | t }}</p>
      }
      @if (error()) {
        <p class="error-text small">{{ error() }}</p>
      }
    </div>
  `,
  styles: [
    `
      /* A phone: the value goes under the name when both don't fit. */
      .elsewhere {
        flex-wrap: wrap;
        gap: 0.25rem 0.75rem;
      }
      .elsewhere .grow {
        flex: 1 1 12rem;
        min-width: 0;
      }
      .first {
        margin-top: 0;
      }
      .hint {
        display: flex;
        gap: 0.5rem;
        align-items: flex-start;
      }
      h3 {
        font-size: 0.9rem;
        margin: 1rem 0 0.25rem;
      }
      .calls {
        padding: 0;
      }
      /* Columns line up on every row: outcome, tool, session, time. */
      .call {
        display: grid;
        grid-template-columns: 3.25rem minmax(0, 1fr) minmax(0, 1.4fr) auto;
        gap: 0.35rem 0.75rem;
      }
      .call .badge {
        justify-content: center;
      }
      .tool,
      .who {
        min-width: 0;
        overflow-wrap: anywhere;
      }
      .when {
        white-space: nowrap;
        text-align: right;
      }
      /* Phones: the session goes under the tool. */
      @media (max-width: 560px) {
        .call {
          grid-template-columns: 3.25rem minmax(0, 1fr) auto;
        }
        .who {
          grid-column: 2 / -1;
          grid-row: 2;
        }
      }
    `,
  ],
})
export class McpPanelComponent {
  private api = inject(ApiService);
  private realtime = inject(RealtimeService);
  protected readonly isDesktop = inject(AuthService).isDesktop;

  protected readonly status = signal<McpToolsStatus | null>(null);
  protected readonly error = signal('');
  protected readonly busy = signal(false);
  protected readonly prefix = computed(
    () => this.status()?.toolPrefix ?? 'mcp__plugin_mother-claude_mc__',
  );
  /** Session id → a name to show. */
  private readonly names = computed(
    () => new Map(this.realtime.sessions().map((s) => [s.id, s.title || s.name || s.projectName])),
  );

  /** Why sessions started outside Mother Claude may not get the tools. */
  protected readonly hint = computed(() => {
    const s = this.status();
    const p = s?.plugin;
    if (!s?.enabled || !p) return '';
    if (!p.installed) {
      return tr(
        'Sessions started outside Mother Claude get these tools once the Mother Claude plugin is installed:',
      );
    }
    if (p.outdated) {
      return p.version
        ? tr(
            "Update the plugin to give sessions Mother Claude's tools (installed: v{installed}, this app: v{expected}):",
            { installed: p.version, expected: p.expectedVersion },
          )
        : tr("Update the plugin to give sessions Mother Claude's tools:");
    }
    return '';
  });

  constructor() {
    void this.load(true);
    // Polls leave the plugin out (its state may run the CLI): the last
    // full answer's stays.
    const timer = setInterval(() => {
      if (document.visibilityState === 'visible' && !this.busy()) void this.load(false);
    }, POLL_MS);
    inject(DestroyRef).onDestroy(() => clearInterval(timer));
  }

  protected sessionLabel(c: McpCall): string {
    if (!c.session) return tr('unknown session');
    const name = this.names().get(c.session);
    const who = name || c.session.slice(0, 8);
    return c.owned ? tr('{name} · owned', { name: who }) : who;
  }

  protected async toggle(enabled: boolean): Promise<void> {
    this.busy.set(true);
    try {
      this.status.set(await this.api.setMcpToolsEnabled(enabled));
      this.error.set('');
    } catch (e) {
      this.error.set(tr('Could not change it: {error}', { error: errorMessage(e) }));
      await this.load(true);
    } finally {
      this.busy.set(false);
    }
  }

  private async load(withPlugin: boolean): Promise<void> {
    try {
      const next = await this.api.getMcpTools(withPlugin);
      const plugin = withPlugin ? next.plugin : (next.plugin ?? this.status()?.plugin);
      this.status.set({ ...next, plugin });
      this.error.set('');
    } catch (e) {
      this.error.set(tr('Could not load Claude Code tools: {error}', { error: errorMessage(e) }));
    }
  }
}
