import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  ElementRef,
  afterNextRender,
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

/** A background task's output tail (last 16 KB), refreshed every few
 * seconds while it runs and the sheet is open. */
@Component({
  selector: 'mc-task-output',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [SheetComponent],
  template: `
    <mc-sheet
      size="full"
      [open]="!!task()"
      [title]="task()?.label ?? 'Output'"
      (closed)="closed.emit()"
    >
      @if (error()) {
        <p class="error-text small">{{ error() }}</p>
      }
      <pre #out class="out" tabindex="0" aria-label="Task output">{{
        text() || (loading() ? 'Loading…' : 'No output yet.')
      }}</pre>
    </mc-sheet>
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
export class TaskOutputComponent {
  private api = inject(ApiService);
  private ctx = inject(SessionContext);
  private injector = inject(Injector);

  readonly task = input<BackgroundTask | null>(null);
  readonly closed = output<void>();

  private readonly out = viewChild<ElementRef<HTMLElement>>('out');
  protected readonly text = signal('');
  protected readonly loading = signal(false);
  protected readonly error = signal('');
  private timer?: ReturnType<typeof setInterval>;

  constructor() {
    effect(() => {
      const t = this.task();
      untracked(() => {
        clearInterval(this.timer);
        this.text.set('');
        this.error.set('');
        if (!t) return;
        void this.load(t);
        if (t.status === 'running') {
          this.timer = setInterval(() => {
            if (document.visibilityState === 'visible') void this.load(t);
          }, REFRESH_MS);
        }
      });
    });
    inject(DestroyRef).onDestroy(() => clearInterval(this.timer));
  }

  private async load(t: BackgroundTask): Promise<void> {
    this.loading.set(true);
    try {
      const text = await this.api.getTaskOutput(this.ctx.id(), t.id);
      if (this.task()?.id !== t.id) return;
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
