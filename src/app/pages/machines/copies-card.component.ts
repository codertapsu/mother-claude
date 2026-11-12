import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  inject,
  signal,
} from '@angular/core';
import { Router } from '@angular/router';

import { ApiService, describeError } from '../../core/api.service';
import {
  HeldCopy,
  KeptCopy,
  copyClock,
  copySize,
  copyStateText,
  copyTitle,
  copyTone,
} from '../../core/copies';
import { CopiesService } from '../../core/copies.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { MovesService } from '../../core/moves.service';
import { NoticeService } from '../../core/notice.service';
import { sessionLink } from '../../core/session-links';

/**
 * Copies on the Machines page (docs/PLAN-MACHINES.md §5.9): the copies of
 * other computers' sessions held here — the session, its computer, its size,
 * when it was last copied, *Continue here* while that computer is offline,
 * *Remove* — and this computer's sessions kept on others, with how each
 * stands and *Stop keeping it there*. A session keeps a copy from its menu
 * (*Keep a copy on …*) or its project's options. The desktop and a paired
 * phone may remove one.
 */
@Component({
  selector: 'mc-machines-copies',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [TPipe],
  template: `
    <section id="copies" class="card copies" [attr.aria-label]="'Copies' | t">
      <div class="chead">
        <h2 class="first">{{ 'Copies' | t }}</h2>
      </div>
      <p class="muted small">
        {{
          'A session marked Keep a copy on … is copied to that computer after each turn: moving it is instant, and that computer can go on with it while this one sleeps.'
            | t
        }}
      </p>
      @if (error()) {
        <p class="notice error small" role="alert">{{ error() }}</p>
      }
      @if (copies.error()) {
        <p class="notice error small" role="alert">{{ copies.error() }}</p>
      }

      <h3>{{ 'Held on this {computer}' | t: { computer: computer() } }}</h3>
      <ul class="list">
        @for (h of copies.held(); track h.id) {
          <li>
            <div class="row">
              <strong class="name">{{ title(h) }}</strong>
              <span class="muted small">{{ 'from {name}' | t: { name: h.machineName } }}</span>
              <span class="spacer"></span>
              <span class="muted small">{{ size(h.bytes) }}</span>
            </div>
            <span class="small">
              @if (h.copiedAt) {
                {{ 'Copied at {time}' | t: { time: clock(h.copiedAt) } }}
              } @else {
                {{ 'Not complete yet' | t }}
              }
              @if (h.livesHere) {
                · {{ 'it lives on this {computer} now' | t: { computer: computer() } }}
              } @else if (!h.online) {
                · {{ '{name} is offline' | t: { name: h.machineName } }}
              }
            </span>
            <span class="muted small mono">{{ h.cwd }}</span>
            <div class="row acts">
              @if (h.canContinue) {
                <button
                  type="button"
                  class="btn small primary"
                  [disabled]="busy() === h.id"
                  (click)="continueHere(h)"
                >
                  {{ 'Continue here' | t }}
                </button>
              }
              <button
                type="button"
                class="btn small"
                [class.danger]="armed(h.id)"
                [disabled]="busy() === h.id"
                (click)="remove(h)"
              >
                {{ armed(h.id) ? ('Tap again to remove' | t) : ('Remove' | t) }}
              </button>
            </div>
          </li>
        } @empty {
          <li class="muted small">{{ 'No copy of another computer’s session is here.' | t }}</li>
        }
      </ul>
      @if (capText(); as cap) {
        <p class="muted small">{{ cap }}</p>
      }

      <h3>{{ 'Kept on other computers' | t }}</h3>
      <ul class="list">
        @for (k of copies.kept(); track k.id) {
          <li>
            <div class="row">
              <strong class="name">{{ title(k) }}</strong>
              <span class="muted small">{{ 'on {name}' | t: { name: k.machineName } }}</span>
              <span class="spacer"></span>
              @if (k.status?.bytes) {
                <span class="muted small">{{ size(k.status!.bytes) }}</span>
              }
            </div>
            <span class="small state" [attr.data-tone]="tone(k)">{{ stateText(k) }}</span>
            @if (k.from === 'project') {
              <span class="muted small">{{ 'as its project’s options say' | t }}</span>
            }
            <div class="row acts">
              <button
                type="button"
                class="btn small"
                [class.danger]="armed(k.id)"
                [disabled]="busy() === k.id"
                (click)="remove(k)"
              >
                {{
                  armed(k.id)
                    ? ('Tap again to stop' | t)
                    : ('Stop keeping it on {name}' | t: { name: k.machineName })
                }}
              </button>
            </div>
          </li>
        } @empty {
          <li class="muted small">
            {{
              'No session of this {computer} keeps a copy elsewhere: choose Keep a copy on … in a session’s menu.'
                | t: { computer: computer() }
            }}
          </li>
        }
      </ul>
    </section>
  `,
  styles: [
    `
      h3 {
        margin: 0.75rem 0 0.25rem;
        font-size: 0.9rem;
      }
      .list {
        list-style: none;
        margin: 0.25rem 0;
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
      .row {
        display: flex;
        align-items: center;
        gap: 0.4rem;
        flex-wrap: wrap;
      }
      .spacer {
        flex: 1;
      }
      .acts {
        margin-top: 0.2rem;
      }
      .state[data-tone='ok'] {
        color: var(--green);
      }
      .state[data-tone='off'] {
        color: var(--amber);
      }
    `,
  ],
})
export class CopiesCardComponent {
  protected readonly copies = inject(CopiesService);
  private readonly api = inject(ApiService);
  private readonly moves = inject(MovesService);
  private readonly notices = inject(NoticeService);
  private readonly router = inject(Router);

