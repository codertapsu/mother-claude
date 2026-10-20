import { ChangeDetectionStrategy, Component, inject, input, output } from '@angular/core';

import { AccountsService } from '../core/accounts.service';
import { PlanUsageService } from '../core/plan-usage.service';
import { AccountChipComponent } from './account-chip.component';
import { TOKEN_NO_USAGE, signInLine } from './accounts';
import { PlanUsageComponent } from './plan-usage.component';
import { SheetComponent } from './sheet.component';

/** The plan usage in a sheet (opened from the sessions page's summary).
 * With several Claude accounts it lists every account's windows, each
 * under its name. */
@Component({
  selector: 'mc-plan-usage-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AccountChipComponent, PlanUsageComponent, SheetComponent],
  template: `
    <mc-sheet [open]="open()" title="Plan usage" (closed)="closed.emit()">
      @if (open()) {
        @if (!accounts.multi()) {
          <p class="intro muted small">
            Your Claude plan: every session on this account shares it.
          </p>
          <mc-plan-usage />
        } @else {
          <p class="intro muted small">
            Each Claude account has its own plan, shared by every session that runs under it.
          </p>
          @for (a of accounts.list(); track a.id; let last = $last) {
            <section class="acct" [attr.aria-label]="'Plan usage of ' + a.name">
              <div class="ahead">
                <mc-account-chip [account]="a" />
                <span class="muted small who">{{ signIn(a) }}</span>
              </div>
              @if (plan.usageOf(a.id); as u) {
                @if (!u.available && a.kind === 'token') {
                  <!-- A token can't read its plan: its running sessions report it. -->
                  <p class="muted small none">{{ tokenNoUsage }}</p>
                } @else {
                  <mc-plan-usage [account]="a.id" [settingsLink]="last" />
                }
              } @else {
                <p class="muted small none">
                  {{
                    a.kind === 'token'
                      ? tokenNoUsage
                      : 'No plan usage reported for this account yet.'
                  }}
                </p>
              }
            </section>
          }
        }
      }
    </mc-sheet>
  `,
  styles: [
    `
      .intro {
        margin: 0 0 0.9rem;
      }
      .acct + .acct {
        margin-top: 1.1rem;
        padding-top: 0.9rem;
        border-top: 1px solid var(--border);
      }
      .ahead {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.25rem 0.5rem;
        margin-bottom: 0.65rem;
      }
      .who {
        min-width: 0;
        overflow-wrap: anywhere;
      }
      .none {
        margin: 0;
      }
    `,
  ],
})
export class PlanUsageSheetComponent {
  protected readonly accounts = inject(AccountsService);
  protected readonly plan = inject(PlanUsageService);

  readonly open = input(false);
  readonly closed = output<void>();

  protected readonly tokenNoUsage = TOKEN_NO_USAGE;

  protected signIn(a: Parameters<typeof signInLine>[0]): string {
    return signInLine(a).text;
  }
}
