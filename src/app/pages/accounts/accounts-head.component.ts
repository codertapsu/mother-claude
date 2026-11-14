import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';

import { AuthService } from '../../core/auth.service';
import { TPipe } from '../../core/i18n/t.pipe';
import { HubTabsComponent } from '../../shared/hub-tabs.component';
import { IconComponent } from '../../shared/icon.component';
import { PageHeadComponent } from '../../shared/page-head.component';
import { AccountAddSheetComponent } from './account-add-sheet.component';

/**
 * The head of every Accounts page (docs/UI-REDESIGN.md §6.6): the name,
 * *Add account* on the desktop, one sentence with *How it works ›*, and the
 * tabs — Accounts · Usage · Recent changes. Backups is a place of its own
 * (§11, 2026-11-14).
 */
@Component({
  selector: 'mc-accounts-head',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AccountAddSheetComponent, HubTabsComponent, IconComponent, PageHeadComponent, TPipe],
  template: `
    <mc-page-head
      [heading]="'Accounts' | t"
      [lede]="'Your Claude accounts, how much of each plan is left, and what you can undo.' | t"
      help="accounts"
    >
      @if (isDesktop()) {
        <button class="btn primary" type="button" (click)="adding.set(true)">
          <mc-icon name="plus" [size]="16" /> {{ 'Add account' | t }}
        </button>
      }
    </mc-page-head>
    <mc-hub-tabs place="accounts" />
    @if (isDesktop()) {
      <mc-account-add-sheet [open]="adding()" (closed)="adding.set(false)" />
    }
  `,
  styles: [
    `
      :host {
        display: block;
      }
    `,
  ],
})
export class AccountsHeadComponent {
  protected readonly isDesktop = inject(AuthService).isDesktop;
  protected readonly adding = signal(false);
}
