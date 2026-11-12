import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';

import { ApiService, describeError } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { IntroduceResult, PeerMachine, introducibleTo, unpairedPairs } from '../../core/machines';
import { MachinesService } from '../../core/machines.service';
import { serverText } from './machines-text';

/** One offer: introduce `machine` (it dials) to `with` (it waits). */
interface Offer {
  machine: PeerMachine;
  with: PeerMachine;
}

/**
 * *Also pair DESKTOP-PC with MacBook?* (docs/PLAN-MACHINES.md §5.12): your
 * paired computers that aren't paired with each other, as each last said.
 * Introducing them tells each who the other is; each then shows the
 * pairing, and the two pair only once both desktops confirmed the six
 * digits. With `machine` (just paired), only its offers; else every pair.
 */
@Component({
  selector: 'mc-introduce-offer',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [TPipe],
  template: `
    @if (offers().length) {
      <div
        class="intro-offers"
        [class.card]="framed()"
        role="group"
        [attr.aria-label]="'Pair your computers with each other' | t"
      >
        @if (heading()) {
          <h3 class="small">{{ 'Pair your computers with each other' | t }}</h3>
        }
        <ul class="list">
          @for (o of offers(); track o.machine.id + o.with.id) {
            <li>
              <span class="grow">{{
                'Also pair {a} with {b}?' | t: { a: o.machine.name, b: o.with.name }
              }}</span>
              @let r = result(o);
              @if (r) {
                <span class="muted small" role="status">{{ r }}</span>
              } @else {
                <button
                  type="button"
                  class="btn small"
                  [disabled]="busy() === key(o)"
                  (click)="introduce(o)"
                >
                  {{ busy() === key(o) ? ('Introducing…' | t) : ('Introduce them' | t) }}
                </button>
              }
            </li>
          }
        </ul>
        @if (error()) {
          <p class="notice error small" role="alert">{{ error() }}</p>
        }
        <p class="muted small">
          {{
            'Nothing is trusted on their behalf: each shows the pairing, and you confirm the six digits on both.'
              | t
          }}
        </p>
      </div>
    }
  `,
  styles: [
    `
      .intro-offers {
        margin-top: 0.8rem;
      }
      h3 {
        margin: 0 0 0.3rem;
        font-weight: 600;
      }
      .list {
        list-style: none;
        margin: 0.2rem 0;
        padding: 0;
      }
      .list > li {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0.5rem;
        padding: 0.3rem 0;
      }
      .grow {
        flex: 1 1 auto;
      }
    `,
  ],
})
export class IntroduceOfferComponent {
  /** The computer just paired: only its offers. */
  readonly machine = input<string | null>(null);
  /** Show the heading (a section of its own). */
  readonly heading = input(true);
  /** A card of its own (Computers › Overview). */
  readonly framed = input(false);

  private readonly api = inject(ApiService);
  private readonly machines = inject(MachinesService);

  protected readonly busy = signal<string | null>(null);
  protected readonly error = signal('');
  /** What each introduction came to, by its two computers. */
  private readonly results = signal<Record<string, string>>({});

  protected readonly offers = computed<Offer[]>(() => {
    // A computer too old to be introduced is named on its own card.
    const peers = (this.machines.view()?.peers ?? []).filter(
      (p) => !p.skew?.lacks?.includes('introductions'),
    );
    const m = this.machine();
    if (m) {
      const self = peers.find((p) => p.id === m);
      return self ? introducibleTo(peers, m).map((p) => ({ machine: self, with: p })) : [];
    }
    return unpairedPairs(peers).map(([a, b]) => ({ machine: b, with: a }));
  });

  protected key(o: Offer): string {
    return `${o.machine.id}:${o.with.id}`;
  }

  protected result(o: Offer): string {
    return this.results()[this.key(o)] ?? '';
  }

  protected async introduce(o: Offer): Promise<void> {
    const key = this.key(o);
    this.busy.set(key);
    this.error.set('');
    try {
      const r = await this.api.introduceMachines(o.machine.id, o.with.id);
      this.results.update((all) => ({ ...all, [key]: outcomeText(r) }));
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(null);
    }
  }
}

/** How an introduction went, in a sentence. */
export function outcomeText(r: IntroduceResult): string {
  const sides = [r.joiner, r.host];
  if (sides.some((s) => s.state === 'already')) {
    return tr('{a} and {b} are paired already.', { a: r.joiner.name, b: r.host.name });
  }
  const failed = sides.find((s) => s.state === 'failed');
  if (failed) {
    return failed.detail
      ? tr('{name} could not be told: {why}', { name: failed.name, why: serverText(failed.detail) })
      : tr('{name} could not be told.', { name: failed.name });
  }
  return tr('Introduced: confirm the six digits on {a} and on {b}.', {
    a: r.joiner.name,
    b: r.host.name,
  });
}
