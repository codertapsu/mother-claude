import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  inject,
  signal,
} from '@angular/core';
import { RouterLink } from '@angular/router';

import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { NoticeService } from '../../core/notice.service';
import { NowService } from '../../core/now.service';
import { sessionLink } from '../../core/session-links';
import {
  PausedSession,
  THRESHOLD_MAX,
  THRESHOLD_MIN,
  UsageGuardPatch,
  UsagePause,
  windowText,
} from '../../core/usage-guard';
import { UsageGuardService } from '../../core/usage-guard.service';
import { IconComponent } from '../../shared/icon.component';
import { pausedWhat } from '../../shared/usage-paused';
import { formatCountdown, pastClock, resetClock } from '../../shared/util';

/**
 * *Usage limits* (`claude::usage_guard`): pause an account's sessions when
 * one of its plan windows reaches the threshold (90 % by default), resume
 * them a minute after the window resets; the pauses now and the last few.
 * The desktop or a paired device (`ManageData`).
 */
@Component({
  selector: 'mc-usage-guard-setting',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, RouterLink, TPipe],
  template: `
    @if (guard.view(); as v) {
      <div class="list">
        <label class="switch list-row">
          <input
            type="checkbox"
            [checked]="v.settings.enabled"
            [disabled]="busy()"
            (change)="set({ enabled: $any($event.target).checked })"
          />
          <span class="switch-text">
            <span>{{ 'Pause sessions near a usage limit' | t }}</span>
            <span class="muted small">{{
              'When an account reaches the threshold of its 5-hour, weekly or per-model limit, Mother Claude stops its sessions at a safe point, saves what each was doing, and resumes them after the limit resets.'
                | t
            }}</span>
          </span>
        </label>
        <div class="list-row threshold" [class.off]="!v.settings.enabled">
          <label for="usage-threshold">{{ 'Pause at' | t }}</label>
          <span class="pct">
            <input
              id="usage-threshold"
              type="number"
              inputmode="numeric"
              step="1"
              [min]="min"
              [max]="max"
              [value]="v.settings.thresholdPct"
              [disabled]="busy() || !v.settings.enabled"
              (change)="setThreshold($any($event.target))"
            />
            <span aria-hidden="true">%</span>
          </span>
          <span class="muted small">{{
            'of any limit, from {min} to {max} %.' | t: { min: min, max: max }
          }}</span>
        </div>
        <label class="switch list-row">
          <input
            type="checkbox"
            [checked]="v.settings.autoResume"
            [disabled]="busy() || !v.settings.enabled"
            (change)="set({ autoResume: $any($event.target).checked })"
          />
          <span class="switch-text">
            <span>{{ 'Resume by itself after the reset' | t }}</span>
            <span class="muted small">{{
              'A minute after the limit resets, each paused session is told to go on where it left off. Off: paused work waits until you resume it.'
                | t
            }}</span>
          </span>
        </label>
        <label class="switch list-row">
          <input
            type="checkbox"
            [checked]="v.settings.includeElsewhere"
            [disabled]="busy() || !v.settings.enabled"
            (change)="set({ includeElsewhere: $any($event.target).checked })"
          />
          <span class="switch-text">
            <span>{{ 'Also sessions started elsewhere' | t }}</span>
            <span class="muted small">{{
              'VS Code or a terminal, through the Mother Claude plugin: they stop at their next tool call.'
                | t
            }}</span>
          </span>
        </label>
      </div>

      @if (guard.active().length || guard.ended().length) {
        <h3 class="pauses-head">{{ 'Pauses' | t }}</h3>
        <ul class="pauses">
          @for (p of guard.active(); track p.id) {
            <li class="pause">
              <span class="what">{{ line(p) }}</span>
              <span class="muted small">{{ when(p) }}</span>
              @if (listed(p).length) {
                <ul class="sessions">
                  @for (s of listed(p); track s.id) {
                    <li class="small">
                      <mc-icon
                        [name]="s.released ? 'check' : s.stopped ? 'pause' : 'clock'"
                        [size]="13"
                      />
                      <a [routerLink]="link(s)">{{ s.name }}</a>
                      <span class="muted"> · {{ state(p, s) }}</span>
                    </li>
                  }
                </ul>
              }
              <span class="acts">
                <button class="btn small" type="button" [disabled]="busy()" (click)="resume(p)">
                  {{ 'Resume now' | t }}
                </button>
                <button class="btn small ghost" type="button" [disabled]="busy()" (click)="hold(p)">
                  {{ p.hold ? ('Resume by itself' | t) : ("Don't resume" | t) }}
                </button>
                <button
                  class="btn small ghost"
                  type="button"
                  [disabled]="busy()"
                  (click)="dismiss(p)"
                >
                  {{ 'Dismiss pause' | t }}
                </button>
              </span>
            </li>
          }
          @for (p of guard.ended(); track p.id) {
            <li class="pause ended">
              <span class="what">{{ line(p) }}</span>
              <span class="muted small">{{ endedText(p) }}</span>
              @for (s of failed(p); track s.id) {
                <span class="error-text small">{{ s.name }}: {{ s.resumeError }}</span>
              }
            </li>
          }
        </ul>
      }
    } @else if (guard.error()) {
      <p class="error-text small" role="alert">{{ guard.error() }}</p>
    }
  `,
  styles: [
    `
      .threshold {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.5rem;
      }
      .threshold.off {
        opacity: 0.6;
      }
      .pct {
        display: inline-flex;
        align-items: center;
        gap: 0.25rem;
      }
      .pct input {
        width: 4.5rem;
      }
      .pauses-head {
        margin: 1rem 0 0.4rem;
        font-size: 0.95rem;
      }
      .pauses {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
      }
      .pause {
        display: flex;
        flex-direction: column;
        gap: 0.2rem;
        padding: 0.6rem 0.75rem;
        border: 1px solid var(--border);
        border-radius: 0.5rem;
      }
      .pause.ended {
        opacity: 0.8;
      }
      .what {
        font-weight: 500;
      }
      .sessions {
        list-style: none;
        margin: 0.15rem 0 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
      }
      .sessions li {
        display: flex;
        align-items: baseline;
        flex-wrap: wrap;
        gap: 0 0.3rem;
      }
      .sessions mc-icon {
        align-self: center;
        color: var(--amber);
      }
      .acts {
        display: flex;
        flex-wrap: wrap;
        gap: 0.35rem;
        margin-top: 0.3rem;
      }
    `,
  ],
})
export class UsageGuardSettingComponent {
  protected readonly guard = inject(UsageGuardService);
  private readonly clock = inject(NowService);
  private readonly notices = inject(NoticeService);
  protected readonly busy = signal(false);
  protected readonly min = THRESHOLD_MIN;
  protected readonly max = THRESHOLD_MAX;
  private readonly now = computed(() => this.clock.now());

