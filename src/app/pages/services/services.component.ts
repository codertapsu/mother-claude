import { Component, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiService, errorMessage } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { ServicesInfo } from '../../core/models';
import { IconComponent } from '../../shared/icon.component';
import { McpPanelComponent } from './mcp-panel.component';
import { ServicesDetailsComponent } from './services-details.component';

/** Daemon health, Mother Claude's tools for Claude Code, configured MCP
 * servers and daemon background jobs. Live agent/background activity lives on
 * the Agents page. */
@Component({
  selector: 'mc-services',
  imports: [RouterLink, IconComponent, McpPanelComponent, ServicesDetailsComponent, TPipe],
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
        <mc-services-details [data]="d" />
      } @else if (loading()) {
        <p class="muted">{{ 'Loading…' | t }}</p>
      }
    </div>
  `,
})
export class ServicesComponent {
  private api = inject(ApiService);

  protected readonly data = signal<ServicesInfo | null>(null);
  protected readonly error = signal('');
  protected readonly loading = signal(false);

  constructor() {
    void this.load();
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
