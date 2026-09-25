import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  ElementRef,
  afterNextRender,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
  viewChild,
  Injector,
} from '@angular/core';

import { ApiService, describeError } from '../../core/api.service';
import { BackgroundTask } from '../../core/models';
import { SheetComponent } from '../../shared/sheet.component';
import { SessionContext } from './session-context';

const REFRESH_MS = 3000;

/** A background task's output tail (last 16 KB), refreshed every few seconds
 * while it runs. Inline: the sheet below and the agent view (for a shell or
 * monitor, which has no transcript of its own) both show it. */
@Component({
  selector: 'mc-task-output-body',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (error()) {
      <p class="error-text small">{{ error() }}</p>
    }
    <pre #out class="out" tabindex="0" aria-label="Task output">{{
      text() || (loading() ? 'Loading…' : 'No output yet.')
    }}</pre>
  `,
  styles: [
    `
      .out {
        margin: 0;
        min-height: 12rem;
        padding: 0.6rem 0.75rem;
        background: var(--code-bg);
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        font-size: 0.75rem;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
      }
    `,
  ],
})
export class TaskOutputBodyComponent {
  private api = inject(ApiService);
  private injector = inject(Injector);

  readonly sessionId = input.required<string>();
  readonly task = input<BackgroundTask | null>(null);

  private readonly out = viewChild<ElementRef<HTMLElement>>('out');
  protected readonly text = signal('');
  protected readonly loading = signal(false);
  protected readonly error = signal('');
  private timer?: ReturnType<typeof setInterval>;

  constructor() {
    effect(() => {
      const t = this.task();
      const sid = this.sessionId();
      untracked(() => {
        clearInterval(this.timer);
        this.text.set('');
        this.error.set('');
        if (!t || !sid) return;
        void this.load(sid, t);
        if (t.status === 'running') {
          this.timer = setInterval(() => {
            if (document.visibilityState === 'visible') void this.load(sid, t);
          }, REFRESH_MS);
        }
      });
    });
    inject(DestroyRef).onDestroy(() => clearInterval(this.timer));
  }

  private async load(sid: string, t: BackgroundTask): Promise<void> {
    this.loading.set(true);
    try {
      const text = await this.api.getTaskOutput(sid, t.id);
      if (this.task()?.id !== t.id || this.sessionId() !== sid) return;
      const el = this.out()?.nativeElement;
      const atEnd = !el || el.scrollHeight - el.scrollTop - el.clientHeight < 40;
      this.text.set(text);
      this.error.set('');
      if (atEnd) {
        afterNextRender(() => el?.scrollIntoView({ block: 'end' }), { injector: this.injector });
      }
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.loading.set(false);
    }
  }
}

/** The output tail in a full-screen sheet. Inside a session page the session
 * comes from its context; elsewhere (the fleet page) pass `sessionId`. */
@Component({
  selector: 'mc-task-output',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [SheetComponent, TaskOutputBodyComponent],
  template: `
    <mc-sheet
      size="full"
      [open]="!!task()"
      [title]="task()?.label ?? 'Output'"
      (closed)="closed.emit()"
    >
      <mc-task-output-body [sessionId]="sid()" [task]="task()" />
    </mc-sheet>
  `,
})
export class TaskOutputComponent {
  private ctx = inject(SessionContext, { optional: true });

  readonly sessionId = input('');
  readonly task = input<BackgroundTask | null>(null);
  readonly closed = output<void>();

  protected readonly sid = computed(() => this.sessionId() || this.ctx?.id() || '');
}
