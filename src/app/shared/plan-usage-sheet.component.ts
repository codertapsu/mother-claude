import { ChangeDetectionStrategy, Component, input, output } from '@angular/core';

import { PlanUsageComponent } from './plan-usage.component';
import { SheetComponent } from './sheet.component';

/** The plan usage in a sheet (opened from the sessions page's summary). */
@Component({
  selector: 'mc-plan-usage-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [PlanUsageComponent, SheetComponent],
  template: `
    <mc-sheet [open]="open()" title="Plan usage" (closed)="closed.emit()">
      @if (open()) {
        <p class="intro muted small">Your Claude plan: every session on this account shares it.</p>
        <mc-plan-usage />
      }
    </mc-sheet>
  `,
  styles: [
    `
      .intro {
        margin: 0 0 0.9rem;
      }
    `,
  ],
})
export class PlanUsageSheetComponent {
  readonly open = input(false);
  readonly closed = output<void>();
}
