import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  NgZone,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
} from '@angular/core';
import { FormsModule } from '@angular/forms';

import { ApiService, describeError } from '../../core/api.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { Grants, NearbyMachine, pairingLive } from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { SheetComponent } from '../../shared/sheet.component';
import { osName, pairErrorText } from './machines-text';
import { PairConfirmComponent } from './pair-confirm.component';

/** How often the computers nearby are listed again while the sheet shows. */
const NEARBY_MS = 3000;

/**
 * *Join another machine*, on the computer that types the code
 * (spikes/machines/UX.md §2.2): the computers nearby showing a code, the
 * code, this computer's name; then the confirmation (§2.3). The code works
 * across the internet too, so the list is only a reassurance.
 */
@Component({
  selector: 'mc-join-machine-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, PairConfirmComponent, SheetComponent, TPipe],
  template: `
    <mc-sheet [open]="open()" [title]="title()" (closed)="close()">
      @if (open()) {
        @let p = pairing();
        @if (!started() || !p) {
          <form (ngSubmit)="join()">
            <section class="nearby" [attr.aria-label]="'Nearby' | t">
              <h3 class="small">{{ 'Nearby' | t }}</h3>
              @if (!nearbyLoaded()) {
                <p class="muted small" role="status">{{ 'Looking for computers nearby…' | t }}</p>
              } @else if (!nearby().length) {
                <p class="muted small">
                  {{ 'None nearby. Type the code: it works across the internet too.' | t }}
                </p>
              } @else {
                <ul class="list">
                  @for (n of nearby(); track n.id) {
                    <li class="small">
                      <strong>{{ n.name }}</strong>
                      <span class="muted">
                        {{ '{os} · showing a code' | t: { os: os(n.os) } }}</span
                      >
                    </li>
                  }
                </ul>
              }
            </section>
            <label class="field">
              <span class="field-label">{{ 'Code' | t }}</span>
              <input
                name="code"
                class="mono code-input"
                autocomplete="off"
                autocapitalize="characters"
                spellcheck="false"
                maxlength="12"
                placeholder="K7QM-4R2X"
                [ngModel]="code()"
                (ngModelChange)="code.set($event)"
              />
            </label>
            <label class="field">
              <span class="field-label">{{
                'Name of this {computer}' | t: { computer: computer() }
              }}</span>
              <input
                name="name"
                maxlength="40"
                [ngModel]="name()"
                (ngModelChange)="name.set($event)"
              />
            </label>
            @if (error()) {
              <p class="notice error small" role="alert">{{ error() }}</p>
            }
            <div class="row acts">
              <button type="submit" class="btn primary" [disabled]="busy() || !code().trim()">
                {{ busy() ? ('Joining…' | t) : ('Join' | t) }}
              </button>
              <button type="button" class="btn ghost" (click)="closed.emit()">
                {{ 'Cancel' | t }}
              </button>
            </div>
          </form>
        } @else {
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
                {{ 'Waiting for {name} to confirm…' | t: { name: p.other?.name ?? '' } }}
              </p>
              <div class="row acts">
                <button type="button" class="btn ghost" (click)="cancel()">
                  {{ 'Cancel' | t }}
                </button>
              </div>
            }
            @case ('paired') {
              <p class="notice ok" role="status">
                {{ 'Paired with {name}.' | t: { name: p.other?.name ?? '' } }}
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
                <button type="button" class="btn" (click)="tryAgain()">
                  {{ 'Try again' | t }}
                </button>
                <button type="button" class="btn ghost" (click)="closed.emit()">
                  {{ 'Close' | t }}
                </button>
              </div>
            }
            @default {
              <p class="muted" role="status">{{ 'Joining…' | t }}</p>
              <div class="row acts">
                <button type="button" class="btn ghost" (click)="cancel()">
                  {{ 'Cancel' | t }}
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
      .nearby {
        margin-bottom: 0.6rem;
      }
      .nearby h3 {
        margin: 0 0 0.3rem;
        font-weight: 600;
      }
      .nearby ul {
        list-style: none;
        margin: 0;
        padding: 0;
      }
      .nearby li {
        display: flex;
        flex-wrap: wrap;
        align-items: baseline;
        gap: 0.5rem;
        padding: 0.3rem 0;
      }
      .code-input {
        letter-spacing: 0.1em;
        text-transform: uppercase;
      }
      .acts {
        margin-top: 0.8rem;
      }
    `,
  ],
})
export class JoinSheetComponent {
  readonly open = input(false);
  readonly closed = output<void>();