  constructor() {
    const release = this.guard.watch();
    inject(DestroyRef).onDestroy(release);
  }

  protected line(p: UsagePause): string {
    return tr('{account}: {pct} % of {window}', {
      account: p.accountName,
      pct: Math.round(p.pct),
      window: windowText(p.window, p.label),
    });
  }

  protected when(p: UsagePause): string {
    if (p.hold) return tr('Waits until you resume it.');
    const now = this.now();
    return tr('Resumes at {time} (in {left}).', {
      time: resetClock(p.resumeAt, now),
      left: formatCountdown(p.resumeAt - now),
    });
  }

  protected endedText(p: UsagePause): string {
    const time = pastClock(p.endedAt, this.now());
    switch (p.ended) {
      case 'resumed':
        return tr('Resumed after the reset, at {time}.', { time });
      case 'resumedByHand':
        return tr('Resumed by hand at {time}.', { time });
      default:
        return tr('Dismissed at {time}.', { time });
    }
  }

  /** The sessions it stopped or holds (stopped first), and those someone
   * sent a message to since. */
  protected listed(p: UsagePause): PausedSession[] {
    const rank = (s: PausedSession) => (s.released ? 2 : s.stopped ? 0 : 1);
    return p.sessions.filter((s) => s.stopped || s.held).sort((a, b) => rank(a) - rank(b));
  }

  protected state(p: UsagePause, s: PausedSession): string {
    if (s.released) return tr('you sent it a message: left alone');
    return pausedWhat({ pause: p, session: s, kind: s.stopped ? 'stopped' : 'held' }, this.now());
  }

  protected link(s: PausedSession): string[] {
    return sessionLink(s.id);
  }

  protected failed(p: UsagePause): PausedSession[] {
    return p.sessions.filter((s) => !!s.resumeError);
  }

  protected setThreshold(input: HTMLInputElement): void {
    const n = Math.round(Number(input.value));
    const current = this.guard.view()?.settings.thresholdPct;
    if (!Number.isFinite(n) || n < this.min || n > this.max) {
      if (current !== undefined) input.value = String(current);
      this.notices.warn(
        tr('The threshold is a percentage from {min} to {max}.', { min: this.min, max: this.max }),
      );
      return;
    }
    if (n === current) return;
    void this.set({ thresholdPct: n });
  }

  protected async set(patch: UsageGuardPatch): Promise<void> {
    await this.act(() => this.guard.setSettings(patch), tr('Saved.'));
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

  protected async dismiss(p: UsagePause): Promise<void> {
    await this.act(() => this.guard.dismiss(p.id), tr('Dismissed.'));
  }

  private async act(run: () => Promise<void>, done: string): Promise<void> {
    if (this.busy()) return;
    this.busy.set(true);
    try {
      await run();
      this.notices.success(done);
    } catch (e) {
      this.notices.error(e);
      void this.guard.load();
    } finally {
      this.busy.set(false);
    }
  }
}
