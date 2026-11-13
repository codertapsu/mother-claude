import {
  Component,
  DestroyRef,
  ElementRef,
  afterRenderEffect,
  computed,
  inject,
  untracked,
  viewChild,
} from '@angular/core';
import { ActivatedRoute, NavigationEnd, Router, RouterOutlet } from '@angular/router';
import { filter } from 'rxjs';

import { LIST_PATHS, ListDetailService, isWorkList } from '../../core/list-detail';
import { ListNavDirective, focusTarget, isField } from '../../shared/list-nav.directive';
import { AgentsComponent } from '../agents/agents.component';
import { InboxComponent } from '../inbox/inbox.component';
import { ProjectsPageComponent } from '../sessions/projects-page.component';
import { SessionsComponent } from '../sessions/sessions.component';

/**
 * The work's lists and the sessions they open, as one layout
 * (docs/UI-REDESIGN.md §9, stage 9): Sessions, its Agents and Projects
 * tabs and the Inbox are its list routes, and a session's pages
 * (`session/:id`, `…/on/:machine`, an agent's) its detail routes. It draws
 * the list itself — the list routes' own outlet shows nothing — so the
 * list stays the same one, scrolled where it was, its search and filters
 * kept, when a session opens.
 *
 * From 1280 px (`ListDetailService.wide`) the session shows beside the
 * list it was opened from (the Inbox for a notification's link opened
 * first; the Sessions list beside a session opened from Projects, whose
 * rows open none), and Esc closes it again. Narrower, a session takes the
 * page, as before. In every width ↓ and ↑ move through the list and Enter
 * opens an item (`mcListNav`).
 */
@Component({
  selector: 'mc-list-detail',
  imports: [
    RouterOutlet,
    AgentsComponent,
    InboxComponent,
    ListNavDirective,
    ProjectsPageComponent,
    SessionsComponent,
  ],
  host: {
    '[class.ld-split]': 'split()',
    '(document:keydown.escape)': 'onEscape($event)',
  },
  template: `
    @if (listShown()) {
      <div #pane class="ld-list" mcListNav>
        @switch (list()) {
          @case ('inbox') {
            <mc-inbox />
          }
          @case ('agents') {
            <mc-agents />
          }
          @case ('projects') {
            <mc-projects-page />
          }
          @default {
            <mc-sessions />
          }
        }
      </div>
    }
    <div class="ld-detail"><router-outlet /></div>
  `,
  styles: [
    `
      :host {
        display: block;
        --list-w: clamp(21rem, 30vw, 28rem);
      }
      :host(.ld-split) {
        display: grid;
        grid-template-columns: var(--list-w) minmax(0, 1fr);
        height: 100dvh;
      }
      :host(.ld-split) .ld-list {
        min-width: 0;
        min-height: 0;
        overflow-y: auto;
        overscroll-behavior: contain;
        scrollbar-gutter: stable;
        padding: 1.25rem 1rem 2rem;
        border-right: 1px solid var(--border);
      }
      :host(.ld-split) .ld-detail {
        min-width: 0;
        min-height: 0;
      }
    `,
  ],
})
export class ListDetailComponent {
  private readonly route = inject(ActivatedRoute);
  private readonly router = inject(Router);
  private readonly ld = inject(ListDetailService);

  private readonly pane = viewChild<ElementRef<HTMLElement>>('pane');

  /** A session shows beside the list. */
  protected readonly split = this.ld.split;
  /** The list on screen: a list route's own, else the one beside the
   * session. */
  protected readonly list = computed(() => (this.ld.detail() ? this.ld.beside() : this.ld.list()));
  /** Narrower than wide, a session takes the page. */
  protected readonly listShown = computed(() => !this.ld.detail() || this.ld.wide());

  /** The open session's item in the list, while one is open beside it. */
  private current: HTMLElement | null = null;
  private wasSplit = false;

  constructor() {
    this.read();
    const sub = this.router.events
      .pipe(filter((e) => e instanceof NavigationEnd))
      .subscribe(() => this.read());
    inject(DestroyRef).onDestroy(() => {
      sub.unsubscribe();
      this.ld.leave();
    });
    afterRenderEffect(() => {
      const split = this.split();
      this.ld.detail();
      untracked(() => this.keepPlace(split));
    });
  }

  /** What the route shows now: a list, or a session's page. */
  private read(): void {
    const child = this.route.firstChild?.snapshot;
    if (!child) return;
    const list: unknown = child.data['list'];
    if (isWorkList(list)) {
      const url = this.router.createUrlTree([LIST_PATHS[list]], {
        queryParams: child.queryParams,
        fragment: child.fragment ?? undefined,
      });
      this.ld.showList(list, this.router.serializeUrl(url));
      return;
    }
    const id = child.paramMap.get('id');
    if (!id) return;
    this.ld.showDetail(
      { id, machine: child.paramMap.get('machine'), agent: child.paramMap.get('agentId') },
      child.queryParamMap.has('pending'),
    );
  }

  /**
   * Keeps the list where the eye is: the open session's item comes into
   * view when the list narrows beside it, and when another one opens; when
   * the list is on its own again, the item of the session that was open
   * stays in view, and takes the focus if the list had none.
   */
  private keepPlace(split: boolean): void {
    const pane = this.pane()?.nativeElement;
    if (split) {
      const item = pane?.querySelector<HTMLElement>('[data-nav-current]') ?? null;
      if (item && item !== this.current) {
        item.scrollIntoView({ block: this.wasSplit ? 'nearest' : 'center' });
      }
      this.current = item;
    } else if (this.wasSplit) {
      const item = this.current;
      this.current = null;
      if (item?.isConnected) {
        item.scrollIntoView({ block: 'center' });
        if (!pane?.contains(document.activeElement))
          focusTarget(item).focus({ preventScroll: true });
      }
    }
    this.wasSplit = split;
  }

  /** Esc closes the session beside the list — not while a dialog is open
   * or a field has the keys. */
  protected onEscape(e: Event): void {
    if (!this.split() || e.defaultPrevented) return;
    const target = e.target instanceof Element ? e.target : null;
    if (isField(target) || target?.closest('dialog') || document.querySelector('dialog[open]')) {
      return;
    }
    e.preventDefault();
    void this.ld.close();
  }
}
