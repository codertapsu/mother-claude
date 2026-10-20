import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { isAccountColor } from '../core/accounts';
import { AccountRef } from '../core/models';

/**
 * The Claude account a session runs under: its colour and its name, e.g.
 * `<mc-account-chip [account]="s.account" />`. Sessions carry an account
 * only while more than the Default exists, so callers show it only then.
 */
@Component({
  selector: 'mc-account-chip',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.title]': 'title()' },
  template: `
    <span class="dot" aria-hidden="true" [style.background]="color()"></span>
    <span class="txt"><span class="visually-hidden">Account </span>{{ account().name }}</span>
  `,
  styles: [
    `
      :host {
        display: inline-flex;
        align-items: center;
        gap: 0.3rem;
        max-width: 100%;
        min-width: 0;
        font-size: 0.75rem;
        line-height: 1.4;
        color: var(--text);
        background: var(--panel-2);
        border: 1px solid var(--border);
        border-radius: 0.4rem;
        padding: 0.08rem 0.45rem;
        white-space: nowrap;
      }
      .dot {
        flex: 0 0 auto;
        width: 0.55rem;
        height: 0.55rem;
        border-radius: 50%;
        box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.18);
      }
      .txt {
        overflow: hidden;
        text-overflow: ellipsis;
        min-width: 0;
        max-width: 11rem;
      }
    `,
  ],
})
export class AccountChipComponent {
  readonly account = input.required<Pick<AccountRef, 'name' | 'color' | 'kind'>>();

  protected readonly color = computed(() => {
    const c = this.account().color;
    return isAccountColor(c) ? c : 'var(--accent)';
  });
  protected readonly title = computed(() => {
    const a = this.account();
    return `Claude account: ${a.name}${a.kind === 'token' ? ' (token)' : ''}`;
  });
}
