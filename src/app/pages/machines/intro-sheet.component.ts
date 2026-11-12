import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  input,
  output,
  signal,
} from '@angular/core';

import { ApiService, describeError } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { Grants, pairingLive } from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { SheetComponent } from '../../shared/sheet.component';
import { pairErrorText } from './machines-text';
import { PairConfirmComponent } from './pair-confirm.component';

/**
 * The pairing an accepted introduction runs (docs/PLAN-MACHINES.md
 * §5.12): no code — the computer that introduced them told each whom to
 * dial — then the same confirmation as a pairing by code: the six digits on
 * both screens (typed on this one when the other came over the internet)
 * and what the other may do here. Closing it while it runs cancels it.
 */
@Component({
  selector: 'mc-intro-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [PairConfirmComponent, SheetComponent, TPipe],
  template: `
    <mc-sheet [open]="open()" [title]="title()" (closed)="close()">
      @if (open()) {
        @let p = pairing();
        @if (p && p.intro) {
          @let intro = p.intro;
          <p class="muted small">
            {{ 'Introduced by {name}.' | t: { name: intro.by.name } }}
          </p>
          @switch (p.state) {
            @case ('confirm') {
              <mc-pair-confirm
                [pairing]="p"
                [busy]="busy()"
                [error]="error()"
                (confirmed)="confirm($event)"
                (cancelled)="cancel()"
              />
            }
            @case ('waitingOther') {
              <p class="muted" role="status">
                {{ 'Waiting for {name} to confirm…' | t: { name: intro.other.name } }}
              </p>
              <div class="row acts">
                <button type="button" class="btn ghost" (click)="cancel()">
                  {{ 'Cancel' | t }}
                </button>
              </div>
            }
            @case ('paired') {
              <p class="notice ok" role="status">
                {{ 'Paired with {name}.' | t: { name: intro.other.name } }}
              </p>
              <div class="row acts">
                <button type="button" class="btn primary" (click)="closed.emit()">
                  {{ 'Done' | t }}
                </button>
              </div>
            }
            @case ('failed') {
              <p class="notice error" role="alert">{{ failure() }}</p>
              <div class="row acts">
                <button type="button" class="btn ghost" (click)="closed.emit()">
                  {{ 'Close' | t }}
                </button>
              </div>
            }
            @default {
              <p class="muted" role="status">
                @if (p.waitingAccept) {
                  {{
                    'Waiting for {name} to accept the introduction there…'
                      | t: { name: intro.other.name }
                  }}
                } @else if (p.role === 'host') {
                  {{ 'Waiting for {name}…' | t: { name: intro.other.name } }}
                } @else {
                  {{ 'Connecting to {name}…' | t: { name: intro.other.name } }}
                }
              </p>
              <div class="row acts">
                <button type="button" class="btn ghost" (click)="cancel()">
                  {{ 'Cancel' | t }}
                </button>
              </div>
            }
          }
        } @else {
          <p class="muted" role="status">{{ 'Nothing is pairing now.' | t }}</p>
          <div class="row acts">
            <button type="button" class="btn ghost" (click)="closed.emit()">
              {{ 'Close' | t }}
            </button>
          </div>
        }
      }
    </mc-sheet>
  `,
  styles: [
    `
      .acts {
        margin-top: 0.8rem;
      }
    `,
  ],
})
export class IntroSheetComponent {
  readonly open = input(false);
  readonly closed = output<void>();

  private readonly api = inject(ApiService);
  private readonly machines = inject(MachinesService);

  protected readonly busy = signal(false);
  protected readonly error = signal('');
  /** This computer's pairing, when an introduction runs it. */
  protected readonly pairing = computed(() => {
    const p = this.machines.pairing();
    return p?.intro ? p : null;
  });
  protected readonly title = computed(() => {
    const name = this.pairing()?.intro?.other.name;
    return name ? tr('Pair with {name}?', { name }) : tr('Introductions');
  });
  protected readonly failure = computed(() => pairErrorText(this.pairing()));

  protected async confirm(ev: { grants: Grants; digits?: string }): Promise<void> {
    this.busy.set(true);
    this.error.set('');
    try {
      this.machines.apply(await this.api.confirmPairing(ev.grants, ev.digits));
    } catch (e) {
      this.error.set(describeError(e).text);
      await this.machines.load();
    } finally {
      this.busy.set(false);
    }
  }

  protected async cancel(): Promise<void> {
    if (pairingLive(this.pairing())) {
      try {
        this.machines.apply(await this.api.cancelPairing());
      } catch {
        // Already over on the server: nothing to cancel.
      }
    }
    this.closed.emit();
  }

  protected close(): void {
    if (pairingLive(this.pairing())) {
      void this.cancel();
    } else {
      this.closed.emit();
    }
  }
}
