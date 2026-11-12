import { Component, computed, inject } from '@angular/core';
import { RouterLink } from '@angular/router';

import { AuthService } from '../../core/auth.service';
import { TPipe } from '../../core/i18n/t.pipe';
import { moreEntries } from '../../core/nav';
import { RealtimeService, connectionLabel } from '../../core/realtime.service';
import { IconComponent } from '../../shared/icon.component';

/** The phone's "More" tab: the places the bottom bar has no room for, from
 * the map (core/nav.ts). */
@Component({
  selector: 'mc-more',
  imports: [RouterLink, IconComponent, TPipe],
  template: `
    <div class="page narrow">
      <div class="card list">
        @for (e of entries(); track e.id) {
          <a class="list-row entry" [routerLink]="e.path">
            <span class="ico"><mc-icon [name]="e.icon" /></span>
            <span class="text">
              <span class="label">{{ e.moreLabel ?? e.label | t }}</span>
              @if (e.hint) {
                <span class="muted small">{{ e.hint | t }}</span>
              }
            </span>
            <mc-icon name="chevronRight" [size]="16" />
          </a>
        }
      </div>
      <p class="muted small conn">{{ 'Live updates: {state}' | t: { state: connLabel() } }}</p>
    </div>
  `,
  styles: [
    `
      .list {
        padding-top: 0;
        padding-bottom: 0;
      }
      .entry {
        color: var(--text);
        text-decoration: none;
      }
      .ico {
        width: 2.2rem;
        height: 2.2rem;
        border-radius: 0.6rem;
        background: var(--accent-soft);
        color: var(--accent-text);
        display: inline-flex;
        align-items: center;
        justify-content: center;
        flex: 0 0 auto;
      }
      .text {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
      }
      .label {
        font-weight: 500;
      }
      .entry > mc-icon {
        color: var(--faint);
      }
      .conn {
        text-align: center;
        margin-top: 1rem;
      }
    `,
  ],
})
export class MoreComponent {
  private auth = inject(AuthService);
  private readonly connection = inject(RealtimeService).connection;
  protected readonly connLabel = computed(() => connectionLabel(this.connection()));
  protected readonly entries = computed(() => moreEntries(this.auth.isDesktop()));
}
