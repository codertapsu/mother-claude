import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  Injector,
  afterNextRender,
  computed,
  inject,
  input,
  model,
} from '@angular/core';
import { FormsModule } from '@angular/forms';

import { TPipe } from '../core/i18n/t.pipe';
import { PatternKind } from '../core/models';
import { MAX_PATTERNS, patternExample, patternHint } from '../core/permission-rules';
import { HelpTextComponent } from '../pages/help/help-text.component';
import { IconComponent } from './icon.component';

/**
 * A permission's patterns, a row each (*Only matching…* on a card, *Edit*
 * on the Permissions page): a text field per pattern, × removes one (never
 * the last), *Add a pattern* adds one (up to 20) and takes the focus, and
 * one line says what a pattern of this `kind` means. `rows` is two-way
 * (`[(rows)]`); the rows are kept as typed — `cleanPatterns` trims them.
 */
@Component({
  selector: 'mc-pattern-rows',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, HelpTextComponent, IconComponent, TPipe],
  template: `
    <ul class="rows">
      @for (p of rows(); track $index; let i = $index) {
        <li class="prow">
          <input
            class="mono"
            type="text"
            autocomplete="off"
            autocapitalize="off"
            spellcheck="false"
            [ngModel]="p"
            (ngModelChange)="setRow(i, $event)"
            [ngModelOptions]="{ standalone: true }"
            [placeholder]="example()"
            [attr.aria-label]="'Pattern {n}' | t: { n: i + 1 }"
          />
          <button
            type="button"
            class="btn ghost small icon-only"
            [disabled]="rows().length === 1"
            [attr.aria-label]="'Remove pattern {n}' | t: { n: i + 1 }"
            [title]="'Remove' | t"
            (click)="removeRow(i)"
          >
            <mc-icon name="x" [size]="14" />
          </button>
        </li>
      }
    </ul>
    <button
      type="button"
      class="btn ghost small add"
      [disabled]="rows().length >= max"
      (click)="addRow()"
    >
      <mc-icon name="plus" [size]="14" /> {{ 'Add a pattern' | t }}
    </button>
    <p class="muted small hint"><mc-help-text [text]="hint()" /></p>
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.45rem;
        min-width: 0;
      }
      .rows {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 0.35rem;
      }
      .prow {
        display: flex;
        align-items: center;
        gap: 0.35rem;
      }
      .prow input {
        flex: 1;
        min-width: 0;
      }
      .add {
        align-self: flex-start;
      }
      .hint {
        margin: 0;
      }
    `,
  ],
})
export class PatternRowsComponent {
  private readonly el = inject(ElementRef<HTMLElement>).nativeElement as HTMLElement;
  private readonly injector = inject(Injector);

  readonly rows = model.required<string[]>();
  readonly kind = input.required<PatternKind>();

  protected readonly max = MAX_PATTERNS;
  protected readonly hint = computed(() => patternHint(this.kind()));
  protected readonly example = computed(() => patternExample(this.kind()));

  protected setRow(i: number, value: string): void {
    this.rows.update((list) => list.map((p, j) => (j === i ? value : p)));
  }

  protected addRow(): void {
    if (this.rows().length >= MAX_PATTERNS) return;
    this.rows.update((list) => [...list, '']);
    afterNextRender(
      () => this.el.querySelector<HTMLInputElement>('.prow:last-child input')?.focus(),
      { injector: this.injector },
    );
  }

  protected removeRow(i: number): void {
    if (this.rows().length === 1) return;
    this.rows.update((list) => list.filter((_, j) => j !== i));
  }
}
