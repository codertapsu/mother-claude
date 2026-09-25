import { Component } from '@angular/core';

/** Plain-language "good to know" notes about how the local bridge behaves. */
@Component({
  selector: 'mc-limitations',
  imports: [],
  template: `
    <div class="card notes">
      <div class="note">
        <span class="ico">💻</span>
        <div>
          <strong>Your laptop is the brain.</strong>
          Mother Claude runs on your laptop and bridges it to your phone. Keep the laptop awake
          (plugged in, sleep disabled) — if it sleeps, your phone loses the connection.
        </div>
      </div>

      <div class="note">
        <span class="ico">🧩</span>
        <div>
          <strong>VS Code isn't synced live.</strong>
          The dashboard reads the transcript files Claude writes. A live VS&nbsp;Code session may
          not save its newest messages until it's idle or closed, so they might not show here yet —
          and
          <em>“Continue here”</em> picks up from the last saved point. To capture the very latest,
          <strong>close and reopen VS&nbsp;Code (or its Claude panel)</strong> first.
        </div>
      </div>

      <div class="note">
        <span class="ico">🎮</span>
        <div>
          <strong>One driver at a time.</strong>
          After taking a session over, drive it from one place. If you keep typing in the original
          VS&nbsp;Code/terminal <em>and</em> here, the conversation can split into two branches.
        </div>
      </div>

      <div class="note">
        <span class="ico">📶</span>
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
        font-size: 1.1rem;
        line-height: 1.3;
        flex: 0 0 auto;
      }
      .note strong {
        color: var(--text);
      }
    `,
  ],
})
export class LimitationsComponent {}
