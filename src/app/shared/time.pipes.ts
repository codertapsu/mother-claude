import { Pipe, PipeTransform, inject } from '@angular/core';

import { NowService } from '../core/now.service';
import { formatDuration, relativeTime } from './util';

/**
 * `{{ ts | relTime }}` → "12s ago", re-rendered every second by the shared
 * clock (impure on purpose: it reads the `now` signal).
 */
@Pipe({ name: 'relTime', pure: false })
export class RelativeTimePipe implements PipeTransform {
  private clock = inject(NowService);

  transform(ms: number | undefined | null): string {
    return relativeTime(ms ?? undefined, this.clock.now());
  }
}

/** `{{ since | elapsed }}` → "3m 04s" since the timestamp (ticks), or
 * `{{ start | elapsed: end }}` for a fixed span. */
@Pipe({ name: 'elapsed', pure: false })
export class ElapsedPipe implements PipeTransform {
  private clock = inject(NowService);

  transform(since: number | undefined | null, until?: number | null): string {
    if (!since) return '';
    const end = until ?? this.clock.now();
    return formatDuration(Math.max(0, end - since));
  }
}
