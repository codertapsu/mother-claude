import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  inject,
  signal,
} from '@angular/core';
import { RouterLink } from '@angular/router';

import { tr } from '../core/i18n/i18n';
import { TPipe } from '../core/i18n/t.pipe';
import { NoticeService } from '../core/notice.service';
import { NowService } from '../core/now.service';
import { sessionLink } from '../core/session-links';
import { PausedInfo, UsagePause, pausedSessions, windowText } from '../core/usage-guard';
import { UsageGuardService } from '../core/usage-guard.service';
import { IconComponent } from './icon.component';
import { pausedWhat } from './usage-paused';
import { formatCountdown, resetClock } from './util';

/** Sessions named in a banner before "+N more". */
const SHOWN = 5;

/**
 * Above every page while the usage guard holds work (Settings › Usage
 * limits): which account reached which limit, when its work resumes (the
 * reset plus a minute, counting down), the sessions it holds — each a link
 * to its page, ⏸ for one whose work was stopped, 🕑 for one whose
 * scheduled run waits, the rest in Settings — and *Resume now* / *Don't
 * resume*.
 */
@Component({
  selector: 'mc-usage-pause-banner',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, RouterLink, TPipe],
  template: `
    @for (p of guard.active(); track p.id) {
      @let list = sessions(p);
      <section class="ub" role="status" [attr.aria-label]="headline(p)">
        <mc-icon class="ub-icon" name="pause" />
        <div class="ub-body">
          <p class="ub-head">{{ headline(p) }}</p>
          <p class="ub-when">{{ when(p, list.length) }}</p>
          @if (list.length) {
            <ul class="ub-list" [attr.aria-label]="'Paused sessions' | t">
              @for (x of list.slice(0, shown); track x.session.id) {
                <li>
                  <a
                    class="ub-chip"
                    [class.held]="x.kind === 'held'"
                    [routerLink]="link(x)"
                    [title]="what(x)"
                  >
                    <mc-icon [name]="x.kind === 'held' ? 'clock' : 'pause'" [size]="12" />
                    <span class="nm">{{ x.session.name }}</span>
                    <span class="visually-hidden">: {{ what(x) }}</span>
                  </a>
                </li>
              }
              @if (list.length > shown) {
                <li>
                  <a class="ub-more" routerLink="/settings" fragment="usage-limits">{{
                    '+{n} more' | t: { n: list.length - shown }
                  }}</a>
                </li>
              }
            </ul>
          }
        </div>
        <div class="ub-acts">
          <button class="btn small" type="button" [disabled]="busy()" (click)="resume(p)">
            {{ 'Resume now' | t }}
          </button>
          <button class="btn small ghost" type="button" [disabled]="busy()" (click)="hold(p)">
            {{ p.hold ? ('Resume by itself' | t) : ("Don't resume" | t) }}
          </button>
          <a class="btn small ghost" routerLink="/settings" fragment="usage-limits">{{
            'Details' | t
          }}</a>
        </div>
      </section>
    }
  `,
  styles: [
    `
      :host {
        display: block;
      }
      /* The app's warning banner, drawn here: the shell's own banner styles
         stay inside the shell's template. */
      .ub {
        display: flex;
        align-items: flex-start;
        flex-wrap: wrap;
        gap: 0.5rem 0.75rem;
        padding: 0.65rem 1rem;
        font-size: 0.9rem;
        line-height: 1.45;
        background: var(--amber-soft);
        border-bottom: 1px solid var(--amber-border);
        border-left: 5px solid var(--amber);
      }
      .ub-icon {
        flex: 0 0 auto;
        margin-top: 0.1rem;
        color: var(--amber);
      }
      .ub-body {
        flex: 1 1 18rem;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
      }
      .ub-head {
        margin: 0;
        font-weight: 600;
      }
      .ub-when {
        margin: 0;
        font-size: 0.85rem;
        color: var(--muted);
        font-variant-numeric: tabular-nums;
      }
      .ub-list {
        list-style: none;
        margin: 0.3rem 0 0;
        padding: 0;
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0.3rem;
      }
      .ub-chip {
        display: inline-flex;
        align-items: center;
        gap: 0.3rem;
        max-width: 15rem;
        min-height: 26px;
        padding: 0.1rem 0.6rem;
        border: 1px solid var(--amber-border);
        border-radius: 999px;
        background: var(--panel);
        color: var(--text);
        font-size: 0.8rem;
        font-weight: 500;
        text-decoration: none;
      }
      .ub-chip:hover,
      .ub-chip:focus-visible {
        border-color: var(--amber);
      }
      /* A scheduled run that waits: nothing was stopped. */
      .ub-chip.held {
        border-style: dashed;
      }
      .ub-chip mc-icon {
        flex: 0 0 auto;
        color: var(--amber);
      }
      .nm {
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .ub-more {
        font-size: 0.8rem;
        color: var(--muted);
      }
      .ub-acts {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0.35rem;
      }
      /* Phones: the buttons go under the text, lined up with it. */
      @media (max-width: 767px) {
        .ub-acts {
          flex-basis: 100%;
          padding-left: calc(20px + 0.75rem);
        }
      }
    `,
  ],
})
export class UsagePauseBannerComponent {
  protected readonly guard = inject(UsageGuardService);
  private readonly clock = inject(NowService);
  private readonly notices = inject(NoticeService);
  protected readonly busy = signal(false);
  protected readonly shown = SHOWN;
  private readonly now = computed(() => this.clock.now());

  constructor() {
    const release = this.guard.watch();
    inject(DestroyRef).onDestroy(release);
  }

  protected sessions(p: UsagePause): PausedInfo[] {
    return pausedSessions(p);
  }

  protected headline(p: UsagePause): string {
    return tr('{account} reached {pct} % of {window}.', {
      account: p.accountName,
      pct: Math.round(p.pct),
      window: windowText(p.window, p.label),
    });
  }

  /** When its work goes on; with no session held, new work waits. */
  protected when(p: UsagePause, held: number): string {
    if (p.hold) {
      return held ? tr('Waits until you resume it.') : tr('New work waits until you resume it.');
    }
    const now = this.now();
    const at = { time: resetClock(p.resumeAt, now), left: formatCountdown(p.resumeAt - now) };
    return held
      ? tr('Resumes at {time} (in {left}).', at)
      : tr('New work waits until {time} (in {left}).', at);
  }

  protected what(x: PausedInfo): string {
    return pausedWhat(x, this.now());
  }

  protected link(x: PausedInfo): string[] {
    return sessionLink(x.session.id);
  }

  protected async resume(p: UsagePause): Promise<void> {
    await this.act(() => this.guard.resume(p.id), tr('Resumed.'));
  }

  protected async hold(p: UsagePause): Promise<void> {
    await this.act(
      () => this.guard.hold(p.id, !p.hold),
      p.hold ? tr('It resumes by itself after the reset.') : tr('It waits until you resume it.'),
    );
  }

  private async act(run: () => Promise<void>, done: string): Promise<void> {
    if (this.busy()) return;
    this.busy.set(true);
    try {
      await run();
      this.notices.success(done);
    } catch (e) {
      this.notices.error(e);
    } finally {
      this.busy.set(false);
    }
  }
}
