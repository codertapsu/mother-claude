import { Component, DestroyRef, inject, signal } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { Router, RouterLink } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { RealtimeService } from '../../core/realtime.service';
import {
  ObservedEvent,
  ROLE_STATE_LABELS,
  ROLE_STATE_TONE,
  TeamView,
  TemplateEntry,
  openCount,
  upsertTeam,
} from '../../core/teams';
import { IconComponent } from '../../shared/icon.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';
import { StartTeamSheetComponent } from './start-team-sheet.component';
import { TemplateSheetComponent, TemplateSheetMode } from './template-sheet.component';

/** Teams of Claude Code sessions (docs/TEAMS.md): the teams, the workflow
 * templates they start from, and what interactive sessions' agent teams
 * report. One code path for the desktop and the phone. */
@Component({
  selector: 'mc-teams',
  imports: [
    RouterLink,
    IconComponent,
    TimeAgoComponent,
    TPipe,
    StartTeamSheetComponent,
    TemplateSheetComponent,
  ],
  template: `
    <div class="page narrow">
      <div class="page-head">
        <h1>{{ 'Teams' | t }}</h1>
        <span class="spacer"></span>
        <button class="btn primary" type="button" (click)="startOpen.set(true)">
          <mc-icon name="plus" [size]="16" /> {{ 'Start a team' | t }}
        </button>
      </div>
      <p class="lede">
        {{
          'A lead session plans the work and writes task packets; role sessions do them, each in its own files. Mother Claude starts and stops the sessions it runs as capacity allows.'
            | t
        }}
      </p>

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
        <div class="empty-state card">
          <strong>{{ 'No team yet.' | t }}</strong>
          <span class="muted">{{
            'Start one on a project: a lead and role sessions work through a task board, each role in its own files.'
              | t
          }}</span>
        </div>
      }

      <div class="row section-head">
        <h2>{{ 'Templates' | t }}</h2>
        <span class="spacer"></span>
        <button class="btn small" type="button" (click)="openTemplate(null, 'new')">
          <mc-icon name="plus" [size]="14" /> {{ 'New template' | t }}
        </button>
      </div>
      <div class="card list">
        @for (t of templates(); track t.id) {
          <div class="list-row template">
            <div class="grow">
              <div class="row">
                <strong>{{ t.name }}</strong>
                @if (t.builtin) {
                  <span class="badge info">{{ 'built in' | t }}</span>
                }
                @if (t.report.errors.length) {
                  <span class="badge danger">{{
                    t.report.errors.length === 1
                      ? ('1 problem' | t)
                      : ('{n} problems' | t: { n: t.report.errors.length })
                  }}</span>
                }
              </div>
              @if (t.description) {
                <div class="muted small">{{ t.description }}</div>
              }
              <div class="small roles-line">{{ rolesLine(t) }}</div>
            </div>
            <div class="row actions">
              <button
                class="btn small"
                type="button"
                (click)="openTemplate(t, t.builtin ? 'view' : 'edit')"
              >
                {{ t.builtin ? ('View' | t) : ('Edit' | t) }}
              </button>
              <button class="btn small" type="button" (click)="duplicate(t)">
                {{ 'Duplicate' | t }}
              </button>
              @if (!t.builtin) {
                <button
                  class="btn small ghost"
                  type="button"
                  [attr.aria-label]="'Delete {name}' | t: { name: t.name }"
                  (click)="remove(t)"
                >
                  <mc-icon name="trash" [size]="14" />
                </button>
              }
            </div>
          </div>
        }
      </div>

      @if (events().length) {
        <h2>{{ 'Agent teams in your sessions' | t }}</h2>
        <p class="muted small">
          {{
            "What Claude Code's own agent teams reported through the hooks (interactive sessions; Mother Claude observes them, it does not run them)."
              | t
          }}
        </p>
        <div class="card list">
          @for (e of events(); track $index) {
            <div class="list-row event">
              <span class="badge">{{ e.event }}</span>
              <span class="grow small"
                >{{ e.teammate || e.toModel || e.source || '' }}
                @if (e.subject) {
                  · {{ e.subject }}
                }
              </span>
              <mc-time class="faint small" [ms]="e.at" />
            </div>
          }
        </div>
      }
    </div>

    <mc-start-team-sheet
      [open]="startOpen()"
      [templates]="templates()"
      (closed)="startOpen.set(false)"
      (started)="started($event)"
    />
    <mc-template-sheet
      [open]="!!editing()"
      [entry]="editing()?.entry ?? null"
      [mode]="editing()?.mode ?? 'view'"
      (closed)="editing.set(null)"
      (saved)="loadTemplates()"
      (duplicated)="loadTemplates()"
    />
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
      .section-head {
        margin-top: 1.25rem;
      }
      .section-head h2 {
        margin: 0;
      }
      .list {
        padding-top: 0;
        padding-bottom: 0;
      }
      .template,
      .event {
        flex-wrap: wrap;
        gap: 0.4rem 0.75rem;
      }
      .grow {
        flex: 1 1 16rem;
        min-width: 0;
        overflow-wrap: anywhere;
      }
      .roles-line {
        color: var(--muted);
        margin-top: 0.2rem;
      }
      .actions {
        gap: 0.35rem;
      }
    `,
  ],
})
export class TeamsComponent {
  private api = inject(ApiService);
  private realtime = inject(RealtimeService);
  private router = inject(Router);

