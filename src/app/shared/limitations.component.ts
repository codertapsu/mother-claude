import { Component } from '@angular/core';

import { IconComponent } from './icon.component';

/** Plain-language "good to know" notes about how the local bridge behaves. */
@Component({
  selector: 'mc-limitations',
  imports: [IconComponent],
  template: `
    <div class="card notes">
      <div class="note">
        <span class="ico" aria-hidden="true"><mc-icon name="monitor" [size]="18" /></span>
        <div>
          <strong>Your laptop is the brain.</strong>
          Mother Claude runs on your laptop and bridges it to your phone. Keep the laptop awake
          (plugged in, sleep disabled) — if it sleeps, your phone loses the connection.
        </div>
      </div>

      <div class="note">
        <span class="ico" aria-hidden="true"><mc-icon name="code" [size]="18" /></span>
        <div>
          <strong>VS Code and terminal sessions update message by message.</strong>
          Their status (working, waiting for you) is live, but their text appears when each message
          finishes rather than word by word. Sessions started here stream live.
        </div>
      </div>

      <div class="note">
        <span class="ico" aria-hidden="true"><mc-icon name="keyboard" [size]="18" /></span>
        <div>
          <strong>One driver at a time.</strong>
          <em>“Continue here”</em> on a session that is still open elsewhere starts a copy, so the
          original is left alone. When you resume an ended session here, drive it from one place:
          typing in VS&nbsp;Code/terminal <em>and</em> here can split the conversation in two.
        </div>
      </div>

      <div class="note">
        <span class="ico" aria-hidden="true"><mc-icon name="wifi" [size]="18" /></span>
        <div>
          <strong>Local first, internet when you choose.</strong>
          Phones pair with a QR code from <strong>Remote</strong> on the desktop. On your own Wi-Fi
          nothing leaves your network. Internet access is off until you turn it on and point a
          tunnel (ngrok, Cloudflare, Tailscale) at it; every device still needs its own pairing, and
          you can revoke any of them at any time.
        </div>
      </div>
    </div>
  `,
  styles: [
    `
      .notes {
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
      }
      .note {
        display: flex;
        gap: 0.6rem;
        line-height: 1.45;
        font-size: 0.9rem;
      }
      .ico {
        flex: 0 0 auto;
        color: var(--accent-text);
        padding-top: 0.1rem;
      }
      .note strong {
        color: var(--text);
      }
    `,
  ],
})
export class LimitationsComponent {}
