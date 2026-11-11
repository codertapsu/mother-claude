import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { ActivatedRoute } from '@angular/router';
import { map } from 'rxjs';

import { MachineScopeComponent } from '../../shared/machine-scope.component';
import { AgentViewComponent } from './agent-view.component';

/**
 * One agent of another computer's session
 * (`/session/:id/agent/:agentId/on/:machine`): the agent page, in that
 * computer's scope (docs/PLAN-MACHINES.md §6.2).
 */
@Component({
  selector: 'mc-remote-agent',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AgentViewComponent, MachineScopeComponent],
  template: `
    @for (m of machine(); track m) {
      <mc-machine-scope><mc-agent-view /></mc-machine-scope>
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
export class RemoteAgentComponent {
  private readonly route = inject(ActivatedRoute);
  protected readonly machine = toSignal(
    this.route.paramMap.pipe(map((p) => [p.get('machine') ?? ''])),
    { initialValue: [this.route.snapshot.paramMap.get('machine') ?? ''] },
  );
}
