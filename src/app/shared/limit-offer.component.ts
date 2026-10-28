import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';

import { AccountLimit, AccountsService } from '../core/accounts.service';
import { AccountView, SessionSummary } from '../core/models';
import { NowService } from '../core/now.service';
import { RealtimeService } from '../core/realtime.service';
import { canContinue } from './continue-on-sheet.component';
import { IconComponent } from './icon.component';
import { switchMode } from './switch-account';
import { SwitchAccountSheetComponent } from './switch-account-sheet.component';
import { resetsIn } from './util';

interface Offer {
  limit: AccountLimit;
  suggestion: AccountView | null;
  /** "resets in 2h 10m" ('' when unknown). */
  resets: string;
  /** `switch`: the session goes on under the other account; `continue`:
   * another app runs it, so a new session continues it there. */
  verb: 'switch' | 'continue';
}

/** A session the offer can help: one that switches account (it runs here,
 * or not at all), or one another app runs that can be continued. */
function helped(s: SessionSummary): boolean {
  return switchMode(s) !== 'elsewhere' || canContinue(s);
}

/**
 * "Account Work has reached its 5-hour limit — switch to Marcus (Pro)?"
 * (docs/ACCOUNTS.md §6.2, §15.1), with a button that opens Switch account
 * with that account suggested: another account that is signed in and free
 * of limits, one sharing the directory first, else one with its own (the
 * conversation moves there first). A session another app runs can't
 * switch: for it the offer is to continue on that account. On a session
 * page (`session`) it speaks for that session's account; on the sessions
 * list for every account at a limit that has conversations here (picked
 * in the sheet). Only while more than one account exists; "Not now" hides
 * it until the window resets.
 */
@Component({
  selector: 'mc-limit-offer',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, SwitchAccountSheetComponent],
  // Parents space it only while it shows something.
  host: { '[class.has-offers]': 'offers().length > 0' },
  template: `
    @for (o of offers(); track o.limit.account.id) {
      <div class="offer" role="status">
        <mc-icon name="alert" [size]="16" />
        <p class="text">
          Account <strong>{{ o.limit.account.name }}</strong> has reached its {{ o.limit.name }} —
          {{ o.verb === 'switch' ? 'switch to' : 'continue on' }}
          @if (o.suggestion; as sug) {
            <strong>{{ sug.name }}</strong
            >?
          } @else {
            another account?
          }
          @if (o.resets) {
            <span class="muted">({{ o.resets }})</span>
          }
        </p>
        <div class="acts">
          <button class="btn small primary" type="button" (click)="choose(o)">
            {{
              o.suggestion
                ? (o.verb === 'switch' ? 'Switch to ' : 'Continue on ') + o.suggestion.name
                : 'Choose an account'
            }}
          </button>
          <button
            class="btn small ghost icon-only"
            type="button"
            aria-label="Not now"
            title="Not now"
            (click)="dismiss(o)"
          >
            <mc-icon name="x" [size]="14" />
          </button>
        </div>
      </div>
    }
    <mc-switch-account-sheet
      [open]="!!sheet()"
      [sessions]="sheetSessions()"
      [suggested]="sheet()?.suggestion ?? null"
      (closed)="sheet.set(null)"
    />
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
      }
      .offer {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.4rem 0.6rem;
        padding: 0.5rem 0.6rem 0.5rem 0.75rem;
        border: 1px solid var(--amber-border);
        border-radius: var(--radius-sm);
        background: var(--amber-soft);
        font-size: 0.85rem;
      }
      .offer > mc-icon {
        color: var(--amber);
      }
      .text {
        flex: 1 1 14rem;
        min-width: 0;
        margin: 0;
        line-height: 1.45;
        overflow-wrap: anywhere;
      }
      .acts {
        display: flex;
        align-items: center;
        gap: 0.25rem;
        margin-left: auto;
      }
    `,
  ],
})
export class LimitOfferComponent {
  private accounts = inject(AccountsService);
  private realtime = inject(RealtimeService);
  private clock = inject(NowService);

  /** Speak for this session's account (a session page); else for every
   * account at a limit with conversations here (the sessions list). */
  readonly session = input<SessionSummary | undefined>(undefined);

  /** The offer whose Switch account sheet is open. */
  protected readonly sheet = signal<{ accountId: string; suggestion: string | null } | null>(null);

  protected readonly offers = computed<Offer[]>(() => {
    if (!this.accounts.multi()) return [];
    const now = this.clock.now();
    const offer = (limit: AccountLimit, verb: Offer['verb']): Offer => ({
      limit,
      suggestion: this.accounts.suggestionFor(limit.account),
      resets: resetsIn(limit.window.resetsAt, now),
      verb,
    });
    const s = this.session();
    if (s) {
      const limit = this.accounts.limitOf(s.account?.id);
      const verb = switchMode(s) === 'elsewhere' ? 'continue' : 'switch';
      return limit && !this.accounts.isDismissed(limit) && helped(s) ? [offer(limit, verb)] : [];
    }
    return [...this.accounts.limits().values()]
      .filter((l) => !this.accounts.isDismissed(l) && this.sessionsOf(l.account.id).length > 0)
      .map((l) => offer(l, 'switch'));
  });

  protected readonly sheetSessions = computed<SessionSummary[]>(() => {
    const open = this.sheet();
    if (!open) return [];
    const s = this.session();
    return s ? [s] : this.sessionsOf(open.accountId);
  });

  protected choose(o: Offer): void {
    this.sheet.set({ accountId: o.limit.account.id, suggestion: o.suggestion?.id ?? null });
  }

  protected dismiss(o: Offer): void {
    this.accounts.dismiss(o.limit);
  }

  /** An account's conversations the offer can help: running ones first,
   * then the most recent. */
  private sessionsOf(accountId: string): SessionSummary[] {
    return this.realtime
      .sessions()
      .filter((s) => s.account?.id === accountId && helped(s))
      .sort(
        (a, b) =>
          Number(b.live) - Number(a.live) ||
          (b.lastActivityAt ?? b.startedAt ?? 0) - (a.lastActivityAt ?? a.startedAt ?? 0),
      );
  }
}
