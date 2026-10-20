import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  input,
  linkedSignal,
  output,
  signal,
} from '@angular/core';

import { ApiService } from '../core/api.service';
import { AuthService } from '../core/auth.service';
import { jobSummary } from '../core/backup';
import {
  canUndo,
  formatBytes,
  jobFraction,
  jobKindLabel,
  jobStateLabel,
  normalizeJob,
  transferErrorText,
} from '../core/data-jobs';
import { JobView } from '../core/models';
import { NowService } from '../core/now.service';
import { IconComponent, IconName } from './icon.component';
import { TimeAgoComponent } from './time-ago.component';

const ICONS: Record<JobView['kind'], IconName> = {
  move: 'move',
  copy: 'copy',
  backup: 'upload',
  restore: 'download',
  pull: 'download',
};

/** "28 Oct, 14:30". */
function dateTime(ms: number): string {
  const d = new Date(ms);
  return `${d.toLocaleDateString(undefined, { day: 'numeric', month: 'short' })}, ${d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })}`;
}

/**
 * One data job (a move, copy, backup, restore or pull): what it is, how far
 * it got (a progress bar while it runs), one line about how it ended, and —
 * on the desktop, while its undo has not run out — Undo, confirmed with a
 * second tap. Emits the job as it is after an undo. Extra details (a pull's
 * conflicts, a restore's skips) go in as content.
 */
@Component({
  selector: 'mc-data-job',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, TimeAgoComponent],
  host: { '[attr.data-state]': 'job().state' },
  template: `
    @let j = job();
    <div class="jhead">
      <mc-icon class="kind" [name]="icon()" [size]="15" />
      <strong class="jtitle">{{ label() || kindLabel() }}</strong>
      <span class="jstate">{{ stateLabel() }}</span>
      <mc-time class="when muted small" [ms]="j.finishedAt ?? j.startedAt" />
    </div>
    @if (j.state === 'running') {
      <div
        class="bar"
        role="progressbar"
        aria-valuemin="0"
        aria-valuemax="100"
        [attr.aria-valuenow]="pct()"
        [attr.aria-label]="(label() || kindLabel()) + ': progress'"
      >
        <span class="fill" [class.unknown]="pct() === null" [style.width.%]="pct() ?? 35"></span>
      </div>
      @if (progress()) {
        <span class="muted small">{{ progress() }}</span>
      }
    } @else if (summary()) {
      <p class="small sum" [class.error-text]="j.state === 'failed'">{{ summary() }}</p>
    }
    <ng-content />
    @if (undoable()) {
      @if (isDesktop()) {
        <div class="undo">
          <button
            class="btn small"
            type="button"
            [class.danger]="confirming()"
            [disabled]="undoing()"
            (click)="undo()"
          >
            <mc-icon name="undo" [size]="14" />
            {{ undoing() ? 'Undoing…' : confirming() ? 'Tap again to undo' : 'Undo' }}
          </button>
          <span class="muted small">Possible until {{ until() }}.</span>
        </div>
      } @else {
        <p class="muted small sum">It can be undone on the Mac until {{ until() }}.</p>
      }
    }
    @if (undoError()) {
      <p class="error-text small sum" role="alert">{{ undoError() }}</p>
    }
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.3rem;
        min-width: 0;
      }
      .jhead {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.2rem 0.5rem;
        min-width: 0;
      }
      .kind {
        color: var(--muted);
      }
      .jtitle {
        min-width: 0;
        overflow-wrap: anywhere;
      }
      .jstate {
        font-size: 0.72rem;
        padding: 0.05rem 0.4rem;
        border-radius: 0.4rem;
        background: var(--panel-3);
        color: var(--muted);
      }
      :host([data-state='running']) .jstate {
        background: var(--accent-soft);
        color: var(--accent-text);
      }
      :host([data-state='done']) .jstate {
        background: var(--green-soft);
        color: var(--green);
      }
      :host([data-state='failed']) .jstate {
        background: var(--red-soft);
        color: var(--red);
      }
      .when {
        margin-left: auto;
      }
      .bar {
        height: 0.4rem;
        border-radius: 0.2rem;
        background: var(--panel-3);
        overflow: hidden;
      }
      .fill {
        display: block;
        height: 100%;
        border-radius: inherit;
        background: var(--accent);
        transition: width 0.3s ease;
      }
      .fill.unknown {
        animation: mc-shimmer 1.2s ease-in-out infinite alternate;
      }
      .sum {
        margin: 0;
        overflow-wrap: anywhere;
      }
      .undo {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.4rem 0.6rem;
      }
    `,
  ],
})
export class DataJobComponent {
  private api = inject(ApiService);
  private clock = inject(NowService);
  protected readonly isDesktop = inject(AuthService).isDesktop;

  readonly job = input.required<JobView>();
  /** What it is, e.g. "Backup of Work" (default: its kind). */
  readonly label = input('');
  /** The job as it is after an undo. */
  readonly changed = output<JobView>();

  protected readonly undoing = signal(false);
  protected readonly confirming = linkedSignal({
    source: () => this.job().id,
    computation: () => false,
  });
  protected readonly undoError = linkedSignal({
    source: () => this.job().id,
    computation: () => '',
  });
  private confirmTimer?: ReturnType<typeof setTimeout>;

  protected readonly icon = computed(() => ICONS[this.job().kind]);
  protected readonly kindLabel = computed(() => jobKindLabel(this.job().kind));
  protected readonly stateLabel = computed(() => jobStateLabel(this.job().state));
  protected readonly pct = computed(() => {
    const f = jobFraction(this.job());
    return f === null ? null : Math.round(f * 100);
  });
  protected readonly progress = computed(() => {
    const j = this.job();
    if (j.bytesTotal > 0) return `${formatBytes(j.bytesDone)} of ${formatBytes(j.bytesTotal)}`;
    if (j.filesTotal > 0) return `${j.filesDone} of ${j.filesTotal} files`;
    return 'Working…';
  });
  protected readonly summary = computed(() => jobSummary(this.job()));
  protected readonly undoable = computed(() => canUndo(this.job(), this.clock.now()));
  protected readonly until = computed(() => dateTime(this.job().undoUntil ?? 0));

  protected async undo(): Promise<void> {
    if (this.undoing()) return;
    if (!this.confirming()) {
      this.confirming.set(true);
      clearTimeout(this.confirmTimer);
      this.confirmTimer = setTimeout(() => this.confirming.set(false), 4000);
      return;
    }
    clearTimeout(this.confirmTimer);
    this.confirming.set(false);
    this.undoing.set(true);
    this.undoError.set('');
    try {
      const next = normalizeJob(await this.api.undoDataJob(this.job().id));
      this.changed.emit(next ?? { ...this.job(), state: 'undone', undoUntil: undefined });
    } catch (e) {
      this.undoError.set(`Not undone: ${transferErrorText(e)}`);
    } finally {
      this.undoing.set(false);
    }
  }
}
