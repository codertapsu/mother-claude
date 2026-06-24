import { Component } from '@angular/core';

import { LimitationsComponent } from '../../shared/limitations.component';

/** Standalone "Good to know" screen (also reachable from the header). */
@Component({
  selector: 'mc-good-to-know',
  imports: [LimitationsComponent],
  template: `
    <h1>Good to know</h1>
    <p class="muted lead">A few things about how Mother Claude bridges your laptop to your phone.</p>
    <mc-limitations />
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
export class GoodToKnowComponent {}
