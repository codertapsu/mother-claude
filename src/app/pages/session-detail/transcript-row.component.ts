import { ChangeDetectionStrategy, Component, computed, input, signal } from '@angular/core';

import { UiEvent } from '../../core/models';
import { IconComponent, IconName } from '../../shared/icon.component';
import { renderMarkdown } from '../../shared/markdown';
import { MediaImageComponent } from '../../shared/media-image.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';
import { ToolStepComponent } from './tool-step.component';
import { Row, ToolStep, stepsSummary } from './transcript-rows';

const SYSTEM_ICONS: Partial<Record<UiEvent['kind'], IconName>> = {
  notice: 'info',
  compact: 'layers',
  error: 'alert',
  task: 'activity',
  decision: 'shield',
  result: 'terminal',
};

/** Long plans start folded. */
const PLAN_FOLD_LINES = 16;

/**
 * One transcript row: a prompt, markdown text, collapsed thinking, a tool
 * call, a collapsed run of steps ("Ran 12 steps · last: Bash npm test", the
 * newest step stays visible), an image, todos, a question with its recorded
 * answers, a plan, a relayed message, a decision, or a system line. OnPush
 * with a stable row object, so appending events doesn't re-render (or
 * re-parse the markdown of) the rows above.
 */
@Component({
  selector: 'mc-transcript-row',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, MediaImageComponent, TimeAgoComponent, ToolStepComponent],
  host: { '[attr.data-row]': 'row().id' },
  templateUrl: './transcript-row.component.html',
  styleUrl: './transcript-row.component.scss',
})
export class TranscriptRowComponent {
  readonly row = input.required<Row>();
  readonly sessionId = input('');
  readonly cwd = input('');
  /** "via hook" / "via tmux" for relayed messages. */
  readonly relayVia = input('');
  /** toolUseId → agent id, to link Agent calls to their transcripts. */
  readonly agentLinks = input<ReadonlyMap<string, string>>(new Map());
  /** The session is still running (unanswered calls spin). */
  readonly running = input(false);

  protected readonly groupOpen = signal(false);
  protected readonly planOpen = signal(false);

  protected readonly ev = computed(() => {
    const r = this.row();
    return r.type === 'event' ? r.ev : undefined;
  });
  protected readonly step = computed<ToolStep | undefined>(() => {
    const r = this.row();
    return r.type === 'tool' ? r.step : undefined;
  });
  protected readonly steps = computed<ToolStep[]>(() => {
    const r = this.row();
    return r.type === 'steps' ? r.steps : [];
  });
  protected readonly summary = computed(() => stepsSummary(this.steps()));
  protected readonly lastStep = computed(() => this.steps()[this.steps().length - 1]);
  protected readonly failed = computed(
    () => this.steps().filter((s) => s.result?.result?.isError).length,
  );
  protected readonly html = computed(() => {
    const e = this.ev();
    if (!e) return '';
    if (e.kind === 'text' || e.kind === 'thinking') return renderMarkdown(e.text ?? '');
    if (e.kind === 'plan') return renderMarkdown(e.plan?.text ?? e.text ?? '');
    return '';
  });
  protected readonly planLong = computed(
    () => (this.ev()?.plan?.text ?? this.ev()?.text ?? '').split('\n').length > PLAN_FOLD_LINES,
  );

  protected systemIcon(e: UiEvent): IconName {
    return SYSTEM_ICONS[e.kind] ?? 'info';
  }

  protected agentFor(s: ToolStep): string | undefined {
    return s.tool.tool?.id ? this.agentLinks().get(s.tool.tool.id) : undefined;
  }

  protected answer(e: UiEvent, question: string): string | undefined {
    return e.question?.answers?.[question];
  }
}
