import { Component } from '@angular/core';

import { PairingComponent } from '../../shared/pairing.component';

/** Dedicated "connect a device" screen — friendly intro + the pairing UI. */
@Component({
  selector: 'mc-pair',
  imports: [PairingComponent],
  template: `
    <h1>Pair a phone</h1>
    <p class="muted lead">
      Connect your phone on the <strong>same Wi-Fi</strong>, then scan the QR code below. You'll keep
      full control of your sessions from the phone — handy when you step away from your laptop.
    </p>
    <mc-pairing />
  `,
  styles: [
    `
      :host {
        display: block;
      }
      .lead {
        line-height: 1.5;
        margin-bottom: 1rem;
      }
    `,
  ],
})
export class PairComponent {}
