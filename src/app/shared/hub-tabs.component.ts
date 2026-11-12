import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';
import { IsActiveMatchOptions, RouterLink, RouterLinkActive } from '@angular/router';

import { TPipe } from '../core/i18n/t.pipe';
import { NavTab, PLACES, tabsOf } from '../core/nav';
import { NavBadges } from '../core/nav-badges';

/** A tab is current on its path (and below it), whatever the query. */
const SUBSET: IsActiveMatchOptions = {
  paths: 'subset',
  queryParams: 'ignored',
  fragment: 'ignored',
  matrixParams: 'ignored',
};
/** …or on exactly its path. */
const EXACT: IsActiveMatchOptions = { ...SUBSET, paths: 'exact' };

/**
 * A place's tabs (docs/UI-REDESIGN.md §5 (3)): links to its pages, from the
 * map (core/nav.ts) — the current one marked `aria-current="page"`, each
 * with its count while something goes on, scrolling sideways on a phone.
 * Panels inside one page (a session's Chat · Agents · Changes · Info) stay
 * a `tablist`; these are routes. Nothing shows for a place of one page.
 */
@Component({
  selector: 'mc-hub-tabs',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, RouterLinkActive, TPipe],
  template: `
    @if (shown().length > 1) {
      <nav class="hub-tabs" [attr.aria-label]="(navLabel() | t) || null">
        @for (t of shown(); track t.path) {
          <a
            class="tab"
            [routerLink]="t.path"
            routerLinkActive="current"
            ariaCurrentWhenActive="page"
            [routerLinkActiveOptions]="t.exact ? exact : subset"
            [attr.aria-label]="badges.label(t)"
          >
            {{ t.label | t }}
            @if (badges.count(t.badge); as n) {
              <span class="tcount" aria-hidden="true">{{ n }}</span>
            } @else if (badges.dot(t.badge)) {
              <span class="tdot" aria-hidden="true"></span>
            }
          </a>
        }
      </nav>
    }
  `,
  styles: [
    `
      :host {
        display: block;
      }
      .hub-tabs {
        display: flex;
        gap: 0.15rem;
        margin: 0 0 1rem;
        border-bottom: 1px solid var(--border);
        overflow-x: auto;
        scrollbar-width: none;
        -webkit-overflow-scrolling: touch;
      }
      .hub-tabs::-webkit-scrollbar {
        display: none;
      }
      .tab {
        display: inline-flex;
        align-items: center;
        gap: 0.35rem;
        flex: 0 0 auto;
        min-height: max(40px, var(--btn-h));
        padding: 0.35rem 0.8rem;
        border-bottom: 2px solid transparent;
        margin-bottom: -1px;
        color: var(--muted);
        font-size: 0.9rem;
        font-weight: 500;
        text-decoration: none;
        white-space: nowrap;
      }
      .tab:hover {
        color: var(--text);
      }
      .tab.current {
        color: var(--text);
        border-bottom-color: var(--accent);
      }
      .tcount {
        font-size: 0.7rem;
        font-weight: 700;
        min-width: 1.2rem;
        padding: 0 0.3rem;
        border-radius: 0.6rem;
        background: var(--panel-3);
        color: var(--muted);
        text-align: center;
      }
      .tdot {
        width: 0.5rem;
        height: 0.5rem;
        border-radius: 50%;
        background: var(--red);
      }
      @media (max-width: 767px) {
        .hub-tabs {
          gap: 0;
        }
        .tab {
          padding: 0.35rem 0.6rem;
        }
      }
    `,
  ],
})
export class HubTabsComponent {
  protected readonly badges = inject(NavBadges);
  protected readonly subset = SUBSET;
  protected readonly exact = EXACT;

  /** The place whose tabs show (an id of the map). */
  readonly place = input('');
  /** Tabs of no place of the map: these instead. */
  readonly tabs = input<readonly NavTab[] | null>(null);

  protected readonly shown = computed(() => this.tabs() ?? tabsOf(this.place()));
  /** The tabs' name for screen readers: the place's (English). */
  protected readonly navLabel = computed(
    () => PLACES.find((p) => p.id === this.place())?.label ?? '',
  );
}
