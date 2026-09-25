import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';

import { NowService } from '../core/now.service';
import { formatDuration, relativeTime } from './util';

/**
 * `<mc-time [ms]="ts" />` → `<time datetime="…" title="25 Sep 2026, 14:03">3m
 * ago</time>`: a relative time that ticks, with the absolute time for
 * tooltips and assistive tech. `mode="elapsed"` shows "3m 04s" since `ms`
 * (or until `until`); `prefix` goes in front ("waiting 3m").
 */
@Component({
  selector: 'mc-time',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `@if (ms()) {
    <time [attr.datetime]="iso()" [title]="absolute()">{{ prefix() }}{{ text() }}</time>
  }`,
  styles: [
    `
      :host {
        font-variant-numeric: tabular-nums;
        white-space: nowrap;
      }
    `,
  ],
})
export class TimeAgoComponent {
  private clock = inject(NowService);
  readonly ms = input<number | undefined | null>(undefined);
  readonly until = input<number | undefined | null>(undefined);
  readonly mode = input<'relative' | 'elapsed'>('relative');
  readonly prefix = input('');

  protected readonly iso = computed(() => {
    const ms = this.ms();
    return ms ? new Date(ms).toISOString() : '';
  });
  protected readonly absolute = computed(() => {
    const ms = this.ms();
    return ms ? new Date(ms).toLocaleString() : '';
  });
  protected readonly text = computed(() => {
    const ms = this.ms();
    if (!ms) return '';
    if (this.mode() === 'elapsed') {
      const end = this.until() ?? this.clock.now();
      return formatDuration(Math.max(0, end - ms));
    }
    return relativeTime(ms, this.clock.now());
  });
}
