import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  inject,
  output,
  signal,
} from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { Router } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { NoticeService } from '../../core/notice.service';
import { RealtimeService } from '../../core/realtime.service';
import { TeamView, TemplateEntry } from '../../core/teams';
import { HubTabsComponent } from '../../shared/hub-tabs.component';
import { IconComponent } from '../../shared/icon.component';
import { PageHeadComponent } from '../../shared/page-head.component';
import { StartTeamSheetComponent } from './start-team-sheet.component';

/**
 * The head of every Teams page (docs/UI-REDESIGN.md §6.2): the name,
 * *Start a team* — the place's one primary action, a sheet that reads the
 * templates when it opens — one sentence with *How it works ›*, and the
 * tabs: Teams · Templates · Capacity. A started team opens on its page.
 */
@Component({
  selector: 'mc-teams-head',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [HubTabsComponent, IconComponent, PageHeadComponent, StartTeamSheetComponent, TPipe],
  template: `
    <mc-page-head
      [heading]="'Teams' | t"
      [lede]="
        'A lead session plans the work and role sessions do it, each in its own files, as capacity allows.'
          | t
      "
      help="teams"
    >
      <button class="btn primary" type="button" (click)="start()">
        <mc-icon name="plus" [size]="16" /> {{ 'Start a team' | t }}
      </button>
    </mc-page-head>
    <mc-hub-tabs place="teams" />
    <mc-start-team-sheet
      [open]="startOpen()"
      [templates]="templates()"
      (closed)="startOpen.set(false)"
      (started)="started($event)"
    />
  `,
  styles: [
    `
      :host {
        display: block;
      }
    `,
  ],
})
export class TeamsHeadComponent {
  private readonly api = inject(ApiService);
  private readonly notices = inject(NoticeService);
  private readonly router = inject(Router);

  protected readonly startOpen = signal(false);
  protected readonly templates = signal<TemplateEntry[]>([]);
  /** A team started from the sheet (its page opens next). */
  readonly teamStarted = output<TeamView>();
  private read = false;

  constructor() {
    // A template written or removed elsewhere is in the next start.
    inject(RealtimeService)
      .teams$.pipe(takeUntilDestroyed(inject(DestroyRef)))
      .subscribe((f) => {
        if (f.templates) this.read = false;
      });
  }

  /** Open *Start a team* (the Teams tab's empty state too). */
  start(): void {
    this.startOpen.set(true);
    if (!this.read) void this.loadTemplates();
  }

  private async loadTemplates(): Promise<void> {
    try {
      this.templates.set((await this.api.getTeamTemplates()).templates);
      this.read = true;
    } catch (e) {
      this.notices.show(
        'error',
        tr('Could not load the templates: {error}', { error: describeError(e).text }),
      );
    }
  }

  protected started(team: TeamView): void {
    this.startOpen.set(false);
    this.teamStarted.emit(team);
    void this.router.navigate(['/teams', team.id]);
  }
}
