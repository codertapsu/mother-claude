import { Component, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiService, errorMessage } from '../../core/api.service';
import { ServicesInfo } from '../../core/models';
import { IconComponent } from '../../shared/icon.component';

/** Daemon health, configured MCP servers and daemon background jobs. Live
 * agent/background activity lives on the Agents page. */
@Component({
  selector: 'mc-services',
  imports: [RouterLink, IconComponent],
  template: `
    <div class="page narrow">
      <div class="page-head">
        <h1>Services</h1>
        <span class="spacer"></span>
        <button class="btn small" type="button" [disabled]="loading()" (click)="load()">
          <mc-icon name="refresh" [size]="14" /> Refresh
        </button>
      </div>
      <p class="lede">
        Running subagents, workflows and background shells are on the
        <a routerLink="/agents">Agents</a> page.
      </p>

      @if (error()) {
        <p class="notice error">{{ error() }}</p>
      }

      @if (data(); as d) {
        <h2>Daemon</h2>
        <div class="card">
          <span
            class="badge"
            [class.ok]="d.daemon?.reachable"
            [class.danger]="!d.daemon?.reachable"
          >
            {{ d.daemon?.reachable ? 'reachable' : 'unreachable' }}
          </span>
          <pre class="mono out">{{ d.daemon?.raw || d.daemon?.error || 'no output' }}</pre>
        </div>

        <h2>MCP servers</h2>
        @if (mcpNames(d).length) {
          <div class="card list">
            @for (name of mcpNames(d); track name) {
              <div class="list-row mono">{{ name }}</div>
            }
          </div>
        } @else {
          <p class="muted">None configured in ~/.claude.json.</p>
        }

        <h2>Background jobs (daemon)</h2>
        @if (d.backgroundJobs?.length) {
          <div class="card list">
            @for (j of d.backgroundJobs; track j.id) {
              <div class="list-row mono small">{{ j.id }} — {{ j.state }} — {{ j.cwd }}</div>
            }
          </div>
        } @else {
          <p class="muted">No background jobs.</p>
        }
      } @else if (loading()) {
        <p class="muted">Loading…</p>
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
    `,
  ],
})
export class ServicesComponent {
  private api = inject(ApiService);

  protected readonly data = signal<ServicesInfo | null>(null);
  protected readonly error = signal('');
  protected readonly loading = signal(false);

  constructor() {
    void this.load();
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
      this.error.set(`Could not load services: ${errorMessage(e)}`);
    } finally {
      this.loading.set(false);
    }
  }
}
