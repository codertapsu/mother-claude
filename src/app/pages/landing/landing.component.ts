import { Component } from '@angular/core';

@Component({
  selector: 'mc-landing',
  imports: [],
  template: `
    <main class="landing">
      <h1>Mother Claude</h1>
      <p>One dashboard to monitor and control every local Claude Code session.</p>
      <p class="hint">Backend wiring lands in the upcoming commits.</p>
    </main>
  `,
  styles: [
    `
      .landing {
        max-width: 40rem;
        margin: 4rem auto;
        padding: 0 1.5rem;
        font-family:
          system-ui,
          -apple-system,
          sans-serif;
      }
      h1 {
        font-size: 2rem;
        margin-bottom: 0.5rem;
      }
      .hint {
        opacity: 0.6;
        font-size: 0.9rem;
      }
    `,
  ],
})
export class LandingComponent {}
