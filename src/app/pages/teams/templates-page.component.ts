import { Component, DestroyRef, inject, signal } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';

import { ApiService, describeError } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { RealtimeService } from '../../core/realtime.service';
import { TemplateEntry, machineWords } from '../../core/teams';
import { IconComponent } from '../../shared/icon.component';
import { TemplateSheetComponent, TemplateSheetMode } from './template-sheet.component';
import { TeamsHeadComponent } from './teams-head.component';

/**
 * *Teams › Templates* (docs/TEAMS.md "Templates"): the workflows a team
 * starts from — the built-in ones (viewed, duplicated) and your own (edited
 * as JSON, duplicated, deleted) — each with its roles, models and where they
 * run, and a template's problems. Kept current by `teams` frames that say
 * the templates changed.
 */
@Component({
  selector: 'mc-team-templates',
  imports: [IconComponent, TeamsHeadComponent, TemplateSheetComponent, TPipe],
  template: `
    <div class="page narrow">
      <mc-teams-head />

      @if (error()) {
        <p class="notice error" role="alert">{{ error() }}</p>
      }

      <div class="row section-head">
        <h2>{{ 'Templates' | t }}</h2>
        <span class="spacer"></span>
        <button class="btn small" type="button" (click)="openTemplate(null, 'new')">
          <mc-icon name="plus" [size]="14" /> {{ 'New template' | t }}
        </button>
      </div>
      @if (templates().length) {
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
      } @else if (loading()) {
        <p class="muted" role="status">{{ 'Loading…' | t }}</p>
      }
    </div>

    <mc-template-sheet
      [open]="!!editing()"
      [entry]="editing()?.entry ?? null"
      [mode]="editing()?.mode ?? 'view'"
      (closed)="editing.set(null)"
      (saved)="load()"
      (duplicated)="load()"
    />
  `,
  styles: [
    `
      .section-head h2 {
        margin: 0;
      }
      .section-head {
        margin: 1.25rem 0 0.55rem;
      }
      .list {
        padding-top: 0;
        padding-bottom: 0;
      }
      .template {
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
export class TemplatesPageComponent {
  private api = inject(ApiService);

  protected readonly templates = signal<TemplateEntry[]>([]);
  protected readonly loading = signal(true);
  protected readonly error = signal('');
  protected readonly editing = signal<{
    entry: TemplateEntry | null;
    mode: TemplateSheetMode;
  } | null>(null);

  constructor() {
    void this.load();
    const realtime = inject(RealtimeService);
    const destroyRef = inject(DestroyRef);
    realtime.teams$.pipe(takeUntilDestroyed(destroyRef)).subscribe((f) => {
      if (f.templates) void this.load();
    });
    realtime.refetch$.pipe(takeUntilDestroyed(destroyRef)).subscribe(() => void this.load());
  }

  /** "Lead (lead) · opus  ·  Developer (on DESKTOP-PC) · sonnet". */
  protected rolesLine(t: TemplateEntry): string {
    return t.roles
      .map((r) => {
        const name = r.lead ? tr('{role} (lead)', { role: r.title }) : r.title;
        const where = machineWords(r.machine);
        const named = where ? `${name} (${tr('on {computer}', { computer: where })})` : name;
        return r.model ? `${named} · ${r.model}` : named;
      })
      .join('  ·  ');
  }

  protected async load(): Promise<void> {
    try {
      this.templates.set((await this.api.getTeamTemplates()).templates);
      this.error.set('');
    } catch (e) {
      this.error.set(tr('Could not load the templates: {error}', { error: describeError(e).text }));
    } finally {
      this.loading.set(false);
    }
  }

  protected openTemplate(entry: TemplateEntry | null, mode: TemplateSheetMode): void {
    this.editing.set({ entry, mode });
  }

  protected async duplicate(t: TemplateEntry): Promise<void> {
    try {
      const copy = await this.api.duplicateTeamTemplate(t.id);
      await this.load();
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
}
