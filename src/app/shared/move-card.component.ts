import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  input,
  linkedSignal,
  signal,
} from '@angular/core';
import { RouterLink } from '@angular/router';

import { locale, tr } from '../core/i18n/i18n';
import { TPipe } from '../core/i18n/t.pipe';
import {
  canCancel,
  canRemoveWorktree,
  canRetry,
  canUndo,
  moveFinal,
  moveFraction,
  movedSession,
} from '../core/moves';
import {
  moveErrorText,
  moveRoute,
  moveStateLabel,
  moveSummary,
  moveTitle,
  moveTone,
  moveWarnings,
} from '../core/moves-text';
import { MoveEntry, MovesService } from '../core/moves.service';
import { NowService } from '../core/now.service';
import { sessionLink } from '../core/session-links';
import { IconComponent } from './icon.component';
import { TimeAgoComponent } from './time-ago.component';

/** "17 Nov, 14:30", in the language shown. */
function dateTime(ms: number): string {
  const d = new Date(ms);
  return `${d.toLocaleDateString(locale(), { day: 'numeric', month: 'short' })}, ${d.toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit' })}`;
}

/**
 * One move of a session between computers, in a list (*Recent moves* on the
 * Machines page, a session's Info tab; spikes/machines/UX.md §7.2–§7.3):
 * which session, from where to where, how far it got or how it ended, and
 * what can be done — cancel it (until it is placed), try starting it again
 * there, open it where it runs, undo it (confirmed with a second tap, until
 * the day it says) — on the computer whose record it is.
 */
@Component({
  selector: 'mc-move-card',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, IconComponent, TimeAgoComponent, TPipe],
  host: { '[attr.data-tone]': 'tone()' },
  template: `
    @let m = entry().move;
    <div class="mhead">
      <mc-icon class="kind" name="monitor" [size]="15" />
      <strong class="mtitle">{{ title() }}</strong>
      <span class="mstate">{{ stateLabel() }}</span>
      <mc-time class="when muted small" [ms]="m.finishedAt ?? m.createdAt" />
    </div>
    <span class="route muted small">{{ route() }}</span>
    @if (running()) {
      <div
        class="bar"
        role="progressbar"
        aria-valuemin="0"
        aria-valuemax="100"
        [attr.aria-valuenow]="pct()"
        [attr.aria-label]="'{name}: progress' | t: { name: title() }"
      >
        <span class="fill" [style.width.%]="pct()"></span>
      </div>
    }
    <p class="small sum" [class.error-text]="m.state === 'failed'">{{ summary() }}</p>
    @if (warnings().length) {
      <ul class="warns small" [attr.aria-label]="'Worth knowing' | t">
        @for (w of warnings(); track $index) {
          <li>
            <mc-icon name="alert" [size]="13" /><span>{{ w }}</span>
          </li>
        }
      </ul>
    }
    @if (m.worktree?.removedAt) {
      <p class="muted small sum">
        {{
          'Its worktree on {name} was removed {time}.'
            | t: { name: worktreeHost(), time: removedAt() }
        }}
      </p>
    }
    @if (cancellable() || retryable() || openLink() || undoable() || removable()) {
      <div class="acts">
        @if (cancellable()) {
          <button class="btn small" type="button" [disabled]="busy()" (click)="act('cancel')">
            {{ 'Cancel the move' | t }}
          </button>
        }
        @if (retryable()) {
          <button class="btn small" type="button" [disabled]="busy()" (click)="act('retry')">
            <mc-icon name="refresh" [size]="14" /> {{ 'Try again' | t }}
          </button>
        }
        @if (openLink(); as link) {
          <a class="btn small" [routerLink]="link">{{ openLabel() }}</a>
        }
        @if (undoable()) {
          <button
            class="btn small"
            type="button"
            [class.danger]="confirming()"
            [disabled]="busy()"
            (click)="undo()"
          >
            <mc-icon name="undo" [size]="14" />
            {{
              busy() === 'undo'
                ? ('Undoing…' | t)
                : confirming()
                  ? ('Tap again to undo' | t)
                  : ('Undo move' | t)
            }}
          </button>
          @if (m.undoUntil) {
            <span class="muted small">{{ 'Possible until {time}.' | t: { time: until() } }}</span>
          }
        }
        @if (removable()) {
          <button
            class="btn small"
            type="button"
            [class.danger]="removing()"
            [disabled]="busy()"
            [title]="'Its work went with the session; the snapshots stay 7 days.' | t"
            (click)="removeWorktree()"
          >
            <mc-icon name="trash" [size]="14" />
            {{
              busy() === 'removeWorktree'
                ? ('Removing…' | t)
                : removing()
                  ? ('Tap again to remove' | t)
                  : ('Remove the worktree' | t)
            }}
          </button>
        }
      </div>
    }
    @if (error()) {
      <p class="error-text small sum" role="alert">{{ error() }}</p>
    }
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.3rem;
        min-width: 0;
      }
      .mhead {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.2rem 0.5rem;
        min-width: 0;
      }
      .kind {
        color: var(--muted);
      }
      .mtitle {
        min-width: 0;
        overflow-wrap: anywhere;
      }
      .mstate {
        font-size: 0.72rem;
        padding: 0.05rem 0.4rem;
        border-radius: 0.4rem;
        background: var(--panel-3);
        color: var(--muted);
      }
      :host([data-tone='running']) .mstate {
        background: var(--accent-soft);
        color: var(--accent-text);
      }
      :host([data-tone='done']) .mstate {
        background: var(--green-soft);
        color: var(--green);
      }
      :host([data-tone='failed']) .mstate {
        background: var(--red-soft);
        color: var(--red);
      }
      .when {
        margin-left: auto;
      }
      .route {
        overflow-wrap: anywhere;
      }
      .bar {
        height: 0.4rem;
        border-radius: 0.2rem;
        background: var(--panel-3);
        overflow: hidden;
      }
      .fill {
        display: block;
        height: 100%;
        border-radius: inherit;
        background: var(--accent);
        transition: width 0.3s ease;
      }
      .sum {
        margin: 0;
        overflow-wrap: anywhere;
      }
      .warns {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 0.2rem;
      }
      .warns li {
        display: flex;
        align-items: flex-start;
        gap: 0.35rem;
        overflow-wrap: anywhere;
      }
      .warns mc-icon {
        flex: 0 0 auto;
        margin-top: 0.15rem;
        color: var(--amber);
      }
      .acts {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.4rem 0.6rem;
      }
    `,
  ],
})
export class MoveCardComponent {
  private readonly moves = inject(MovesService);
  private readonly clock = inject(NowService);

