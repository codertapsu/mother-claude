import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';

import { TPipe } from '../../core/i18n/t.pipe';
import { AccountRef, ServicesInfo } from '../../core/models';
import { RealtimeService } from '../../core/realtime.service';
import { AccountChipComponent } from '../../shared/account-chip.component';

/**
 * What Claude Code runs on this computer besides its sessions (`GET
 * /api/services`): its background service (the daemon) and what it says,
 * the MCP servers configured in `~/.claude.json`, and the daemon's
 * background jobs, each with its Claude account. Shown in *Computers ›
 * This Mac › Details*, and on the Services page.
 */
@Component({
  selector: 'mc-services-details',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AccountChipComponent, TPipe],
  template: `
    @let d = data();
    <h2>{{ 'Claude Code’s background service' | t }}</h2>
    <div class="card">
      <span class="badge" [class.ok]="d.daemon?.reachable" [class.danger]="!d.daemon?.reachable">
        {{ d.daemon?.reachable ? ('reachable' | t) : ('unreachable' | t) }}
      </span>
      <pre class="mono out">{{ d.daemon?.raw || d.daemon?.error || ('no output' | t) }}</pre>
    </div>

    <h2>{{ 'MCP servers' | t }}</h2>
    @if (mcpNames().length) {
      <div class="card list">
        @for (name of mcpNames(); track name) {
          <div class="list-row mono">{{ name }}</div>
        }
      </div>
    } @else {
      <p class="muted">{{ 'None configured in ~/.claude.json.' | t }}</p>
    }

    <h2>{{ 'Background jobs' | t }}</h2>
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
  `,
  styles: [
    `
      :host {
        display: block;
      }
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
export class ServicesDetailsComponent {
  private readonly realtime = inject(RealtimeService);

  readonly data = input.required<ServicesInfo>();

  /** Session id → its Claude account (sessions name one only while there
   * are several). */
  private readonly accounts = computed(
    () => new Map(this.realtime.sessions().map((s) => [s.id, s.account])),
  );
  protected readonly mcpNames = computed(() => Object.keys(this.data().mcpServers ?? {}));

  /** A background job's Claude account (its id is its session's). */
  protected accountOf(sessionId: string): AccountRef | undefined {
    return this.accounts().get(sessionId);
  }
}
