import { ChangeDetectionStrategy, Component, computed, input, output } from '@angular/core';

import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { GRANT_NAMES, GrantName, GrantPair, Grants, setGrant } from '../../core/machines';
import { grantRow } from './machines-text';

/**
 * What another computer may do on this one: the five grants, each on the
 * same network and over the internet (docs/PLAN-MACHINES.md §5.4). Editable
 * at pairing and on a computer's card; it keeps the dependencies the server
 * enforces (driving needs seeing; moving, running and dangerous actions
 * need driving). Running is a later phase: shown, never ticked.
 */
@Component({
  selector: 'mc-grants-matrix',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [TPipe],
  template: `
    <table class="matrix">
      <caption class="visually-hidden">
        {{
          'What {name} may do on this {computer}' | t: { name: name(), computer: computer() }
        }}
      </caption>
      <thead>
        <tr>
          <th scope="col">
            <span class="visually-hidden">{{ 'Permission' | t }}</span>
          </th>
          <th scope="col">{{ 'Same network' | t }}</th>
          <th scope="col">{{ 'Internet' | t }}</th>
        </tr>
      </thead>
      <tbody>
        @for (g of names; track g) {
          <tr [class.risky]="g === 'dangerous'">
            <th scope="row">
              <span class="rowhead">
                <span>{{ rowLabel(g) }}</span>
                @if (g === 'run') {
                  <span class="muted small">{{ '(later)' | t }}</span>
                }
                @if (g === 'dangerous') {
                  <span class="badge danger">{{ 'risky' | t }}</span>
                }
              </span>
            </th>
            @for (col of columns; track col) {
              <td>
                @if (editable()) {
                  <input
                    type="checkbox"
                    [checked]="grants()[g][col]"
                    [disabled]="g === 'run' || disabled()"
                    [attr.aria-label]="cellLabel(g, col)"
                    (change)="toggle(g, col, $any($event.target).checked)"
                  />
                } @else {
                  <span [attr.aria-label]="cellValue(g, col)">{{
                    grants()[g][col] ? '✓' : '–'
                  }}</span>
                }
              </td>
            }
          </tr>
          @if (g === 'dangerous' && dangerousOn()) {
            <tr class="explain">
              <td colspan="3" class="small muted">
                {{
                  "Lets {name} bypass permissions, approve dangerous tool calls, send raw terminal keys, and carry a folder's trust and files git ignores with a move, on this {computer}."
                    | t: { name: name(), computer: computer() }
                }}
              </td>
            </tr>
          }
        }
      </tbody>
    </table>
  `,
  styles: [
    `
      .matrix {
        width: 100%;
        border-collapse: collapse;
        font-size: 0.9rem;
      }
      th,
      td {
        padding: 0.3rem 0.4rem;
        text-align: center;
        border-bottom: 1px solid var(--border);
      }
      thead th {
        font-size: 0.75rem;
        font-weight: 600;
        color: var(--muted);
      }
      tbody th {
        text-align: left;
        font-weight: 400;
      }
      .rowhead {
        display: inline-flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0.35rem;
      }
      tr.explain td {
        text-align: left;
        border-bottom: 0;
      }
      tr.risky .rowhead > span:first-child {
        color: var(--red);
      }
      input[type='checkbox'] {
        width: 1.1rem;
        height: 1.1rem;
      }
    `,
  ],
})
export class GrantsMatrixComponent {
  readonly grants = input.required<Grants>();
  /** The other computer's name. */
  readonly name = input('');
  readonly editable = input(false);
  readonly disabled = input(false);
  readonly changed = output<Grants>();

  protected readonly names = GRANT_NAMES;
  protected readonly columns: (keyof GrantPair)[] = ['lan', 'internet'];
  protected readonly computer = computerNoun;
  protected readonly dangerousOn = computed(
    () => this.grants().dangerous.lan || this.grants().dangerous.internet,
  );

  protected rowLabel(g: GrantName): string {
    return grantRow(g);
  }

  /** A checkbox's name: the grant and its column. */
  protected cellLabel(g: GrantName, col: keyof GrantPair): string {
    return tr('{grant}, {where}', {
      grant: grantRow(g),
      where: col === 'lan' ? tr('Same network') : tr('Internet'),
    });
  }

  /** A read-only cell for a screen reader: the grant, its column, yes or no. */
  protected cellValue(g: GrantName, col: keyof GrantPair): string {
    return tr('{grant}, {where}: {value}', {
      grant: grantRow(g),
      where: col === 'lan' ? tr('Same network') : tr('Internet'),
      value: this.grants()[g][col] ? tr('yes') : tr('no'),
    });
  }

  protected toggle(g: GrantName, col: keyof GrantPair, on: boolean): void {
    this.changed.emit(setGrant(this.grants(), g, col, on));
  }
}
