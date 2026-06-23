import { Component, inject, signal } from '@angular/core';

import { ApiService } from '../../core/api.service';

interface ServicesPayload {
  mcpServers?: Record<string, unknown>;
  daemon?: { reachable?: boolean; raw?: string; error?: string };
  backgroundJobs?: { id: string; cwd: string; state: string }[];
}

@Component({
  selector: 'mc-services',
  imports: [],
  template: `
    <h1>Services</h1>
    @if (error()) {
      <div class="card muted">{{ error() }}</div>
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

      <h2>Background jobs</h2>
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
    `,
  ],
})
export class ServicesComponent {
  private api = inject(ApiService);
  protected readonly data = signal<ServicesPayload | null>(null);
  protected readonly error = signal('');

  constructor() {
    this.load();
  }

  private async load(): Promise<void> {
    try {
      this.data.set((await this.api.getServices()) as ServicesPayload);
    } catch (e) {
      this.error.set(`Could not load services: ${(e as Error).message}`);
    }
  }

  mcpNames(d: ServicesPayload): string[] {
    return d.mcpServers ? Object.keys(d.mcpServers) : [];
  }
}
