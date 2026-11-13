import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  effect,
  inject,
  untracked,
} from '@angular/core';
import { NavigationStart, Router, RouterLink } from '@angular/router';
import { filter } from 'rxjs';

import { AuthService } from '../../core/auth.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { defaultCode } from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { ProjectSettingsService } from '../../core/project-settings';
import { DesktopOnlyComponent } from '../../shared/desktop-only.component';
import { SheetComponent } from '../../shared/sheet.component';
import { ProjectOptionsComponent } from '../machines/project-options.component';
import { SyncCardComponent } from '../machines/sync-card.component';
import { ProjectAccountComponent } from './project-account.component';
import { ProjectFoldersComponent } from './project-folders.component';
import { ProjectLimitsComponent } from './project-limits.component';

/**
 * A project's settings in one sheet (docs/UI-REDESIGN.md §9, stage 10 —
 * F10: they were on four pages): its account (Accounts' per-project rule),
 * its folder on each computer (the shared folders), code sync, how it moves
 * with the copies of its sessions (Computers › Options › Each project) and
 * its teams' limits (Teams › Capacity). Each section is what edits it on its
 * own page, shown for this project; the computers' sections while Computers
 * is in use. A phone sees what only the desktop changes, and says so. Any
 * link inside closes it. The shell hosts it (`ProjectSettingsService`).
 */
@Component({
  selector: 'mc-project-settings-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    DesktopOnlyComponent,
    ProjectAccountComponent,
    ProjectFoldersComponent,
    ProjectLimitsComponent,
    ProjectOptionsComponent,
    RouterLink,
    SheetComponent,
    SyncCardComponent,
    TPipe,
  ],
  template: `
    <mc-sheet [open]="!!project()" [title]="title()" size="wide" (closed)="svc.close()">
      @if (project(); as p) {
        <div class="ps">
          <p class="mono small where">{{ p.cwd }}</p>

          <section aria-labelledby="ps-account">
            <h3 id="ps-account">{{ 'Account' | t }}</h3>
            <mc-project-account [cwd]="p.cwd" />
          </section>

          @if (computers()) {
            <section aria-labelledby="ps-folders">
              <h3 id="ps-folders">{{ 'On each computer' | t }}</h3>
              <mc-project-folders [cwd]="p.cwd" />
            </section>

            <section aria-labelledby="ps-sync">
              <h3 id="ps-sync">{{ 'Code sync' | t }}</h3>
              <p class="muted small">
                {{ 'Its code kept the same on another computer, both ways.' | t }}
              </p>
              <mc-machines-sync [project]="p.cwd" [readonly]="!isDesktop()" />
              <mc-desktop-only />
            </section>

            <section aria-labelledby="ps-moves">
              <h3 id="ps-moves">{{ 'When it moves' | t }}</h3>
              <p class="muted small">
                {{
                  'How its code travels and lands when a session moves, what Claude is asked to run after, the files git ignores that go with it, the computers that keep a copy of its sessions, and code sync.'
                    | t
                }}
              </p>
              <mc-machines-project-options
                [project]="p.cwd"
                [defaults]="code()"
                [readonly]="!isDesktop()"
              />
              <mc-desktop-only />
            </section>
          } @else {
            <section aria-labelledby="ps-computers">
              <h3 id="ps-computers">{{ 'Your other computers' | t }}</h3>
              <p class="muted small">
                {{
                  'Once Computers is set up, where this project is on each computer, its code sync and how it moves are here too.'
                    | t
                }}
                <a routerLink="/computers">{{ 'Open Computers' | t }}</a>
              </p>
            </section>
          }

          <section aria-labelledby="ps-limits">
            <h3 id="ps-limits">{{ 'Team limits' | t }}</h3>
            <mc-project-limits [cwd]="p.cwd" />
          </section>

          <p class="muted small foot">
            {{ 'What every project follows:' | t }}
            <a routerLink="/accounts">{{ 'Accounts' | t }}</a> ·
            <a routerLink="/computers/options">{{ 'Computers › Options' | t }}</a> ·
            <a routerLink="/teams/capacity">{{ 'Teams › Capacity' | t }}</a>
          </p>
        </div>
      }
    </mc-sheet>
  `,
  styles: [
    `
      .ps {
        display: flex;
        flex-direction: column;
        gap: 1.1rem;
      }
      .where {
        margin: -0.2rem 0 0;
        color: var(--muted);
        overflow-wrap: anywhere;
      }
      section {
        display: flex;
        flex-direction: column;
        gap: 0.45rem;
        padding-top: 0.9rem;
        border-top: 1px solid var(--border);
      }
      h3 {
        margin: 0;
        font-size: 0.95rem;
      }
      section > p {
        margin: 0;
      }
      /* The desktop changes it: no line to say so. */
      mc-desktop-only:empty {
        display: none;
      }
      .foot {
        margin: 0;
      }
    `,
  ],
})
export class ProjectSettingsSheetComponent {
  protected readonly svc = inject(ProjectSettingsService);
  private readonly machines = inject(MachinesService);
  protected readonly isDesktop = inject(AuthService).isDesktop;

  protected readonly project = this.svc.project;
  protected readonly title = computed(() => {
    const p = this.project();
    return p ? tr('{name} — project settings', { name: p.name }) : '';
  });
  /** Computers is in use: set up, or being set up, or a computer paired. */
  protected readonly computers = computed(() => {
    const v = this.machines.view();
    return !!v && (v.enabled || v.setupStep !== null || v.peers.length > 0);
  });
  /** The code options every project follows. */
  protected readonly code = computed(() => this.machines.view()?.settings.code ?? defaultCode());

  constructor() {
    // Computers' view, while it shows.
    effect((onCleanup) => {
      if (!this.project()) return;
      const release = untracked(() => this.machines.watch());
      void untracked(() => this.machines.load());
      onCleanup(release);
    });
    // A link inside leaves for another page: the sheet goes with it.
    const sub = inject(Router)
      .events.pipe(filter((e) => e instanceof NavigationStart))
      .subscribe(() => this.svc.close());
    inject(DestroyRef).onDestroy(() => sub.unsubscribe());
  }
}