  protected readonly computer = computerNoun;
  protected readonly busy = signal<string | null>(null);
  protected readonly error = signal('');
  private readonly confirming = signal<string | null>(null);
  private confirmTimer?: ReturnType<typeof setTimeout>;

  protected readonly capText = computed(() => {
    const v = this.copies.view();
    if (!v || !v.held.length) return '';
    return tr(
      'This {computer} keeps at most {cap} of each computer’s copies: the oldest goes first.',
      {
        computer: computerNoun(),
        cap: copySize(v.capBytes),
      },
    );
  });

  constructor() {
    const release = this.copies.watch();
    inject(DestroyRef).onDestroy(() => {
      release();
      clearTimeout(this.confirmTimer);
    });
  }

  protected title(c: HeldCopy | KeptCopy): string {
    return copyTitle(c);
  }

  protected size(bytes: number): string {
    return copySize(bytes);
  }

  protected clock(ms: number | null): string {
    return copyClock(ms);
  }

  protected stateText(k: KeptCopy): string {
    return copyStateText(k.status);
  }

  protected tone(k: KeptCopy): string {
    return copyTone(k.status);
  }

  protected armed(id: string): boolean {
    return this.confirming() === id;
  }

  protected async remove(c: HeldCopy | KeptCopy): Promise<void> {
    if (!this.armed(c.id)) {
      this.confirming.set(c.id);
      clearTimeout(this.confirmTimer);
      this.confirmTimer = setTimeout(() => this.confirming.set(null), 4000);
      return;
    }
    this.confirming.set(null);
    await this.withBusy(c.id, async () => {
      this.copies.apply(await this.api.removeCopy(c.id));
      this.notices.success(
        c.id.startsWith('held.')
          ? tr('The copy is gone from this {computer}.', { computer: computerNoun() })
          : tr('{name} keeps no copy of it any more.', { name: c.machineName }),
      );
    });
  }

  /** Its home is offline: it goes on here from this copy. */
  protected async continueHere(h: HeldCopy): Promise<void> {
    await this.withBusy(h.id, async () => {
      const reply = await this.moves.takeOver(h.session, { fromCopy: true });
      if (reply.resumeError) {
        this.notices.show(
          'warning',
          tr('It is on this {computer} now but did not start: {why}', {
            computer: computerNoun(),
            why: reply.resumeError,
          }),
        );
      } else {
        this.notices.success(
          tr('{title} goes on here from the copy of {time}.', {
            title: copyTitle(h),
            time: copyClock(h.copiedAt),
          }),
        );
      }
      void this.copies.load();
      void this.router.navigate(sessionLink(h.session, null));
    });
  }

  private async withBusy(key: string, fn: () => Promise<void>): Promise<void> {
    this.busy.set(key);
    this.error.set('');
    try {
      await fn();
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.busy.set(null);
    }
  }
}
