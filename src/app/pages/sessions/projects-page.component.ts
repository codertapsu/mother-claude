import { ChangeDetectionStrategy, Component } from '@angular/core';

import { ProjectsViewComponent } from './projects-view.component';
import { SessionsHeadComponent } from './sessions-head.component';

/**
 * *Sessions › Projects* (`/projects`, docs/UI-REDESIGN.md §6.2): every
 * project folder that holds conversations, in every account's directory
 * (`ProjectsViewComponent`, docs/ACCOUNTS.md §14.4), under the Sessions
 * head.
 */
@Component({
  selector: 'mc-projects-page',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ProjectsViewComponent, SessionsHeadComponent],
  template: `
    <div class="page narrow">
      <mc-sessions-head />
      <mc-projects-view />
    </div>
  `,
})
export class ProjectsPageComponent {}
