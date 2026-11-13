import { Component, DestroyRef, inject, signal } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { RouterLink } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { RealtimeService } from '../../core/realtime.service';
import {
  ROLE_STATE_LABELS,
  ROLE_STATE_TONE,
  RoleView,
  TeamView,
  openCount,
  roleComputer,
  upsertTeam,
} from '../../core/teams';
import { EmptyStateComponent } from '../../shared/empty-state.component';
import { IconComponent } from '../../shared/icon.component';
import { TeamsHeadComponent } from './teams-head.component';

/** *Teams › Teams* (docs/TEAMS.md): your teams of Claude Code sessions,
 * each with its roles and its board, kept current by `teams` frames. The
 * templates they start from are the Templates tab, Claude Code's own agent
 * teams *Sessions › Agents › Teammates*. One code path for the desktop and
 * the phone. */
@Component({
  selector: 'mc-teams',
  imports: [EmptyStateComponent, IconComponent, RouterLink, TeamsHeadComponent, TPipe],
  template: `
    <div class="page narrow">
      <mc-teams-head #head (teamStarted)="started($event)" />

      @if (error()) {
        <p class="notice error" role="alert">{{ error() }}</p>
      }

      <h2>{{ 'Your teams' | t }}</h2>
      @if (teams().length) {
        <div class="stack">
          @for (t of teams(); track t.id) {
            <a class="card team" [routerLink]="['/teams', t.id]">
              <div class="row">
                <strong class="name">{{ t.label }}</strong>
                @if (t.stopped) {
                  <span class="badge">{{ 'stopped' | t }}</span>
                }
                <span class="spacer"></span>
                <span class="muted small">{{ t.templateName }}</span>
              </div>
              <div class="mono small path">{{ t.project }}</div>
              <div class="row roles">
                @for (r of t.roles; track r.id) {
                  <span class="badge" [class]="'badge ' + tone(r.state)" [title]="r.sessionName">
                    {{ r.title }} · {{ label(r.state) }}
                    @if (r.machine) {
                      · {{ 'on {computer}' | t: { computer: computerOf(r) } }}
                    }
                  </span>
                }
              </div>
              <div class="muted small">
                {{ '{n} open' | t: { n: openCount(t) } }} ·
                {{ '{n} done' | t: { n: t.counts.done ?? 0 } }}
                @if (t.counts.review) {
                  · {{ '{n} to review' | t: { n: t.counts.review } }}
                }
                @if (t.counts.failed) {
                  · {{ '{n} failed' | t: { n: t.counts.failed } }}
                }
              </div>
            </a>
          }
        </div>
      } @else if (!loading()) {
        <mc-empty-state
          icon="layers"
          [heading]="'No team yet.' | t"
          [text]="
            'Start one on a project: a lead and role sessions work through a task board, each role in its own files.'
              | t
          "
          help="teams"
        >
          <button class="btn primary" type="button" (click)="head.start()">
            <mc-icon name="plus" [size]="16" /> {{ 'Start a team' | t }}
          </button>
        </mc-empty-state>
      }
    </div>
  `,
  styles: [
    `
      .team {
        display: flex;
        flex-direction: column;
        gap: 0.35rem;
        color: inherit;
        text-decoration: none;
      }
      .team:hover {
        border-color: var(--accent);
      }
      .name {
        overflow-wrap: anywhere;
      }
      .path {
        color: var(--muted);
        overflow-wrap: anywhere;
      }
      .roles {
        gap: 0.3rem;
      }
    `,
  ],
})
export class TeamsComponent {
  private api = inject(ApiService);
  private realtime = inject(RealtimeService);

  protected readonly teams = signal<TeamView[]>([]);
  protected readonly loading = signal(true);
  protected readonly error = signal('');
  protected readonly openCount = openCount;

  constructor() {
    void this.load();
    this.realtime.teams$.pipe(takeUntilDestroyed(inject(DestroyRef))).subscribe((f) => {
      if (f.team) {
        const team = f.team;
        this.teams.update((list) => upsertTeam(list, team));
      }
      if (f.removed) {
        const gone = f.removed;
        this.teams.update((list) => list.filter((t) => t.id !== gone));
      }
    });
    this.realtime.refetch$.pipe(takeUntilDestroyed(inject(DestroyRef))).subscribe(() => {
      void this.load();
    });
  }

  protected label(state: TeamView['roles'][number]['state']): string {
    return tr(ROLE_STATE_LABELS[state] ?? state);
  }

  protected tone(state: TeamView['roles'][number]['state']): string {
    return ROLE_STATE_TONE[state] ?? '';
  }

  protected computerOf(r: RoleView): string {
    return roleComputer(r);
  }

  protected async load(): Promise<void> {
    this.loading.set(true);
    try {
      this.teams.set((await this.api.getTeams()).teams);
      this.error.set('');
    } catch (e) {
      this.error.set(tr('Could not load the teams: {error}', { error: describeError(e).text }));
    } finally {
      this.loading.set(false);
    }
  }

  /** A team started from the head's sheet (its page opens next). */
  protected started(team: TeamView): void {
    this.teams.update((list) => upsertTeam(list, team));
  }
}
