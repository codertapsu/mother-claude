import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  Injector,
  afterNextRender,
  computed,
  inject,
  input,
  linkedSignal,
  output,
  signal,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';

import { computerNoun, tr } from '../core/i18n/i18n';
import { TPipe } from '../core/i18n/t.pipe';
import { GrantOffer, GrantScope, RememberChoice } from '../core/models';
import {
  MAX_PATTERNS,
  SCOPES,
  cleanPatterns,
  needsDangerous,
  patternExample,
  patternHint,
  patternsHeading,
  quickChoices,
  scopeLabel,
} from '../core/permission-rules';
import { HelpTextComponent } from '../pages/help/help-text.component';
import { IconComponent } from './icon.component';

/** Each panel's radio group has its own name. */
let panels = 0;

/**
 * The approval card's *Allow for…* (docs: Help › Permissions): allow the
 * request now and remember the answer, so only what lies outside it asks
 * again. One tap for this session, its project (every session) or the
 * whole computer, worded for what is asked — a Bash command, a file edit,
 * a command on another computer (`quickChoices`); or *Only matching…*:
 * patterns first (prefilled from the offer's `suggested`, a row each, up
 * to 20), a scope, *Allow and remember*. A project or the whole computer
 * needs the dangerous right: without it those choices are disabled, with
 * where to answer instead. Claude Code's own suggestions, when the request
 * has any, come in as content. Escape goes back (to the choices, then
 * out); the first choice takes the focus.
 *
 * Inputs: `offer` (the item's `grant`), `canDangerous`, `disabled` (an
 * answer in flight, offline), `host` (another computer the request waits
 * at: its name). Outputs: `chose` (`{scope, patterns?}`), `cancelled`.
 */
@Component({
  selector: 'mc-remember-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, RouterLink, HelpTextComponent, IconComponent, TPipe],
  host: { '(keydown.escape)': 'back($event)' },
  template: `
    @if (mode() === 'choices') {
      <p class="muted small lead">{{ 'Allow now, and remember it:' | t }}</p>
      <div class="choices" role="group" [attr.aria-label]="'Allow for…' | t">
        @for (c of choices(); track c.scope) {
          <button
            type="button"
            class="choice"
            [attr.data-scope]="c.scope"
            [disabled]="disabled() || c.locked"
            (click)="pick(c.scope)"
          >
            <span class="clabel">{{ c.label }}</span>
            <span class="chint">{{ c.hint }}</span>
          </button>
        }
      </div>
      @if (lockedText()) {
        <p class="muted small why">{{ lockedText() }}</p>
      }
      @if (offer().patterns) {
        <button
          type="button"
          class="btn ghost small narrow"
          [disabled]="disabled()"
          (click)="openPatterns()"
        >
          <mc-icon name="edit" [size]="14" /> {{ 'Only matching…' | t }}
        </button>
      }
      <ng-content />
    } @else {
      <form class="patterns" (ngSubmit)="submit()">
        <p class="ptitle">{{ heading() }}</p>
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
          class="btn ghost small narrow"
          [disabled]="rows().length >= max"
          (click)="addRow()"
        >
          <mc-icon name="plus" [size]="14" /> {{ 'Add a pattern' | t }}
        </button>
        <p class="muted small hint"><mc-help-text [text]="hint()" /></p>
        <fieldset class="scopes">
          <legend class="field-label">{{ 'Where it applies' | t }}</legend>
          @for (c of scopes(); track c.scope) {
            <label class="radio" [class.locked]="c.locked">
              <input
                type="radio"
                [name]="group"
                [value]="c.scope"
                [checked]="scope() === c.scope"
                [disabled]="c.locked"
                (change)="scope.set(c.scope)"
              />
              {{ c.label }}
            </label>
          }
        </fieldset>
        @if (lockedText()) {
          <p class="muted small why">{{ lockedText() }}</p>
        }
        <div class="actions">
          <button
            type="submit"
            class="btn primary"
            [disabled]="disabled() || !patterns().length || scopeLocked()"
          >
            <mc-icon name="check" [size]="15" /> {{ 'Allow and remember' | t }}
          </button>
          <button type="button" class="btn ghost" (click)="mode.set('choices')">
            {{ 'Back' | t }}
          </button>
        </div>
      </form>
    }
    <p class="foot small">
      <mc-icon name="shield" [size]="14" />
      <span>
        {{ 'Dangerous actions always ask.' | t }}
        @if (host(); as name) {
          {{ 'Change it later in Permissions on {name}.' | t: { name } }}
        } @else {
          <a routerLink="/permissions">{{ 'Change later in Permissions' | t }}</a>
        }
      </span>
    </p>
    @if (mode() === 'choices') {
      <div class="actions">
        <button type="button" class="btn ghost" (click)="cancelled.emit()">
          {{ 'Cancel' | t }}
        </button>
      </div>
    }
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
        margin-top: 0.65rem;
      }
      .lead,
      .why,
      .hint,
      .ptitle,
      .foot {
        margin: 0;
      }
      .ptitle {
        font-weight: 500;
        font-size: 0.875rem;
      }
      .choices {
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
      }
      .choice {
        appearance: none;
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        gap: 0.15rem;
        text-align: left;
        font: inherit;
        color: var(--text);
        background: var(--panel-2);
        border: 1px solid var(--border-strong);
        border-radius: var(--radius-sm);
        padding: 0.55rem 0.7rem;
        min-height: var(--tap);
        cursor: pointer;
      }
      .choice:hover:not(:disabled) {
        border-color: var(--accent);
      }
      .choice:focus-visible {
        outline: 2px solid var(--focus);
        outline-offset: 2px;
      }
      .choice:disabled {
        cursor: default;
        opacity: 0.6;
      }
      .clabel {
        font-weight: 500;
        overflow-wrap: anywhere;
      }
      .chint {
        font-size: 0.8rem;
        color: var(--muted);
        overflow-wrap: anywhere;
      }
      .narrow {
        align-self: flex-start;
      }
      .patterns {
        display: flex;
        flex-direction: column;
        gap: 0.45rem;
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
      .scopes {
        border: 0;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 0.1rem;
        min-width: 0;
      }
      .scopes legend {
        padding: 0;
        margin-bottom: 0.2rem;
      }
      .radio {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        min-height: 32px;
        font-size: 0.875rem;
        overflow-wrap: anywhere;
      }
      .radio.locked {
        color: var(--muted);
      }
      .foot {
        display: flex;
        align-items: flex-start;
        gap: 0.4rem;
        color: var(--muted);
      }
      .foot mc-icon {
        flex: 0 0 auto;
        margin-top: 0.1rem;
      }
      .foot span {
        flex: 1;
        min-width: 0;
      }
      .actions {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0.45rem;
      }
    `,
  ],
})
export class RememberPanelComponent {
  private readonly el = inject(ElementRef<HTMLElement>).nativeElement as HTMLElement;
  private readonly injector = inject(Injector);

