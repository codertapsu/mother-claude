import { ChangeDetectionStrategy, Component, input } from '@angular/core';

import { ServerAddress } from '../../core/api-servers';
import { CopyButtonComponent } from '../../shared/copy-button.component';

/**
 * Where an API server answers (docs/ACCOUNTS.md §13.5 "Reaching them"),
 * best first for whoever is looking — this Mac on the desktop, the network
 * and tunnels on a phone — each with who may call it and a copy button.
 */
@Component({
  selector: 'mc-api-server-addresses',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CopyButtonComponent],
  template: `
    <div class="card list addrs">
      @for (a of addresses(); track a.key) {
        <div class="addr">
          <div class="amain">
            <span class="alabel">{{ a.label }}</span>
            @if (a.url) {
              <span class="mono aurl">{{ a.url }}</span>
            } @else {
              <span class="mono aurl muted">{{ a.pattern }}</span>
            }
            <span class="muted small">{{ a.hint }}</span>
          </div>
          @if (a.url) {
            <mc-copy-button [text]="a.url" />
          }
        </div>
      }
    </div>
  `,
  styles: [
    `
      .addrs {
        padding-top: 0;
        padding-bottom: 0;
      }
      .addr {
        display: flex;
        align-items: center;
        gap: 0.75rem;
        padding: 0.65rem 0;
      }
      .amain {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
      }
      .alabel {
        font-size: 0.75rem;
        font-weight: 600;
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.04em;
      }
      .aurl {
        overflow-wrap: anywhere;
      }
      mc-copy-button {
        flex: 0 0 auto;
      }
    `,
  ],
})
export class ApiServerAddressesComponent {
  readonly addresses = input.required<ServerAddress[]>();
}
