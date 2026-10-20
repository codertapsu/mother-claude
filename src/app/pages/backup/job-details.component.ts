import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { normalizePullResult, normalizePushResult } from '../../core/backup';
import { sentence } from '../../core/data-jobs';
import { JobView } from '../../core/models';
import { shortPath } from '../../shared/util';

/**
 * What a finished restore or pull did beyond its one line: the conflicts
 * (each session and the file written beside its own transcript), the
 * sessions it skipped and why, and its warnings; a backup's failed
 * archives. Nothing for other jobs.
 */
@Component({
  selector: 'mc-job-details',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (conflicts().length) {
      <p class="h small">Changed on both — the backup's copy is kept beside this Mac's:</p>
      <ul>
        @for (c of conflicts(); track c[1]) {
          <li class="small">
            <span class="mono">{{ c[0].slice(0, 8) }}</span> →
            <span class="mono path">{{ short(c[1]) }}</span>
          </li>
        }
      </ul>
    }
    @if (skipped().length) {
      <p class="h small">Skipped:</p>
      <ul>
        @for (s of skipped(); track s.id + s.code) {
          <li class="small">
            @if (s.id) {
              <span class="mono">{{ s.id.slice(0, 8) }}</span>
            }
            {{ s.reason }}
          </li>
        }
      </ul>
    }
    @if (warnings().length) {
      <ul class="warn">
        @for (w of warnings(); track $index) {
          <li class="small">
            @if (w.id) {
              <span class="mono">{{ w.id.slice(0, 8) }}</span>
            }
            {{ w.text }}
          </li>
        }
      </ul>
    }
  `,
  styles: [
    `
      :host {
        display: block;
      }
      .h {
        margin: 0.3rem 0 0.15rem;
        color: var(--muted);
      }
      ul {
        margin: 0;
        padding-left: 1.1rem;
      }
      li {
        overflow-wrap: anywhere;
      }
      .warn li {
        color: var(--amber);
      }
      .path {
        color: var(--muted);
      }
    `,
  ],
})
export class JobDetailsComponent {
  readonly job = input.required<JobView>();

  private readonly done = computed(() => {
    const j = this.job();
    return j.state === 'done' ? j : null;
  });
  protected readonly conflicts = computed(() => {
    const j = this.done();
    return j?.kind === 'pull' ? normalizePullResult(j.result).conflicts : [];
  });
  protected readonly skipped = computed(() => {
    const j = this.done();
    if (j?.kind === 'pull' || j?.kind === 'restore') {
      return normalizePullResult(j.result).skipped.map((s) => ({
        ...s,
        reason: sentence(s.reason) || s.code,
      }));
    }
    if (j?.kind === 'backup') {
      return (normalizePushResult(j.result)?.failed ?? []).map(([id, why]) => ({
        id,
        code: 'failed',
        reason: sentence(why) || 'It could not be archived.',
      }));
    }
    return [];
  });
  protected readonly warnings = computed(() => {
    const j = this.done();
    return j?.kind === 'pull' || j?.kind === 'restore'
      ? normalizePullResult(j.result).warnings
      : [];
  });

  protected short(p: string): string {
    return shortPath(p);
  }
}