  /** What Mother Claude can remember (the item's `grant`, not blocked). */
  readonly offer = input.required<GrantOffer>();
  /** This viewer may keep it for a project or the whole computer. */
  readonly canDangerous = input(false);
  /** Nothing can be chosen now (an answer in flight, offline). */
  readonly disabled = input(false);
  /** Another computer the request waits at (its name); null: this one. */
  readonly host = input<string | null>(null);
  readonly chose = output<RememberChoice>();
  readonly cancelled = output<void>();

  protected readonly max = MAX_PATTERNS;
  protected readonly group = `remember-scope-${++panels}`;
  protected readonly mode = signal<'choices' | 'patterns'>('choices');
  /** The pattern rows, from the offer's suggestions (one empty row else). */
  protected readonly rows = linkedSignal<string[]>(() => {
    const s = this.offer().suggested ?? [];
    return s.length ? s.slice(0, MAX_PATTERNS) : [''];
  });
  /** The scope of *Only matching…*: the project when it may be, else this
   * session. */
  protected readonly scope = linkedSignal<GrantScope>(() =>
    this.offer().project && this.canDangerous() ? 'project' : 'session',
  );

  private readonly locked = (scope: GrantScope) => needsDangerous(scope) && !this.canDangerous();

  protected readonly choices = computed(() =>
    quickChoices(this.offer(), this.host()).map((c) => ({ ...c, locked: this.locked(c.scope) })),
  );
  protected readonly scopes = computed(() => {
    const o = this.offer();
    return SCOPES.filter((s) => s !== 'project' || !!o.project).map((scope) => ({
      scope,
      label: scopeLabel(o.kind, scope, { project: o.projectName, host: this.host() }),
      locked: this.locked(scope),
    }));
  });
  /** Why a project or the whole computer can't be chosen here, and where
   * they can ('' when they can). */
  protected readonly lockedText = computed(() => {
    if (this.canDangerous()) return '';
    const name = this.host();
    return name
      ? tr(
          'To remember it for a project or everywhere, answer at {name}, or on a computer it allows dangerous actions.',
          { name },
        )
      : tr(
          'To remember it for a project or everywhere, answer on the {computer}, or let this device take dangerous actions (Settings › Phone & devices).',
          { computer: computerNoun() },
        );
  });
  protected readonly patterns = computed(() => cleanPatterns(this.rows()));
  protected readonly scopeLocked = computed(() => this.locked(this.scope()));
  protected readonly heading = computed(() => patternsHeading(this.offer().patterns ?? 'command'));
  protected readonly hint = computed(() => patternHint(this.offer().patterns ?? 'command'));
  protected readonly example = computed(() => patternExample(this.offer().patterns ?? 'command'));

  constructor() {
    afterNextRender(() => this.focus('.choice:not(:disabled)'));
  }

  protected pick(scope: GrantScope): void {
    if (this.disabled() || this.locked(scope)) return;
    this.chose.emit({ scope });
  }

  protected openPatterns(): void {
    this.mode.set('patterns');
    afterNextRender(() => this.focus('.prow input'), { injector: this.injector });
  }

  protected setRow(i: number, value: string): void {
    this.rows.update((list) => list.map((p, j) => (j === i ? value : p)));
  }

  protected addRow(): void {
    if (this.rows().length >= MAX_PATTERNS) return;
    this.rows.update((list) => [...list, '']);
    afterNextRender(() => this.focus('.prow:last-child input'), { injector: this.injector });
  }

  protected removeRow(i: number): void {
    if (this.rows().length === 1) return;
    this.rows.update((list) => list.filter((_, j) => j !== i));
  }

  protected submit(): void {
    const patterns = this.patterns();
    if (this.disabled() || !patterns.length || this.scopeLocked()) return;
    this.chose.emit({ scope: this.scope(), patterns });
  }

  /** Escape: back to the choices, then out. */
  protected back(e: Event): void {
    e.preventDefault();
    e.stopPropagation();
    if (this.mode() === 'patterns') {
      this.mode.set('choices');
      afterNextRender(() => this.focus('.choice:not(:disabled)'), { injector: this.injector });
    } else {
      this.cancelled.emit();
    }
  }

  private focus(selector: string): void {
    this.el.querySelector<HTMLElement>(selector)?.focus();
  }
}
