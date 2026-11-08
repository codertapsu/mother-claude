import { Component, computed, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiService, errorMessage } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { AccountRef, ServicesInfo } from '../../core/models';
import { RealtimeService } from '../../core/realtime.service';
import { AccountChipComponent } from '../../shared/account-chip.component';
import { IconComponent } from '../../shared/icon.component';
import { McpPanelComponent } from './mcp-panel.component';

/** Daemon health, Mother Claude's tools for Claude Code, configured MCP
 * servers and daemon background jobs. Live agent/background activity lives on
 * the Agents page. */
@Component({
  selector: 'mc-services',
  imports: [RouterLink, AccountChipComponent, IconComponent, McpPanelComponent, TPipe],
  template: `
    <div class="page narrow">
      <div class="page-head">
        <h1>{{ 'Services' | t }}</h1>
        <span class="spacer"></span>
        <button class="btn small" type="button" [disabled]="loading()" (click)="load()">
          <mc-icon name="refresh" [size]="14" /> {{ 'Refresh' | t }}
        </button>
      </div>
      <p class="lede">
        {{ 'Running subagents, workflows and background shells are on the' | t }}
        <a routerLink="/agents">{{ 'Agents page' | t }}</a
        >.
      </p>

      @if (error()) {
        <p class="notice error">{{ error() }}</p>
      }

      <mc-mcp-panel />

      @if (data(); as d) {
        <h2>{{ 'Daemon' | t }}</h2>
        <div class="card">
          <span
            class="badge"
            [class.ok]="d.daemon?.reachable"
            [class.danger]="!d.daemon?.reachable"
          >
            {{ d.daemon?.reachable ? ('reachable' | t) : ('unreachable' | t) }}
          </span>
          <pre class="mono out">{{ d.daemon?.raw || d.daemon?.error || ('no output' | t) }}</pre>
        </div>

        <h2>{{ 'MCP servers' | t }}</h2>
        @if (mcpNames(d).length) {
          <div class="card list">
            @for (name of mcpNames(d); track name) {
              <div class="list-row mono">{{ name }}</div>
            }
          </div>
        } @else {
          <p class="muted">{{ 'None configured in ~/.claude.json.' | t }}</p>
        }

        <h2>{{ 'Background jobs (daemon)' | t }}</h2>
        @if (d.backgroundJobs?.length) {
          <div class="card list">
            @for (j of d.backgroundJobs; track j.id) {
              <div class="list-row job">
                <span class="mono small grow">{{ j.id }} — {{ j.state }} — {{ j.cwd }}</span>
                @if (accountOf(j.id); as a) {
                  <mc-account-chip [account]="a" />
                }
              </div>
            }
          </div>
        } @else {
          <p class="muted">{{ 'No background jobs.' | t }}</p>
        }
      } @else if (loading()) {
        <p class="muted">{{ 'Loading…' | t }}</p>
      }
    </div>
  `,
  styles: [
    `
      .out {
        white-space: pre-wrap;
        margin: 0.5rem 0 0;
        font-size: 0.8rem;
      }
      .list {
        padding-top: 0;
        padding-bottom: 0;
      }
      .job {
        flex-wrap: wrap;
        gap: 0.35rem 0.75rem;
      }
      .grow {
        flex: 1 1 14rem;
        min-width: 0;
        overflow-wrap: anywhere;
      }
    `,
  ],
})
export class ServicesComponent {
  private api = inject(ApiService);
  private realtime = inject(RealtimeService);

  protected readonly data = signal<ServicesInfo | null>(null);
  protected readonly error = signal('');
  protected readonly loading = signal(false);
  /** Session id → its Claude account (sessions name one only while there
   * are several). */
  private readonly accounts = computed(
    () => new Map(this.realtime.sessions().map((s) => [s.id, s.account])),
  );

  constructor() {
    void this.load();
  }

  /** A background job's Claude account (its id is its session's). */
  protected accountOf(sessionId: string): AccountRef | undefined {
    return this.accounts().get(sessionId);
  }

  protected mcpNames(d: ServicesInfo): string[] {
    return d.mcpServers ? Object.keys(d.mcpServers) : [];
  }

  protected async load(): Promise<void> {
    this.loading.set(true);
    try {
      this.data.set(await this.api.getServices());
      this.error.set('');
    } catch (e) {
      this.error.set(tr('Could not load services: {error}', { error: errorMessage(e) }));
    } finally {
      this.loading.set(false);
    }
  }
}
