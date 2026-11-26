import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { tr } from '../core/i18n/i18n';
import { TPipe } from '../core/i18n/t.pipe';
import { NoticeService } from '../core/notice.service';
import { NowService } from '../core/now.service';
import { UsageGuardService } from '../core/usage-guard.service';
import { IconComponent } from './icon.component';
import { pausedWhat, pausedWhy } from './usage-paused';

/**
 * Under a session page's header while the usage guard holds that session
 * (Settings › Usage limits): why — its work was stopped, or its scheduled
 * run waits — and when it goes on, with *Resume now* (its whole pause) and
 * *Details*. A message sent from the page goes through and leaves it alone.
 * Nothing for a session that is not held.
 */
@Component({
  selector: 'mc-usage-paused-line',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, RouterLink, TPipe],
  template: `
    @if (info(); as x) {
      <div class="upl" role="status">
        <mc-icon [name]="x.kind === 'held' ? 'clock' : 'pause'" [size]="16" />
        <span class="txt">
          <span>{{ detail() }}</span>
          <span class="what">{{ what() }}</span>
        </span>
        <span class="acts">
          <button class="btn small" type="button" [disabled]="busy()" (click)="resume()">
            {{ 'Resume now' | t }}
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
        display: block;
      }
      .upl {
        margin: 0 0.25rem 0.35rem;
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.35rem 0.6rem;
        padding: 0.4rem 0.65rem;
        border: 1px solid var(--amber-border);
        border-left: 4px solid var(--amber);
        border-radius: var(--radius-sm);
        background: var(--amber-soft);
        font-size: 0.85rem;
      }
      .upl mc-icon {
        flex: 0 0 auto;
        color: var(--amber);
      }
      .txt {
        flex: 1 1 14rem;
        min-width: 0;
        display: flex;
        flex-direction: column;
      }
      .what {
        font-size: 0.8rem;
        color: var(--muted);
      }
      .acts {
        display: flex;
        flex-wrap: wrap;
        gap: 0.35rem;
      }
    `,
  ],
})
export class UsagePausedLineComponent {
  private readonly guard = inject(UsageGuardService);
  private readonly clock = inject(NowService);
  private readonly notices = inject(NoticeService);

  /** The session (this computer's). */
  readonly session = input.required<string>();

  protected readonly busy = signal(false);
  protected readonly info = computed(() => this.guard.pausedFor(this.session()));
  protected readonly detail = computed(() => {
    const x = this.info();
    return x ? pausedWhy(x, this.clock.now()) : '';
  });
  protected readonly what = computed(() => {
    const x = this.info();
    return x ? pausedWhat(x, this.clock.now()) : '';
  });

  protected async resume(): Promise<void> {
    const x = this.info();
    if (!x || this.busy()) return;
    this.busy.set(true);
    try {
      await this.guard.resume(x.pause.id);
      this.notices.success(tr('Resumed.'));
    } catch (e) {
      this.notices.error(e);
    } finally {
      this.busy.set(false);
    }
  }
}
