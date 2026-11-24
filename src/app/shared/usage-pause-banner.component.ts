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
import { UsagePause, stoppedCount, windowText } from '../core/usage-guard';
import { UsageGuardService } from '../core/usage-guard.service';
import { IconComponent } from './icon.component';
import { formatCountdown, resetClock } from './util';

/**
 * Above every page while the usage guard holds work (Settings › Usage
 * limits): which account reached which limit, how many sessions it paused,
 * when they resume (the reset plus a minute, counting down), and *Resume
 * now* / *Don't resume*.
 */
@Component({
  selector: 'mc-usage-pause-banner',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, RouterLink, TPipe],
  template: `
    @for (p of guard.active(); track p.id) {
      <div class="banner warn usage-pause" role="status">
        <mc-icon name="pause" />
        <span class="texts">
          <strong>{{ headline(p) }}</strong>
          <span>{{ detail(p) }}</span>
        </span>
        <span class="acts">
          <button class="btn small" type="button" [disabled]="busy()" (click)="resume(p)">
            {{ 'Resume now' | t }}
          </button>
          <button class="btn small ghost" type="button" [disabled]="busy()" (click)="hold(p)">
            {{ p.hold ? ('Resume by itself' | t) : ("Don't resume" | t) }}
          </button>
          <a class="btn small ghost" routerLink="/settings" fragment="usage-limits">{{
            'Details' | t
          }}</a>
        </span>
      </div>
    }
  `,
  styles: [
    `
      :host {
        display: contents;
      }
      .usage-pause {
        flex-wrap: wrap;
      }
      .texts {
        display: flex;
        flex-direction: column;
        gap: 0.1rem;
        flex: 1 1 16rem;
        min-width: 0;
      }
      .acts {
        display: flex;
        flex-wrap: wrap;
        gap: 0.35rem;
      }
    `,
  ],
})
export class UsagePauseBannerComponent {
  protected readonly guard = inject(UsageGuardService);
  private readonly clock = inject(NowService);
  private readonly notices = inject(NoticeService);
  protected readonly busy = signal(false);
  private readonly now = computed(() => this.clock.now());

  constructor() {
    const release = this.guard.watch();
    inject(DestroyRef).onDestroy(release);
  }

  protected headline(p: UsagePause): string {
    return tr('{account} reached {pct} % of {window}.', {
      account: p.accountName,
      pct: Math.round(p.pct),
      window: windowText(p.window, p.label),
    });
  }

  protected detail(p: UsagePause): string {
    const n = stoppedCount(p);
    const now = this.now();
    const at = { time: resetClock(p.resumeAt, now), left: formatCountdown(p.resumeAt - now) };
    if (p.hold) {
      if (n === 0) return tr('New work waits until you resume it.');
      return n === 1
        ? tr('Mother Claude paused 1 session; it waits until you resume it.')
        : tr('Mother Claude paused {n} sessions; they wait until you resume them.', { n });
    }
    if (n === 0) return tr('New work waits until {time} (in {left}).', at);
    return n === 1
      ? tr('Mother Claude paused 1 session and resumes it at {time} (in {left}).', at)
      : tr('Mother Claude paused {n} sessions and resumes them at {time} (in {left}).', {
          n,
          ...at,
        });
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
