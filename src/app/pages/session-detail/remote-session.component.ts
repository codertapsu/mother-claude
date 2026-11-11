import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { ActivatedRoute } from '@angular/router';
import { map } from 'rxjs';

import { MachineScopeComponent } from '../../shared/machine-scope.component';
import { SessionDetailComponent } from './session-detail.component';

/**
 * Another computer's session (`/session/:id/on/:machine`,
 * docs/PLAN-MACHINES.md §6.2): the session page, in that computer's scope —
 * its transcript, requests and actions go through this computer's proxy and
 * live updates. A link to another computer's session makes a new scope.
 */
@Component({
  selector: 'mc-remote-session',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [MachineScopeComponent, SessionDetailComponent],
  template: `
    @for (m of machine(); track m) {
      <mc-machine-scope><mc-session-detail /></mc-machine-scope>
    }
  `,
  styles: [
    `
      :host {
        display: contents;
      }
    `,
  ],
})
export class RemoteSessionComponent {
  private readonly route = inject(ActivatedRoute);
  /** The computer, as a one-item list: a new one makes a new scope. */
  protected readonly machine = toSignal(
    this.route.paramMap.pipe(map((p) => [p.get('machine') ?? ''])),
    { initialValue: [this.route.snapshot.paramMap.get('machine') ?? ''] },
  );
}
