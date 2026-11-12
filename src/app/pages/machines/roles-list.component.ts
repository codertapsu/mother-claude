import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { RunsService } from '../../core/runs.service';
import { sessionLink } from '../../core/session-links';
import { HostedRole } from '../../core/tasks';
import { ChipComponent } from '../../shared/chip.component';
import { IconComponent } from '../../shared/icon.component';
import { RelativeTimePipe } from '../../shared/time.pipes';
import { Tone } from '../../shared/util';

/**
 * Team roles this computer runs for another computer's lead, on the
 * Machines page's Runs card (docs/PLAN-MACHINES.md Phase 4, decision 9):
 * the team and role, whose lead, the project, how its session stands, its
 * session (opened here), the work it brought back; *Stop* while its session
 * runs (its lead starts it again when it has work), *Remove* (its session,
 * worktree and record here). Shown only when there is one.
 */
@Component({
  selector: 'mc-machines-roles',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ChipComponent, IconComponent, RelativeTimePipe, RouterLink, TPipe],
  template: `
    @if (runs.roles().length) {
      <h3>{{ 'Team roles' | t }}</h3>
      <p class="muted small">
        {{
          'Roles of teams whose lead is on another computer, run here in a worktree of their own at the lead’s code. The lead starts and stops them; their board is on its computer.'
            | t
        }}
      </p>
      @if (error()) {
        <p class="notice error small" role="alert">{{ error() }}</p>
      }
      <ul class="items">
        @for (r of runs.roles(); track r.binding) {
          <li [attr.data-state]="r.state">
            <div class="row">
              <mc-icon name="download" [size]="15" />
              <strong class="name">{{
                '{role} of {team}' | t: { role: r.roleTitle || r.role, team: r.teamLabel }
              }}</strong>
              <span class="muted small">{{ 'for {name}' | t: { name: r.leadName } }}</span>
              <span class="spacer"></span>
              <mc-chip [tone]="tone(r)">{{ label(r) }}</mc-chip>
            </div>
            <span class="muted small meta">
              @if (r.projectName) {
                <span>{{ r.projectName }}</span>
              }
              <span class="mono path">{{ r.folder }}</span>
              @if (r.results) {
                <span>{{
                  r.results === 1 ? ('1 result' | t) : ('{n} results' | t: { n: r.results })
                }}</span>
              }
              @if (r.denials) {
                <span>{{ '{n} guard denials' | t: { n: r.denials } }}</span>
              }
              @if (r.startedAt) {
                <span>{{ r.startedAt | relTime }}</span>
              }
            </span>
            @if (r.error) {
              <p class="small detail warn">{{ r.error }}</p>
            }
            @if (r.keptNote) {
              <p class="small detail">{{ r.keptNote }}</p>
            }
            <div class="row acts">
              @if (r.session) {
                <a class="btn ghost small" [routerLink]="link(r)">{{ 'Open its session' | t }}</a>
              }
              @if (r.alive) {
                <button
                  type="button"
                  class="btn small"
                  [disabled]="busy() === r.binding"
                  (click)="stop(r)"
                >
                  {{ 'Stop' | t }}
                </button>
              }
              <button
                type="button"
                class="btn ghost small"
                [class.danger]="armed() === r.binding"
                [disabled]="busy() === r.binding"
                [attr.aria-label]="
                  'Remove the {role} role here' | t: { role: r.roleTitle || r.role }
                "
                (click)="remove(r)"
              >
                {{ armed() === r.binding ? ('Tap again to remove' | t) : ('Remove' | t) }}
              </button>
            </div>
          </li>
        }
      </ul>
    }
  `,
  styles: [
    `
      :host {
        display: block;
      }
      h3 {
        margin: 0.9rem 0 0.2rem;
        font-size: 0.95rem;
      }
      .items {
        list-style: none;
        margin: 0.3rem 0;
        padding: 0;
      }
      .items > li {
        display: flex;
        flex-direction: column;
        gap: 0.2rem;
        padding: 0.5rem 0;
        border-bottom: 1px solid var(--border);
      }
      .items > li:last-child {
        border-bottom: 0;
      }
      .row {
        flex-wrap: wrap;
      }
      .meta {
        display: flex;
        flex-wrap: wrap;
        gap: 0.2rem 0.6rem;
      }
      .path {
        overflow-wrap: anywhere;
      }
      .detail {
        margin: 0;
        overflow-wrap: anywhere;
      }
      .warn {
        color: var(--amber);
      }
      .acts {
        margin-top: 0.1rem;
      }
    `,
  ],
})
export class RolesListComponent {
  private readonly api = inject(ApiService);
  protected readonly runs = inject(RunsService);
  protected readonly busy = signal<string | null>(null);
  protected readonly armed = signal<string | null>(null);
  protected readonly error = signal('');

  protected label(r: HostedRole): string {
    if (r.state === 'running' && r.alive) return r.busy ? tr('Working') : tr('Idle');
    switch (r.state) {
      case 'sending':
        return tr('Sending the code');
      case 'starting':
        return tr('Starting its session');
      case 'error':
        return tr('Could not start');
      default:
        return tr('Stopped');
    }
  }

  protected tone(r: HostedRole): Tone {
    if (r.state === 'error') return 'amber';
    if (r.alive) return r.busy ? 'accent' : 'green';
    return 'muted';
  }

  /** Its session's page, here. */
  protected link(r: HostedRole): string[] {
    return sessionLink(r.session ?? '');
  }

  protected async stop(r: HostedRole): Promise<void> {
    await this.withBusy(r.binding, async () => {
      await this.api.stopHostedRole(r.binding);
      await this.runs.load();
    });
  }

  /** Twice: its worktree here goes with it. */
  protected async remove(r: HostedRole): Promise<void> {
    if (this.armed() !== r.binding) {
      this.armed.set(r.binding);
      return;
    }
    this.armed.set(null);
    await this.withBusy(r.binding, async () => {
      await this.api.removeHostedRole(r.binding);
      this.runs.roles.update((list) => list.filter((x) => x.binding !== r.binding));
    });
  }

  private async withBusy(key: string, fn: () => Promise<void>): Promise<void> {
    this.busy.set(key);
    this.error.set('');
    try {
      await fn();
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(null);
    }
  }
}
