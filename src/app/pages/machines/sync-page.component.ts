import { Component, DestroyRef, computed, inject, signal, viewChild } from '@angular/core';
import { RouterLink } from '@angular/router';

import { AuthService } from '../../core/auth.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { MachinesService } from '../../core/machines.service';
import { EmptyStateComponent } from '../../shared/empty-state.component';
import { ComputersHeadComponent } from './computers-head.component';
import { KeepInSyncSheetComponent } from './keep-in-sync-sheet.component';
import { SyncCardComponent } from './sync-card.component';

/**
 * *Computers › Code sync* (docs/UI-REDESIGN.md §6.5, docs/PLAN-MACHINES.md
 * §5.15): the projects kept the same on this computer and another, how
 * each stands, and *Keep a project in sync…* — a sheet, no more going
 * through the setup. A phone sees them read-only.
 */
@Component({
  selector: 'mc-computers-sync',
  imports: [
    ComputersHeadComponent,
    EmptyStateComponent,
    KeepInSyncSheetComponent,
    RouterLink,
    SyncCardComponent,
    TPipe,
  ],
  template: `
    <div class="page narrow">
      <mc-computers-head />
      @if (notice()) {
        <p class="notice ok" role="status">{{ notice() }}</p>
      }
      @if (!machines.view()) {
        @if (machines.error()) {
          <p class="notice error" role="alert">{{ machines.error() }}</p>
        } @else {
          <p class="muted" role="status">{{ 'Loading…' | t }}</p>
        }
      } @else if (!paired()) {
        <mc-empty-state
          icon="refresh"
          [heading]="'No project kept in sync' | t"
          [text]="
            'Once another computer is paired, a project’s code can be kept the same on both: what changes on one goes to the other a few seconds later.'
              | t
          "
          help="machines-sync"
        >
          @if (isDesktop()) {
            @if (inUse()) {
              <button type="button" class="btn primary small" (click)="head()?.pair()">
                {{ 'Pair a computer' | t }}
              </button>
            } @else {
              <a class="btn primary small" routerLink="/computers/setup">{{
                'Set up your computers' | t
              }}</a>
            }
          }
        </mc-empty-state>
      } @else {
        <div class="section-head">
          <h2>{{ 'Projects kept in sync' | t }}</h2>
          <span class="spacer"></span>
          @if (isDesktop()) {
            <button type="button" class="btn small" (click)="keepOpen.set(true)">
              {{ 'Keep a project in sync…' | t }}
            </button>
          }
        </div>
        <mc-machines-sync [readonly]="!isDesktop()" />
      }
      @if (isDesktop()) {
        <mc-keep-in-sync-sheet
          [open]="keepOpen()"
          (closed)="keepOpen.set(false)"
          (asked)="asked($event)"
        />
      }
    </div>
  `,
  styles: [
    `
      .section-head {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        margin: 1.4rem 0 0.55rem;
      }
      .section-head h2 {
        margin: 0;
      }
    `,
  ],
})
export class SyncPageComponent {
  protected readonly machines = inject(MachinesService);
  protected readonly isDesktop = inject(AuthService).isDesktop;

  protected readonly head = viewChild(ComputersHeadComponent);
  protected readonly keepOpen = signal(false);
  protected readonly notice = signal('');
  /** Another computer is paired: a project may be kept in sync with it. */
  protected readonly paired = computed(() => (this.machines.view()?.peers.length ?? 0) > 0);
  /** On, or being set up: pairing is open. */
  protected readonly inUse = computed(() => {
    const v = this.machines.view();
    return !!v && (v.enabled || v.setupStep !== null || v.peers.length > 0);
  });

  constructor() {
    const release = this.machines.watch();
    inject(DestroyRef).onDestroy(release);
    void this.machines.load();
  }

  protected asked(name: string): void {
    this.notice.set(
      tr('Asked {name} to keep it in sync: it is on once someone there confirms.', { name }),
    );
  }
}
