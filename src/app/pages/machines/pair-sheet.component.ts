import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
} from '@angular/core';

import { ApiService, describeError } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { Grants, pairingLive } from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { NowService } from '../../core/now.service';
import { CopyButtonComponent } from '../../shared/copy-button.component';
import { SheetComponent } from '../../shared/sheet.component';
import { IntroduceOfferComponent } from './introduce-offer.component';
import { pairErrorText } from './machines-text';
import { PairConfirmComponent } from './pair-confirm.component';

/**
 * *Pair a machine*, on the computer that shows the code
 * (spikes/machines/UX.md §2.1): a new code when it opens, its countdown,
 * then the confirmation (§2.3) once the other computer joined. Closing it
 * while the pairing runs cancels the pairing (the other computer is told).
 */
@Component({
  selector: 'mc-pair-machine-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    CopyButtonComponent,
    IntroduceOfferComponent,
    PairConfirmComponent,
    SheetComponent,
    TPipe,
  ],
  template: `
    <mc-sheet [open]="open()" [title]="title()" (closed)="close()">
      @if (open()) {
        @let p = pairing();
        @if (!p || p.state === 'starting') {
          @if (error()) {
            <p class="notice error" role="alert">{{ error() }}</p>
            <div class="row">
              <button type="button" class="btn" (click)="newCode()">{{ 'Try again' | t }}</button>
            </div>
          } @else {
            <p class="muted" role="status">{{ 'Creating…' | t }}</p>
          }
        } @else {
          @switch (p.state) {
            @case ('waiting') {
              <p>
                {{
                  'On the other computer, open Mother Claude › Machines › Join and type this code. On the same network it lists “{name}” among the computers nearby, so you can check which computer shows the code.'
                    | t: { name: selfName() }
                }}
              </p>
              @if (expired()) {
                <p class="notice warn" role="status">{{ 'Expired — create a new code.' | t }}</p>
              } @else {
                <p
                  class="code mono"
                  [attr.aria-label]="'Pairing code {code}' | t: { code: spoken() }"
                >
                  {{ p.code }}
                </p>
                <p class="muted small" role="timer">
                  {{ 'Works once · expires in {time}' | t: { time: countdown() } }}
                </p>
                <div class="row">
                  <mc-copy-button [text]="p.code ?? ''" [label]="'Copy code' | t" />
                </div>
                <p class="muted" role="status">{{ 'Waiting for the other computer…' | t }}</p>
              }
              <div class="row acts">
                <button type="button" class="btn" [disabled]="busy()" (click)="newCode()">
                  {{ 'New code' | t }}
                </button>
                <button type="button" class="btn ghost" [disabled]="busy()" (click)="cancel()">
                  {{ 'Cancel' | t }}
                </button>
              </div>
            }
            @case ('joining') {
              <p class="muted" role="status">
                {{ '{name} is joining…' | t: { name: p.other?.name ?? ('A computer' | t) } }}
              </p>
            }
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
                {{ 'Waiting for {name} to confirm…' | t: { name: p.other?.name ?? '' } }}
              </p>
              <div class="row acts">
                <button type="button" class="btn ghost" [disabled]="busy()" (click)="cancel()">
                  {{ 'Cancel' | t }}
                </button>
              </div>
            }
            @case ('paired') {
              <p class="notice ok" role="status">
                {{ 'Paired with {name}.' | t: { name: p.other?.name ?? '' } }}
              </p>
              <mc-introduce-offer [machine]="p.peerId" [heading]="false" />
              <div class="row acts">
                <button type="button" class="btn primary" (click)="closed.emit()">
                  {{ 'Done' | t }}
                </button>
              </div>
            }
            @case ('failed') {
              <p class="notice error" role="alert">{{ failure() }}</p>
              <div class="row acts">
                <button type="button" class="btn" [disabled]="busy()" (click)="newCode()">
                  {{ 'New code' | t }}
                </button>
                <button type="button" class="btn ghost" (click)="closed.emit()">
                  {{ 'Close' | t }}
                </button>
              </div>
            }
          }
        }
      }
    </mc-sheet>
  `,
  styles: [
    `
      .code {
        font-size: 2rem;
        font-weight: 700;
        letter-spacing: 0.12em;
        text-align: center;
        margin: 0.6rem 0 0.2rem;
      }
      p.small {
        text-align: center;
      }
      .acts {
        margin-top: 0.8rem;
      }
    `,
  ],
})
export class PairSheetComponent {
  readonly open = input(false);
  readonly closed = output<void>();

  private readonly api = inject(ApiService);
  private readonly machines = inject(MachinesService);
  private readonly clock = inject(NowService);

  protected readonly busy = signal(false);
  protected readonly error = signal('');
  /** This computer's pairing as the one showing the code (an introduced
   * one has its own sheet). */
  protected readonly pairing = computed(() => {
    const p = this.machines.pairing();
    return p?.role === 'host' && !p.intro ? p : null;
  });
  protected readonly selfName = computed(() => this.machines.view()?.self.name ?? '');
  protected readonly title = computed(() =>
    this.pairing()?.state === 'confirm' || this.pairing()?.state === 'waitingOther'
      ? tr('Pair with {name}?', { name: this.pairing()?.other?.name ?? '' })
      : tr('Pair a machine'),
  );
  private readonly secondsLeft = computed(() => {
    const at = this.pairing()?.expiresAt;
    return at ? Math.max(0, Math.round((at - this.clock.now()) / 1000)) : 0;
  });
  protected readonly expired = computed(
    () => this.pairing()?.state === 'waiting' && this.secondsLeft() === 0,
  );
  protected readonly countdown = computed(() => {
    const s = this.secondsLeft();
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
  });
  /** The code spelled out for a screen reader. */
  protected readonly spoken = computed(() =>
    (this.pairing()?.code ?? '').replace(/-/g, '').split('').join(' '),
  );
  protected readonly failure = computed(() => pairErrorText(this.pairing()));

  constructor() {
    effect(() => {
      if (!this.open()) return;
      untracked(() => {
        this.error.set('');
        if (!pairingLive(this.pairing())) void this.newCode();
      });
    });
  }

  protected async newCode(): Promise<void> {
    this.busy.set(true);
    this.error.set('');
    try {
      this.machines.apply(await this.api.startPairing());
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(false);
    }
  }

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
