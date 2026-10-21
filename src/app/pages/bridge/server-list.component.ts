import { ChangeDetectionStrategy, Component, computed, inject, input, output } from '@angular/core';

import { MAIN_SERVER, ServerAccount, ServerStatus, serverAccount } from '../../core/api-servers';
import { AccountsService } from '../../core/accounts.service';
import { AccountRef } from '../../core/models';
import { AccountChipComponent } from '../../shared/account-chip.component';
import { IconComponent } from '../../shared/icon.component';

interface Row {
  s: ServerStatus;
  account: ServerAccount;
  /** Its account's chip, when it names one that exists. */
  chip: Pick<AccountRef, 'name' | 'color' | 'kind'> | null;
  /** Running and open to tunnels: no token from anywhere. */
  open: boolean;
}

/**
 * The API servers (docs/ACCOUNTS.md §13.6 item 5), main first: each with its
 * state, port, account (and "only this account") and the tunnel that points
 * at it; picking one shows its panel. Shown only while a server besides the
 * main one exists.
 */
@Component({
  selector: 'mc-api-server-list',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AccountChipComponent, IconComponent],
  template: `
    <section class="card servers" aria-label="API servers">
      @for (r of rows(); track r.s.id) {
        <button
          type="button"
          class="srv"
          [class.on]="r.s.id === selected()"
          [attr.aria-pressed]="r.s.id === selected()"
          (click)="picked.emit(r.s.id)"
        >
          <span class="dot" [class.live]="r.s.running" aria-hidden="true"></span>
          <span class="smain">
            <span class="sline">
              <strong class="sname">{{ r.s.name }}</strong>
              <span class="badge" [class.working]="r.s.running">{{
                r.s.running ? 'running' : 'stopped'
              }}</span>
              @if (r.open) {
                <span class="badge warn">open to tunnels</span>
              }
            </span>
            <span class="meta muted small">
              @if (r.s.port) {
                <span>port {{ r.s.port }}</span>
                <span class="sep" aria-hidden="true">·</span>
              }
              @if (r.chip) {
                <mc-account-chip [account]="r.chip" />
              } @else if (r.account.missing) {
                <span class="warn-text">its account was removed</span>
              } @else if (r.account.id) {
                <span>{{ r.account.name }}</span>
              } @else {
                <span>automatic account</span>
              }
              @if (r.account.only) {
                <span class="sep" aria-hidden="true">·</span>
                <span>only this account</span>
              }
            </span>
            @if (r.s.exposedByTunnel) {
              <span class="tunnel small">
                <mc-icon name="globe" [size]="13" />
                <span
                  >A tunnel points at it{{ r.s.tunnelUrl ? ': ' : ''
                  }}<span class="mono">{{ r.s.tunnelUrl }}</span></span
                >
              </span>
            }
          </span>
        </button>
      }
    </section>
  `,
  styles: [
    `
      .servers {
        display: flex;
        flex-direction: column;
        padding: 0.35rem;
      }
      .srv {
        appearance: none;
        display: flex;
        align-items: flex-start;
        gap: 0.65rem;
        width: 100%;
        min-height: var(--tap);
        padding: 0.6rem 0.65rem;
        border: 1px solid transparent;
        border-radius: var(--radius-sm);
        background: transparent;
        color: var(--text);
        font: inherit;
        text-align: left;
        cursor: pointer;
      }
      .srv + .srv {
        margin-top: 2px;
      }
      .srv:hover {
        background: var(--panel-2);
      }
      .srv.on {
        border-color: var(--accent);
        background: var(--accent-soft);
      }
      .dot {
        flex: 0 0 auto;
        width: 0.6rem;
        height: 0.6rem;
        margin-top: 0.4rem;
        border-radius: 50%;
        background: var(--faint);
      }
      .dot.live {
        background: var(--green);
      }
      .smain {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.2rem;
      }
      .sline {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.4rem;
      }
      .sname {
        overflow-wrap: anywhere;
      }
      .meta {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.2rem 0.4rem;
      }
      .sep {
        color: var(--faint);
      }
      .warn-text {
        color: var(--amber);
      }
      .tunnel {
        display: flex;
        align-items: flex-start;
        gap: 0.3rem;
        color: var(--amber);
        overflow-wrap: anywhere;
        min-width: 0;
      }
      .tunnel mc-icon {
        margin-top: 0.1rem;
      }
      .tunnel .mono {
        color: var(--text);
      }
    `,
  ],
})
export class ApiServerListComponent {
  private accounts = inject(AccountsService);

  readonly servers = input.required<ServerStatus[]>();
  /** The server whose panel shows. */
  readonly selected = input<string>(MAIN_SERVER);
  readonly picked = output<string>();

  protected readonly rows = computed<Row[]>(() =>
    this.servers().map((s) => {
      const account = serverAccount(s, (id) => this.accounts.byId(id)?.name);
      const a = account.id && !account.missing ? this.accounts.byId(account.id) : undefined;
      return {
        s,
        account,
        chip: a ? { name: a.name, color: a.color, kind: a.kind } : null,
        open: s.running && s.tokenPolicy === 'never',
      };
    }),
  );
}
