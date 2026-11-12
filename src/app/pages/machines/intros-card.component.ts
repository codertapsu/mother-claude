import { ChangeDetectionStrategy, Component, inject, input, output, signal } from '@angular/core';

import { ApiService, describeError } from '../../core/api.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { Introduction, MachinesView } from '../../core/machines';
import { osName, serverText } from './machines-text';

/**
 * Introductions this computer received (docs/PLAN-MACHINES.md §5.12): one
 * of your computers says another one exists — *Pair with it?* An
 * introduction alone pairs nothing: accepting starts the normal pairing
 * with that computer (the same six digits on both screens, what each may
 * do), and it is paired only once both desktops confirmed. *Not now* tells
 * the other computer. A phone sees them, never answers.
 */
@Component({
  selector: 'mc-machine-intros',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [TPipe],
  template: `
    <section class="card intros" [attr.aria-label]="'Introductions' | t">
      <h2 class="first">{{ 'Introductions' | t }}</h2>
      @if (error()) {
        <p class="notice error small" role="alert">{{ error() }}</p>
      }
      <ul class="list">
        @for (i of intros(); track i.id) {
          <li [attr.data-state]="i.state">
            <div class="row">
              <strong>{{ i.other.name }}</strong>
              <span class="muted small">· {{ os(i) }} · {{ i.other.key }}</span>
            </div>
            <span class="small">{{
              '{from} introduces {name}: pair this {computer} with it?'
                | t: { from: i.fromName, name: i.other.name, computer: computer() }
            }}</span>
            @switch (i.state) {
              @case ('offered') {
                @if (!readonly()) {
                  <div class="row acts">
                    <button
                      type="button"
                      class="btn primary small"
                      [disabled]="busy() === i.id"
                      (click)="answer(i, true)"
                    >
                      {{ 'Pair with {name}…' | t: { name: i.other.name } }}
                    </button>
                    <button
                      type="button"
                      class="btn ghost small"
                      [disabled]="busy() === i.id"
                      (click)="answer(i, false)"
                    >
                      {{ 'Not now' | t }}
                    </button>
                  </div>
                }
              }
              @case ('accepted') {
                @if (i.pairing) {
                  <span class="muted small">{{ 'Pairing…' | t }}</span>
                  @if (!readonly()) {
                    <div class="row acts">
                      <button type="button" class="btn small" (click)="opened.emit(i)">
                        {{ 'Show the pairing' | t }}
                      </button>
                    </div>
                  }
                } @else if (!readonly()) {
                  <span class="muted small">{{ 'Not paired yet.' | t }}</span>
                  <div class="row acts">
                    <button
                      type="button"
                      class="btn small"
                      [disabled]="busy() === i.id"
                      (click)="answer(i, true)"
                    >
                      {{ 'Try again' | t }}
                    </button>
                    <button
                      type="button"
                      class="btn ghost small"
                      [disabled]="busy() === i.id"
                      (click)="answer(i, false)"
                    >
                      {{ 'Not now' | t }}
                    </button>
                  </div>
                }
              }
              @default {
                <span class="muted small state">{{ closedText(i) }}</span>
              }
            }
          </li>
        }
      </ul>
      <p class="muted small">
        {{
          'An introduction pairs nothing on its own: both computers show the same six digits, and each chooses what the other may do.'
            | t
        }}
      </p>
    </section>
  `,
  styles: [
    `
      .list {
        list-style: none;
        margin: 0.3rem 0;
        padding: 0;
      }
      .list > li {
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
        padding: 0.4rem 0;
        border-bottom: 1px solid var(--border);
      }
      .list > li:last-child {
        border-bottom: 0;
      }
      .acts {
        margin-top: 0.2rem;
      }
    `,
  ],
})
export class IntrosCardComponent {
  readonly intros = input.required<readonly Introduction[]>();
  readonly readonly = input(false);
  /** The view after an answer. */
  readonly answered = output<MachinesView>();
  /** Show the pairing an accepted introduction runs. */
  readonly opened = output<Introduction>();

  private readonly api = inject(ApiService);
  protected readonly computer = computerNoun;
  protected readonly busy = signal<string | null>(null);
  protected readonly error = signal('');

  protected os(i: Introduction): string {
    return osName(i.other.os);
  }

  protected closedText(i: Introduction): string {
    switch (i.state) {
      case 'declined':
        return serverText(i.detail) || tr('Declined.');
      case 'withdrawn':
        return serverText(i.detail) || tr('Taken back.');
      default:
        return tr('It expired: ask for the introduction again.');
    }
  }

  protected async answer(i: Introduction, accept: boolean): Promise<void> {
    this.busy.set(i.id);
    this.error.set('');
    try {
      this.answered.emit(await this.api.answerIntroduction(i.id, accept));
      if (accept) this.opened.emit(i);
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(null);
    }
  }
}
