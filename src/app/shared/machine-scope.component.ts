import { ChangeDetectionStrategy, Component } from '@angular/core';

import { machineScopeProviders } from '../core/machine-scope';

/**
 * Puts what it holds in another computer's scope — the route's `:machine`
 * (`machine-scope.ts`): a session page inside it reads and drives that
 * computer's session through this one (docs/PLAN-MACHINES.md §6.2).
 */
@Component({
  selector: 'mc-machine-scope',
  changeDetection: ChangeDetectionStrategy.OnPush,
  providers: machineScopeProviders(),
  template: '<ng-content />',
  styles: [
    `
      :host {
        display: contents;
      }
    `,
  ],
})
export class MachineScopeComponent {}
