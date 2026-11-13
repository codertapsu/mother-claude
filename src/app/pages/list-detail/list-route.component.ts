import { ChangeDetectionStrategy, Component } from '@angular/core';

/**
 * A list's route (`/`, `/inbox`, `/agents`, `/projects`) in the list and
 * detail layout: the layout draws the list itself, so that it stays the
 * same list — scrolled where it was, its search and filters kept — when a
 * session opens beside it. This route's own outlet shows nothing.
 */
@Component({
  selector: 'mc-list-route',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: '',
  styles: [
    `
      :host {
        display: none;
      }
    `,
  ],
})
export class ListRouteComponent {}
