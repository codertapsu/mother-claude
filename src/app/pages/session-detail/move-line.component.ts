import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  signal,
  untracked,
} from '@angular/core';

import { plural } from '../../core/data-jobs';
import { TPipe } from '../../core/i18n/t.pipe';
import { MachineContext } from '../../core/machine-context';
import { canCancel, canRetry, sessionInTurn } from '../../core/moves';
import { activeText, moveErrorText, moveSummary, moveWarnings } from '../../core/moves-text';
import { MoveEntry, MovesService } from '../../core/moves.service';
import { IconComponent } from '../../shared/icon.component';
import { MoveSessionSheetComponent } from '../../shared/move-session-sheet.component';
import { SessionContext } from './session-context';

/**
 * Under a session's header while a move of it runs (spikes/machines/UX.md
 * §7.2, the switch of account's form): "Moving to DESKTOP-PC once this turn
 * ends", what it does now, *Details* (the steps) and *Cancel the move* until
 * it is being placed; once it is there but did not start, why, and *Try
 * again*. Nothing while none runs. It also reads the moves of the
 * session's computer when the page opens, so a move under way shows.
 */
@Component({
  selector: 'mc-move-line',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, MoveSessionSheetComponent, TPipe],
  template: `
    @if (entry(); as e) {
      <div class="pend" role="status" [class.warn]="retryable()">
        @if (!retryable()) {
          <span class="spin" aria-hidden="true"></span>
        }
        <span class="txt">{{ text() }}</span>
        @if (warnings()) {
          <button
            class="btn ghost small warns"
            type="button"
            [title]="'Details' | t"
            (click)="details()"
          >
            <mc-icon name="alert" [size]="13" /> {{ warnings() }}
          </button>
        }
        <button class="btn ghost small" type="button" (click)="details()">
          {{ 'Details' | t }}
        </button>
        @if (retryable()) {
          <button class="btn ghost small" type="button" [disabled]="busy()" (click)="act('retry')">
            {{ 'Try again' | t }}
          </button>
        }
        @if (cancellable()) {
          <button class="btn ghost small" type="button" [disabled]="busy()" (click)="act('cancel')">
            {{ 'Cancel the move' | t }}
          </button>
        }
      </div>
      @if (error()) {
        <p class="err small" role="alert">{{ error() }}</p>
      }
    }
    <mc-move-session-sheet
      [open]="!!shown()"
      [session]="ctx.session() ?? null"
      [at]="shown()?.at ?? null"
      [show]="shown()"
      (closed)="shown.set(null)"
    />
  `,
  styles: [
    `
      :host {
        display: block;
        padding: 0 max(0.6rem, env(safe-area-inset-right)) 0 max(0.6rem, env(safe-area-inset-left));
      }
      .pend {
        max-width: 960px;
        margin: 0.35rem auto;
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.3rem 0.5rem;
        padding: 0.35rem 0.6rem;
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        background: var(--panel-2);
        font-size: 0.85rem;
      }
      .pend.warn {
        border-color: var(--amber-border);
        background: var(--amber-soft);
      }
      .txt {
        flex: 1 1 12rem;
        min-width: 0;
        overflow-wrap: anywhere;
      }
      .spin {
        flex: 0 0 auto;
        width: 0.8rem;
        height: 0.8rem;
        border: 2px solid var(--border-strong);
        border-top-color: var(--accent);
        border-radius: 50%;
        animation: mc-spin 0.8s linear infinite;
      }
      .warns {
        color: var(--amber);
      }
      .err {
        max-width: 960px;
        margin: 0 auto 0.35rem;
        color: var(--red);
      }
    `,
  ],
})
export class MoveLineComponent {
  protected readonly ctx = inject(SessionContext);
  /** Another computer's session (none: this computer's). */
  private readonly machine = inject(MachineContext, { optional: true });
  private readonly moves = inject(MovesService);

  protected readonly busy = signal(false);
  protected readonly error = signal('');
  /** The move whose steps show (null: closed). */
  protected readonly shown = signal<MoveEntry | null>(null);

  protected readonly entry = computed(() => {
    const id = this.ctx.id();
    return id ? this.moves.current(id) : null;
  });
  protected readonly retryable = computed(() => {
    const e = this.entry();
    return !!e && canRetry(e.move);
  });
  protected readonly cancellable = computed(() => {
    const e = this.entry();
    return !!e && canCancel(e.move);
  });
  protected readonly text = computed(() => {
    const e = this.entry();
    if (!e) return '';
    const words = this.moves.words(sessionInTurn(this.ctx.session()));
    return this.retryable() ? moveSummary(e.move, words) : activeText(e.move, words);
  });
  /** "2 warnings" when its record has some ('' for none): *Details* shows
   * them. */
  protected readonly warnings = computed(() => {
    const e = this.entry();
    const n = e ? moveWarnings(e.move).length : 0;
    return n ? plural(n, 'warning') : '';
  });

  constructor() {
    this.moves.ensureMachines();
    // The moves of the computer the session lives on, once per page: a move
    // under way shows (this computer's follow by frames, another's are
    // asked about until they end).
    let read = '';
    effect(() => {
      const id = this.ctx.id();
      const at = this.machine?.id ?? null;
      const on = at !== null || this.moves.enabled();
      const key = `${at ?? ''}|${id}`;
      if (!id || !on || read === key) return;
      read = key;
      untracked(() => void this.moves.load(at));
    });
    // A move of it ended: what the page shows of it (where it lives) may
    // have changed outside the live list.
    let had = false;
    effect(() => {
      const running = !!this.entry();
      if (had && !running) untracked(() => void this.ctx.refreshDetail());
      had = running;
    });
  }

  protected details(): void {
    this.shown.set(this.entry());
  }

  protected async act(what: 'cancel' | 'retry'): Promise<void> {
    const e = this.entry();
    if (!e || this.busy()) return;
    this.busy.set(true);
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
      this.busy.set(false);
    }
  }
}
