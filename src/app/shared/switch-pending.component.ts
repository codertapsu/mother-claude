import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';

import { AccountsService } from '../core/accounts.service';
import { ApiService } from '../core/api.service';
import { SessionSummary } from '../core/models';
import { NoticeService } from '../core/notice.service';
import { switchErrorText, switchingToOf } from './switch-account';

/**
 * A switch of account waiting for the end of the session's turn
 * (docs/ACCOUNTS.md §15.1): "Switching to Work once this turn ends", and
 * Cancel. Nothing while none waits.
 */
@Component({
  selector: 'mc-switch-pending',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (name(); as to) {
      <div class="pend" role="status">
        <span class="spin" aria-hidden="true"></span>
        <span class="txt"
          >Switching to <strong>{{ to }}</strong> once this turn ends</span
        >
        <button class="btn ghost small" type="button" [disabled]="busy()" (click)="cancel()">
          Cancel
        </button>
      </div>
    }
  `,
  styles: [
    `
      :host {
        display: block;
      }
      .pend {
        margin: 0 0.25rem 0.35rem;
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.3rem 0.5rem;
        padding: 0.35rem 0.6rem;
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        background: var(--panel-2);
        font-size: 0.85rem;
      }
      .txt {
        flex: 1 1 12rem;
        min-width: 0;
      }
      .spin {
        flex: 0 0 auto;
        width: 0.8rem;
        height: 0.8rem;
        border: 2px solid var(--border-strong);
        border-top-color: var(--accent);
        border-radius: 50%;
        animation: mc-spin 0.8s linear infinite;
      }
    `,
  ],
})
export class SwitchPendingComponent {
  private api = inject(ApiService);
  private accounts = inject(AccountsService);
  private notices = inject(NoticeService);

  readonly session = input.required<SessionSummary>();

  protected readonly busy = signal(false);
  protected readonly name = computed(() => {
    const id = switchingToOf(this.session());
    return id ? (this.accounts.byId(id)?.name ?? 'another account') : '';
  });

  protected async cancel(): Promise<void> {
    if (this.busy()) return;
    this.busy.set(true);
    try {
      await this.api.cancelSessionSwitch(this.session().id);
      this.notices.info('The switch is cancelled: it stays on its account.');
    } catch (e) {
      this.notices.show('error', switchErrorText(e));
    } finally {
      this.busy.set(false);
    }
  }
}