  readonly entry = input.required<MoveEntry>();

  /** The action under way ('' for none). */
  protected readonly busy = signal<'' | 'cancel' | 'undo' | 'retry' | 'removeWorktree'>('');
  protected readonly error = linkedSignal({
    source: () => this.entry().move.id,
    computation: () => '',
  });
  protected readonly confirming = linkedSignal({
    source: () => this.entry().move.id,
    computation: () => false,
  });
  /** *Remove the worktree* armed (a second tap removes it). */
  protected readonly removing = linkedSignal({
    source: () => this.entry().move.id,
    computation: () => false,
  });
  private confirmTimer?: ReturnType<typeof setTimeout>;
  private removeTimer?: ReturnType<typeof setTimeout>;

  /** Its computers in this computer's words. */
  private readonly words = this.moves.words();
  protected readonly title = computed(() => moveTitle(this.entry().move));
  protected readonly route = computed(() => moveRoute(this.entry().move, this.words));
  protected readonly stateLabel = computed(() => moveStateLabel(this.entry().move));
  protected readonly tone = computed(() => moveTone(this.entry().move));
  protected readonly running = computed(() => !moveFinal(this.entry().move));
  protected readonly pct = computed(() => Math.round(moveFraction(this.entry().move) * 100));
  protected readonly summary = computed(() => moveSummary(this.entry().move, this.words));
  /** What its record says is worth knowing. */
  protected readonly warnings = computed(() => moveWarnings(this.entry().move));
  protected readonly cancellable = computed(() => canCancel(this.entry().move));
  protected readonly retryable = computed(() => canRetry(this.entry().move));
  protected readonly undoable = computed(() => canUndo(this.entry().move, this.clock.now()));
  protected readonly until = computed(() => dateTime(this.entry().move.undoUntil ?? 0));
  /** *Remove the worktree* (plan §5.10): the separate folder its code
   * landed in, once the session left it or ended there. */
  protected readonly removable = computed(() => canRemoveWorktree(this.entry().move));
  /** The computer the worktree is on: the one the move went to. */
  protected readonly worktreeHost = computed(() => this.words.name(this.entry().move.to));
  protected readonly removedAt = computed(() =>
    dateTime(this.entry().move.worktree?.removedAt ?? 0),
  );

  /** The computer it went on on, when it runs there now. */
  private readonly runsOn = computed(() => {
    const { at, move: m } = this.entry();
    if (m.state !== 'resumed') return null;
    const self = this.moves.selfId();
    const where = m.role === 'out' ? m.to.id : (at ?? self);
    if (where === self) return { machine: null };
    return this.moves.peers().some((p) => p.id === where) ? { machine: where } : null;
  });
  protected readonly openLink = computed(() => {
    const r = this.runsOn();
    return r ? sessionLink(movedSession(this.entry().move), r.machine) : null;
  });
  protected readonly openLabel = computed(() =>
    this.runsOn()?.machine ? tr('Open it there') : tr('Open it'),
  );

  protected removeWorktree(): void {
    if (this.busy()) return;
    if (!this.removing()) {
      this.removing.set(true);
      clearTimeout(this.removeTimer);
      this.removeTimer = setTimeout(() => this.removing.set(false), 4000);
      return;
    }
    clearTimeout(this.removeTimer);
    this.removing.set(false);
    void this.act('removeWorktree');
  }

  protected async act(what: 'cancel' | 'retry' | 'undo' | 'removeWorktree'): Promise<void> {
    if (this.busy()) return;
    const e = this.entry();
    this.busy.set(what);
    this.error.set('');
    try {
      await this.moves.act(e, what);
    } catch (err) {
      this.error.set(
        moveErrorText(err, {
          source: e.at ? this.moves.nameOf(e.at) : '',
          target: e.move.to.name,
        }),
      );
    } finally {
      this.busy.set('');
    }
  }

  protected undo(): void {
    if (this.busy()) return;
    if (!this.confirming()) {
      this.confirming.set(true);
      clearTimeout(this.confirmTimer);
      this.confirmTimer = setTimeout(() => this.confirming.set(false), 4000);
      return;
    }
    clearTimeout(this.confirmTimer);
    this.confirming.set(false);
    void this.act('undo');
  }
}
