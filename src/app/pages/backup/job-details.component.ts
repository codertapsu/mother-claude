import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';

import { normalizePullResult, normalizePushResult } from '../../core/backup';
import { plural, sentence } from '../../core/data-jobs';
import { FolderPairDraftService } from '../../core/folder-pair-draft.service';
import { ForeignProject, JobView } from '../../core/models';
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
 * backup: the sessions it did not push because another machine changed them
 * too (a Pull brings the other copy beside this one), those newer in the
 * backup (a Pull brings them), what failed or was left out, and the old
 * files it removed. A restore or pull: the conflicts (each session and the
 * file written beside its own transcript), the sessions it skipped and why,
 * and its warnings. Nothing for other jobs. A pull or restore also says
 * what a folder pair moved into the folder here, and which projects kept
 * another machine's path (with "Pair a folder…", which fills the Folder
 * pairs form); a force job, what it replaced.
 */
@Component({
  selector: 'mc-job-details',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (relocated().length) {
      <p class="h small">Moved into the folder a folder pair maps them to here:</p>
      <ul>
        @for (s of relocated(); track s.id) {
          <li class="small">
            <span class="mono">{{ s.short }}</span>
            @if (s.name) {
              {{ s.name }}
            }
          </li>
        }
      </ul>
    }
    @if (foreign().length) {
      <p class="h small">
        Kept another machine's path — no folder pair maps these projects, so Claude Code here lists
        them only under that path. Pair the folder, then pull again, and they move into the matching
        project here:
      </p>
      <ul class="foreign">
        @for (f of foreign(); track f.folder + f.machine) {
          <li class="small">
            <span class="mono path">{{ f.folder }}</span>
            <span class="muted"> from {{ f.host || 'another machine' }}</span>
            <button class="btn small ghost pair" type="button" (click)="offerPair(f)">
              Pair a folder…
            </button>
          </li>
        }
      </ul>
    }
    @if (replaced().length) {
      <p class="h small">{{ replacedLabel() }}</p>
      <ul class="replaced">
        @for (s of replaced(); track s.id) {
          <li class="small">
            <span class="mono">{{ s.short }}</span>
            @if (s.name) {
              {{ s.name }}
            }
          </li>
        }
      </ul>
    }
    @if (pushConflicts().length) {
      <p class="h small">
        Changed here and on another machine, so not backed up — a Pull brings the other copy beside
        this one:
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
      <p class="h small">
        Newer in the backup (another machine backed them up) — a Pull brings them:
      </p>
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
      <p class="h small">Changed on both — the backup's copy is kept beside the one here:</p>
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
      .foreign li {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0 0.4rem;
      }
    `,
  ],
})
export class JobDetailsComponent {
  private realtime = inject(RealtimeService);
  private drafts = inject(FolderPairDraftService);

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
  /** A pull or restore: what a folder pair moved into the folder here. */
  protected readonly relocated = computed(() => {
    const j = this.done();
    return j?.kind === 'pull'
      ? normalizePullResult(j.result).relocated.map((id) => this.named(id))
      : [];
  });
  /** A pull or restore: projects that kept another machine's path. */
  protected readonly foreign = computed(() => {
    const j = this.done();
    return j?.kind === 'pull' || j?.kind === 'restore' ? normalizePullResult(j.result).foreign : [];
  });

  protected offerPair(f: ForeignProject): void {
    this.drafts.offer(f.machine, f.folder);
  }

  /** What a force backup, pull or restore replaced. */
  protected readonly replaced = computed(() => {
    const j = this.done();
    if (j?.kind === 'pull' || j?.kind === 'restore') {
      return normalizePullResult(j.result).replaced.map((id) => this.named(id));
    }
    return (this.push()?.replaced ?? []).map((id) => this.named(id));
  });
  protected readonly replacedLabel = computed(() =>
    this.done()?.kind === 'backup'
      ? "The copy here replaced the backup's, which another machine had backed up since this one last had it (or which was newer); the folder keeps the replaced copies seven days:"
      : "The backup's copy replaced the one here, which is kept for undo:",
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
