import { ChangeDetectionStrategy, Component, input } from '@angular/core';
import { RouterLink } from '@angular/router';

import { TPipe } from '../core/i18n/t.pipe';
import { IconComponent, IconName } from './icon.component';

/**
 * What an empty list shows (docs/UI-REDESIGN.md §5): what will show here,
 * and the one thing that makes it show (the content: a button or a link),
 * with *How it works ›* to its Help topic.
 */
@Component({
  selector: 'mc-empty-state',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, RouterLink, TPipe],
  template: `
    <div class="card empty-state" [attr.data-tone]="tone()">
      @if (icon(); as i) {
        <mc-icon class="ico" [name]="i" [size]="28" />
      }
      <strong>{{ heading() }}</strong>
      @if (text()) {
        <p class="text">{{ text() }}</p>
      }
      <div class="acts">
        <ng-content />
        @if (help(); as topic) {
          <a class="how" [routerLink]="['/help', topic]">{{ 'How it works ›' | t }}</a>
        }
      </div>
    </div>
  `,
  styles: [
    `
      :host {
        display: block;
      }
      .ico {
        display: block;
        width: 28px;
        margin: 0 auto 0.5rem;
        color: var(--tone, var(--muted));
      }
      .text {
        margin: 0 auto;
        max-width: 30rem;
      }
      .acts {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        justify-content: center;
        gap: 0.5rem 1rem;
        margin-top: 0.85rem;
      }
      .acts:empty {
        display: none;
      }
    `,
  ],
})
export class EmptyStateComponent {
  /** What would show here, in the language shown ("No phone paired yet"). */
  readonly heading = input.required<string>();
  /** One or two sentences: what makes it show (in the language shown). */
  readonly text = input('');
  readonly icon = input<IconName | null>(null);
  /** The icon's colour (`green` for "all done"). */
  readonly tone = input<'green' | 'amber' | 'red' | 'accent' | null>(null);
  /** Its Help topic: *How it works ›* opens `/help/<topic>`. */
  readonly help = input('');
}
