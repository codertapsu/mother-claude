import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';

import { normalizePullResult, normalizePushResult } from '../../core/backup';
import { plural, sentence } from '../../core/data-jobs';
import { JobView } from '../../core/models';
import { RealtimeService } from '../../core/realtime.service';
import { sessionName, shortPath } from '../../shared/util';

/** A session in a list: its id prefix, and its name when the app knows it. */
interface Named {
  id: string;
  short: string;
  name: string;
}

/**
 * What a finished backup, restore or pull did beyond its one line. A
 * backup: the sessions it did not push because another Mac changed them
 * too (a Pull brings the other copy beside this one), those newer in the
 * backup (a Pull brings them), what failed or was left out, and the old
 * files it removed. A restore or pull: the conflicts (each session and the
 * file written beside its own transcript), the sessions it skipped and why,
 * and its warnings. Nothing for other jobs.
 */
@Component({
  selector: 'mc-job-details',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (pushConflicts().length) {
      <p class="h small">
        Changed here and on another Mac, so not backed up — a Pull brings the other copy beside this
        one:
      </p>
      <ul>
        @for (s of pushConflicts(); track s.id) {
          <li class="small">
            <span class="mono">{{ s.short }}</span>
            @if (s.name) {
              {{ s.name }}
            }
          </li>
        }
      </ul>
    }
    @if (newerInSet().length) {
      <p class="h small">Newer in the backup (another Mac backed them up) — a Pull brings them:</p>
      <ul>
        @for (s of newerInSet(); track s.id) {
          <li class="small">
            <span class="mono">{{ s.short }}</span>
            @if (s.name) {
              {{ s.name }}
            }
          </li>
        }
      </ul>
    }
    @if (conflicts().length) {
      <p class="h small">Changed on both — the backup's copy is kept beside this Mac's:</p>
      <ul>
        @for (c of conflicts(); track c.path) {
          <li class="small">
            <span class="mono">{{ c.session.short }}</span>
            @if (c.session.name) {
              {{ c.session.name }}
            }
            → <span class="mono path">{{ short(c.path) }}</span>
          </li>
        }
      </ul>
    }
    @if (skipped().length) {
      <p class="h small">{{ push() ? 'Could not be backed up:' : 'Skipped:' }}</p>
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
    @if (removed()) {
      <p class="muted small gone">{{ removed() }}</p>
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
        line-height: 1.45;
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
      .gone {
        margin: 0.3rem 0 0;
      }
    `,
  ],
})
export class JobDetailsComponent {
  private realtime = inject(RealtimeService);

  readonly job = input.required<JobView>();

  private readonly done = computed(() => {
    const j = this.job();
    return j.state === 'done' ? j : null;
  });
  protected readonly push = computed(() => {
    const j = this.done();
    return j?.kind === 'backup' ? normalizePushResult(j.result) : null;
  });
  /** What the app calls each session it knows (its live list). */
  private readonly names = computed(
    () => new Map(this.realtime.sessions().map((s) => [s.id, sessionName(s)])),
  );

  protected readonly pushConflicts = computed(() =>
    (this.push()?.conflicts ?? []).map((id) => this.named(id)),
  );
  protected readonly newerInSet = computed(() =>
    (this.push()?.newerInSet ?? []).map((id) => this.named(id)),
  );
  protected readonly conflicts = computed(() => {
    const j = this.done();
    return j?.kind === 'pull'
      ? normalizePullResult(j.result).conflicts.map(([id, path]) => ({
          session: this.named(id),
          path,
        }))
      : [];
  });
  protected readonly skipped = computed(() => {
    const j = this.done();
    if (j?.kind === 'pull' || j?.kind === 'restore') {
      return normalizePullResult(j.result).skipped.map((s) => ({
        ...s,
        reason: sentence(s.reason) || s.code,
      }));
    }
    return (this.push()?.failed ?? []).map(([id, why]) => ({
      id,
      code: 'failed',
      reason: sentence(why) || 'It could not be archived.',
    }));
  });
  protected readonly warnings = computed(() => {
    const j = this.done();
    if (j?.kind === 'pull' || j?.kind === 'restore') return normalizePullResult(j.result).warnings;
    // A backup's warnings start with the file's path: kept as they are.
    return (this.push()?.warnings ?? []).map((text) => ({ id: '', text: fullStop(text) }));
  });
  protected readonly removed = computed(() => {
    const n = this.push()?.removed ?? 0;
    return n ? `Removed ${plural(n, 'old file')} the backup no longer names.` : '';
  });

  protected short(p: string): string {
    return shortPath(p);
  }

  private named(id: string): Named {
    const name = this.names().get(id) ?? '';
    return { id, short: id.slice(0, 8), name: name && name !== id.slice(0, 8) ? name : '' };
  }
}

/** `text` ending in a full stop, its first letter as it was (it may be a
 * path). */
function fullStop(text: string): string {
  const t = text.trim();
  return /[.!?…]$/.test(t) ? t : `${t}.`;
}
