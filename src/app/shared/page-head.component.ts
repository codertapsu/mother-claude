import {
  ChangeDetectionStrategy,
  Component,
  Directive,
  contentChildren,
  inject,
  input,
  signal,
} from '@angular/core';
import { RouterLink } from '@angular/router';

import { TPipe } from '../core/i18n/t.pipe';
import { IconComponent } from './icon.component';
import { SheetComponent } from './sheet.component';

/** A place's state in the Machines words: fine, works but worse, blocks. */
export interface PageState {
  tone: 'ok' | 'warn' | 'danger';
  /** In the language shown ("All good", "2 required"). */
  text: string;
}

/** The mark each tone shows beside its words. */
const MARKS: Record<PageState['tone'], string> = { ok: '✓', warn: '!', danger: '✕' };

/**
 * One of the actions under a page head's ⋯ (`<button mcPageMenu …>`):
 * choosing it closes the menu.
 */
@Directive({
  selector: '[mcPageMenu]',
  host: { '(click)': 'head.closeMenu()' },
})
export class PageMenuDirective {
  protected readonly head = inject(PageHeadComponent);
}

/**
 * A page's header (docs/UI-REDESIGN.md §5): the place's name, its state as
 * a badge (✓ fine · ! works, but worse · ✕ blocks), its one primary action
 * (the content) and the rest under ⋯ (`mcPageMenu` items, in a sheet), then
 * one sentence saying what the place is for with *How it works ›* to its
 * Help topic. On a phone the top bar names the place, so the heading is
 * there for screen readers only, unless `named` (a detail page names its
 * item).
 */
@Component({
  selector: 'mc-page-head',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, RouterLink, SheetComponent, TPipe],
  template: `
    <div class="page-head" [class.named]="named()">
      <h1>{{ heading() }}</h1>
      @if (state(); as s) {
        <span
          class="badge state"
          [class.ok]="s.tone === 'ok'"
          [class.warn]="s.tone === 'warn'"
          [class.danger]="s.tone === 'danger'"
          ><span aria-hidden="true">{{ mark(s) }}</span> {{ s.text }}</span
        >
      }
      <span class="spacer"></span>
      <ng-content />
      @if (menu().length) {
        <button
          class="btn ghost icon-only"
          type="button"
          aria-haspopup="dialog"
          [attr.aria-label]="'More actions' | t"
          [title]="'More actions' | t"
          (click)="menuOpen.set(true)"
        >
          <mc-icon name="more" />
        </button>
      }
    </div>
    @if (lede() || help()) {
      <p class="lede">
        {{ lede() }}
        @if (help(); as topic) {
          <a class="how" [routerLink]="['/help', topic]">{{ 'How it works ›' | t }}</a>
        }
      </p>
    }
    <mc-sheet [open]="menuOpen()" [title]="heading()" (closed)="closeMenu()">
      <div class="menu"><ng-content select="[mcPageMenu]" /></div>
    </mc-sheet>
  `,
  styles: [
    `
      :host {
        display: block;
      }
      .how {
        white-space: nowrap;
      }
      .menu {
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
      }
    `,
  ],
})
export class PageHeadComponent {
  /** The place's name, in the language shown. */
  readonly heading = input.required<string>();
  /** One sentence: what the place is for (in the language shown). */
  readonly lede = input('');
  /** Its Help topic: *How it works ›* opens `/help/<topic>`. */
  readonly help = input('');
  readonly state = input<PageState | null>(null);
  /** Keep the heading on a phone (a page that names its item). */
  readonly named = input(false);

  protected readonly menu = contentChildren(PageMenuDirective);
  protected readonly menuOpen = signal(false);

  protected mark(s: PageState): string {
    return MARKS[s.tone];
  }

  closeMenu(): void {
    this.menuOpen.set(false);
  }
}