  protected readonly teams = signal<TeamView[]>([]);
  protected readonly templates = signal<TemplateEntry[]>([]);
  protected readonly events = signal<ObservedEvent[]>([]);
  protected readonly loading = signal(true);
  protected readonly error = signal('');
  protected readonly startOpen = signal(false);
  protected readonly editing = signal<{
    entry: TemplateEntry | null;
    mode: TemplateSheetMode;
  } | null>(null);
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
      if (f.templates) void this.loadTemplates();
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

  protected rolesLine(t: TemplateEntry): string {
    return t.roles
      .map((r) => {
        const name = r.lead ? tr('{role} (lead)', { role: r.title }) : r.title;
        return r.model ? `${name} · ${r.model}` : name;
      })
      .join('  ·  ');
  }

  protected async load(): Promise<void> {
    this.loading.set(true);
    try {
      const [teams, templates, events] = await Promise.all([
        this.api.getTeams(),
        this.api.getTeamTemplates(),
        this.api.getTeamEvents().catch(() => ({ events: [] })),
      ]);
      this.teams.set(teams.teams);
      this.templates.set(templates.templates);
      this.events.set(events.events.slice(0, 20));
      this.error.set('');
    } catch (e) {
      this.error.set(tr('Could not load the teams: {error}', { error: describeError(e).text }));
    } finally {
      this.loading.set(false);
    }
  }

  protected async loadTemplates(): Promise<void> {
    try {
      this.templates.set((await this.api.getTeamTemplates()).templates);
    } catch (e) {
      this.error.set(tr('Could not load the templates: {error}', { error: describeError(e).text }));
    }
  }

  protected openTemplate(entry: TemplateEntry | null, mode: TemplateSheetMode): void {
    this.editing.set({ entry, mode });
  }

  protected async duplicate(t: TemplateEntry): Promise<void> {
    try {
      const copy = await this.api.duplicateTeamTemplate(t.id);
      await this.loadTemplates();
      const entry = this.templates().find((x) => x.id === copy.id) ?? null;
      this.editing.set({ entry, mode: 'edit' });
    } catch (e) {
      this.error.set(tr('Could not duplicate it: {error}', { error: describeError(e).text }));
    }
  }

  protected async remove(t: TemplateEntry): Promise<void> {
    const ask = tr('Delete the template "{name}"? Teams started from it keep running.', {
      name: t.name,
    });
    if (!confirm(ask)) return;
    try {
      await this.api.deleteTeamTemplate(t.id);
      this.templates.update((list) => list.filter((x) => x.id !== t.id));
    } catch (e) {
      this.error.set(tr('Could not delete it: {error}', { error: describeError(e).text }));
    }
  }

  protected started(team: TeamView): void {
    this.startOpen.set(false);
    this.teams.update((list) => upsertTeam(list, team));
    void this.router.navigate(['/teams', team.id]);
  }
}
