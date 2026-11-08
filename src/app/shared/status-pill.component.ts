import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';

import { tr } from '../core/i18n/i18n';
import { TPipe } from '../core/i18n/t.pipe';
import { Activity, SessionStatus } from '../core/models';
import { RealtimeService } from '../core/realtime.service';
import { IconComponent, IconName } from './icon.component';
import { statusLabel, statusTone } from './util';

/** Each status has its own shape, so colour is never the only signal. */
const STATUS_ICONS: Record<string, IconName> = {
  working: 'activity',
  waiting: 'hand',
  background: 'layers',
  idle: 'pauseCircle',
  starting: 'hourglass',
  ended: 'stopCircle',
  failed: 'xCircle',
  stopped: 'stopCircle',
};

/** Session status: a distinct icon + text; the icon pulses while the
 * process is alive and busy (static under prefers-reduced-motion) and says
 * "live" to screen readers. Given the session's activity, a session that
 * resumes by itself reads "Background work" (pulsing: it still runs) or
 * "Scheduled" (a clock: a wakeup or job will wake it). */
@Component({
  selector: 'mc-status-pill',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, TPipe],
  host: { '[attr.data-tone]': 'tone()', '[attr.title]': 'hint()' },
  template: `<span class="ico" [class.pulse]="pulse()"
      ><mc-icon [name]="icon()" [size]="12" [stroke]="2.4"
    /></span>
    <span class="txt">{{ text() }}</span>
    @if (pulse()) {
      <span class="visually-hidden">{{ ', live' | t }}</span>
    }`,
  styles: [
    `
      :host {
        display: inline-flex;
        align-items: center;
        gap: 0.3rem;
        font-size: 0.75rem;
        font-weight: 600;
        color: var(--tone);
        background: var(--tone-soft);
        border: 1px solid var(--tone-border);
        border-radius: 1rem;
        padding: 0.1rem 0.5rem 0.1rem 0.4rem;
        white-space: nowrap;
        line-height: 1.4;
      }
      .ico {
        display: inline-flex;
        border-radius: 50%;
      }
      .ico.pulse {
        animation: mc-pulse 1.6s ease-out infinite;
      }
    `,
  ],
})
export class StatusPillComponent {
  private realtime = inject(RealtimeService);
  readonly status = input.required<SessionStatus | string>();
  /** The process is alive (drives the pulsing dot). */
  readonly live = input(false);
  /** Override the label. */
  readonly label = input<string | undefined>(undefined);
  /** The session's activity: says which kind of `background` it is. */
  readonly activity = input<Pick<Activity, 'kind'> | undefined>(undefined);

  private readonly scheduled = computed(
    () => this.status() === 'background' && this.activity()?.kind === 'scheduled',
  );
  protected readonly tone = computed(() => statusTone(this.status()));
  protected readonly text = computed(
    () => this.label() ?? statusLabel(this.status(), this.activity()),
  );
  protected readonly icon = computed<IconName>(() =>
    this.scheduled() ? 'clock' : (STATUS_ICONS[this.status()] ?? 'circle'),
  );
  protected readonly hint = computed(() => {
    if (this.status() !== 'background') return null;
    return this.scheduled()
      ? tr('No turn is running: a scheduled wakeup or job resumes this session by itself.')
      : tr(
          'No turn is running: Claude resumes this session by itself when its background work finishes.',
        );
  });
  /** Only while the data is live: a stale page shouldn't look busy. */
  protected readonly pulse = computed(
    () =>
      this.live() &&
      this.realtime.connection() === 'open' &&
      (['working', 'starting', 'waiting'].includes(this.status()) ||
        (this.status() === 'background' && !this.scheduled())),
  );
}
