import { ChangeDetectionStrategy, Component, computed, input, output, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { computerNoun } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { Grants, PairingView, defaultGrants, groupDigits } from '../../core/machines';
import { GrantsMatrixComponent } from './grants-matrix.component';
import { osName, pathName } from './machines-text';

/**
 * The confirmation both computers show (spikes/machines/UX.md §2.3): the
 * six digits to compare, and what the other computer may do on this one.
 * When the other computer joined over the internet, the first computer types
 * the digits it shows instead (the phone pairing's rule) — so they are not
 * shown here then.
 */
@Component({
  selector: 'mc-pair-confirm',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, GrantsMatrixComponent, TPipe],
  template: `
    @let p = pairing();
    @let other = p.other?.name ?? '';
    <p class="who">
      <strong>{{ other }}</strong>
      @if (p.other) {
        <span class="muted"> · {{ os() }} · {{ path() }}</span>
      }
    </p>

    @if (p.mustType) {
      <label class="field">
        <span class="field-label">{{
          '{name} joined over the internet. Type the number it shows:' | t: { name: other }
        }}</span>
        <input
          class="typed mono"
          inputmode="numeric"
          autocomplete="one-time-code"
          maxlength="7"
          [ngModel]="typed()"
          (ngModelChange)="typed.set($event)"
          [attr.aria-describedby]="'pair-type-hint'"
        />
        <span class="field-hint" id="pair-type-hint">{{
          'Six digits, as {name} shows them.' | t: { name: other }
        }}</span>
      </label>
    } @else {
      <p>{{ 'Check that {name} shows the same number:' | t: { name: other } }}</p>
      <p class="digits">
        <span class="visually-hidden">{{
          'The number: {digits}' | t: { digits: p.digits ?? '' }
        }}</span>
        <span aria-hidden="true">{{ grouped() }}</span>
      </p>
    }

    <h3 class="small">
      {{ 'What {name} may do on this {computer}' | t: { name: other, computer: computer() } }}
    </h3>
    <mc-grants-matrix
      [grants]="grants()"
      [name]="other"
      [editable]="true"
      [disabled]="busy()"
      (changed)="grants.set($event)"
    />
    <p class="muted small">{{ 'You can change these later on Computers.' | t }}</p>

    @if (error()) {
      <p class="notice error small" role="alert">{{ error() }}</p>
    }
    <div class="row acts">
      <button
        type="button"
        class="btn primary"
        [disabled]="busy() || (p.mustType && !typedOk())"
        (click)="
          confirmed.emit({ grants: grants(), digits: p.mustType ? typedDigits() : undefined })
        "
      >
        {{ p.mustType ? ('Pair' | t) : ('The numbers match — Pair' | t) }}
      </button>
      <button type="button" class="btn ghost" [disabled]="busy()" (click)="cancelled.emit()">
        {{ 'Cancel' | t }}
      </button>
    </div>
  `,
  styles: [
    `
      .who {
        margin: 0 0 0.6rem;
      }
      .digits {
        font-family: var(--mono);
        font-size: 2rem;
        font-weight: 700;
        letter-spacing: 0.15em;
        text-align: center;
        margin: 0.4rem 0 0.8rem;
      }
      .typed {
        font-size: 1.4rem;
        letter-spacing: 0.2em;
        max-width: 12rem;
      }
      h3 {
        margin: 0.8rem 0 0.3rem;
        font-weight: 600;
      }
      .acts {
        margin-top: 0.8rem;
      }
    `,
  ],
})
export class PairConfirmComponent {
  readonly pairing = input.required<PairingView>();
  readonly busy = input(false);
  readonly error = input('');
  readonly confirmed = output<{ grants: Grants; digits?: string }>();
  readonly cancelled = output<void>();

  protected readonly computer = computerNoun;
  protected readonly grants = signal<Grants>(defaultGrants());
  protected readonly typed = signal('');
  protected readonly typedDigits = computed(() => this.typed().replace(/\D/g, ''));
  protected readonly typedOk = computed(() => this.typedDigits().length === 6);
  protected readonly grouped = computed(() => groupDigits(this.pairing().digits));
  protected readonly os = computed(() => osName(this.pairing().other?.os ?? ''));
  protected readonly path = computed(() => pathName(this.pairing().other?.path));
}