  private readonly api = inject(ApiService);
  private readonly machines = inject(MachinesService);
  private readonly zone = inject(NgZone);

  protected readonly computer = computerNoun;
  protected readonly code = signal('');
  protected readonly name = signal('');
  protected readonly busy = signal(false);
  protected readonly error = signal('');
  /** The join was sent: the pairing's own state shows from here. */
  protected readonly started = signal(false);
  protected readonly nearby = signal<NearbyMachine[]>([]);
  protected readonly nearbyLoaded = signal(false);
  protected readonly pairing = computed(() => {
    const p = this.machines.pairing();
    return p?.role === 'joiner' ? p : null;
  });
  protected readonly title = computed(() =>
    this.started() &&
    (this.pairing()?.state === 'confirm' || this.pairing()?.state === 'waitingOther')
      ? tr('Pair with {name}?', { name: this.pairing()?.other?.name ?? '' })
      : tr('Join another machine'),
  );
  protected readonly failure = computed(() => pairErrorText(this.pairing()));
  private timer?: ReturnType<typeof setInterval>;

  constructor() {
    effect(() => {
      const open = this.open();
      untracked(() => {
        if (open) {
          this.error.set('');
          this.started.set(pairingLive(this.pairing()));
          this.name.set(this.machines.view()?.self.name ?? '');
          this.nearbyLoaded.set(false);
          void this.loadNearby();
          this.startNearby();
        } else {
          this.stopNearby();
        }
      });
    });
    inject(DestroyRef).onDestroy(() => this.stopNearby());
  }

  protected os(os: string): string {
    return osName(os);
  }

  protected async join(): Promise<void> {
    const code = this.code().trim();
    if (!code) return;
    this.busy.set(true);
    this.error.set('');
    try {
      const name = this.name().trim();
      this.machines.apply(await this.api.joinMachine(code, name || undefined));
      this.started.set(true);
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(false);
    }
  }

  protected async confirm(ev: { grants: Grants }): Promise<void> {
    this.busy.set(true);
    this.error.set('');
    try {
      this.machines.apply(await this.api.confirmPairing(ev.grants));
    } catch (e) {
      this.error.set(describeError(e).text);
      await this.machines.load();
    } finally {
      this.busy.set(false);
    }
  }

  protected tryAgain(): void {
    this.started.set(false);
    this.error.set('');
  }

  protected async cancel(): Promise<void> {
    if (pairingLive(this.pairing())) {
      try {
        this.machines.apply(await this.api.cancelPairing());
      } catch {
        // Already over on the server.
      }
    }
    this.closed.emit();
  }

  protected close(): void {
    if (this.started() && pairingLive(this.pairing())) {
      void this.cancel();
    } else {
      this.closed.emit();
    }
  }

  private async loadNearby(): Promise<void> {
    try {
      const r = await this.api.nearbyMachines();
      this.nearby.set(r.machines ?? []);
    } catch {
      this.nearby.set([]);
    } finally {
      this.nearbyLoaded.set(true);
    }
  }

  private startNearby(): void {
    this.stopNearby();
    this.zone.runOutsideAngular(() => {
      this.timer = setInterval(() => {
        if (document.visibilityState !== 'visible' || this.started()) return;
        this.zone.run(() => void this.loadNearby());
      }, NEARBY_MS);
    });
  }

  private stopNearby(): void {
    clearInterval(this.timer);
    this.timer = undefined;
  }
}
