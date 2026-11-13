import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { RouterLink } from '@angular/router';

import { TPipe } from '../../core/i18n/t.pipe';
import { HubTabsComponent } from '../../shared/hub-tabs.component';
import { IconComponent } from '../../shared/icon.component';
import { PageHeadComponent } from '../../shared/page-head.component';

/**
 * The head of every Sessions page (docs/UI-REDESIGN.md §6.3): the name,
 * what a page adds (the content: the list's *Select*), *New session* — the
 * place's one primary action — one sentence with *How it works ›*, and the
 * tabs: Sessions · Agents · Projects.
 */
@Component({
  selector: 'mc-sessions-head',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [HubTabsComponent, IconComponent, PageHeadComponent, RouterLink, TPipe],
  template: `
    <mc-page-head
      [heading]="'Sessions' | t"
      [lede]="
        'Every Claude Code session, whatever started it: what it is doing, and what it needs from you.'
          | t
      "
      help="sessions"
    >
      <ng-content />
      <a class="btn primary" routerLink="/new" [queryParams]="newQuery()"
        ><mc-icon name="plus" [size]="16" /> {{ 'New session' | t }}</a
      >
    </mc-page-head>
    <mc-hub-tabs place="sessions" />
  `,
  styles: [
    `
      :host {
        display: block;
      }
    `,
  ],
})
export class SessionsHeadComponent {
  /** Another computer the list shows (its id): a new session starts there. */
  readonly machine = input('');

  protected readonly newQuery = computed(() => {
    const m = this.machine();
    return m ? { machine: m } : null;
  });
}
