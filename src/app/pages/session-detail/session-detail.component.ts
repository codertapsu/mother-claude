import { Component, ElementRef, computed, effect, inject, signal, viewChild } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { FormsModule } from '@angular/forms';
import { ActivatedRoute, Router, RouterLink } from '@angular/router';
import { map } from 'rxjs';

import { ApiService } from '../../core/api.service';
import { RealtimeService } from '../../core/realtime.service';
import {
  ContentBlock,
  GitOverview,
  QuestionOption,
  Session,
  TranscriptEvent,
} from '../../core/models';
import { renderMarkdown } from '../../shared/markdown';
import { relativeTime, stateLabel, surfaceLabel, toolSummary } from '../../shared/util';

interface RenderedTool {
  name: string;
  summary: string;
}

interface RenderedQuestion {
  question: string;
  header?: string;
  multiSelect: boolean;
  options: QuestionOption[];
  /** The user's answer, when a later tool_result resolved this question. */
  answer?: string;
}

interface RenderedEvent {
  who: string;
  cls: string;
  text: string;
  /** Markdown-rendered HTML (assistant prose only); bound via [innerHTML]. */
  html: string;
  tools: RenderedTool[];
  questions: RenderedQuestion[];
  /** Markdown-rendered plan proposals (ExitPlanMode). */
  plans: string[];
  error: boolean;
}

const MAX_RENDER = 800;

/** Tools that pose a question to the human (rendered as question cards). */
function isQuestionTool(name: string | undefined): boolean {
  return name === 'AskUserQuestion' || name === 'ask_user' || !!name?.endsWith('__ask_user');
}

@Component({
  selector: 'mc-session-detail',
  imports: [RouterLink, FormsModule],
  templateUrl: './session-detail.component.html',
  styleUrl: './session-detail.component.scss',
})
export class SessionDetailComponent {
  private route = inject(ActivatedRoute);
  private api = inject(ApiService);
  private realtime = inject(RealtimeService);
  private router = inject(Router);

  // Reactive: "Continue here" may navigate /session/A → /session/B, and
  // Angular reuses this component instance across that navigation — a
  // snapshot-read id would silently keep targeting the old session.
  protected readonly id = toSignal(this.route.paramMap.pipe(map((p) => p.get('id') ?? '')), {
    initialValue: this.route.snapshot.paramMap.get('id') ?? '',
  });
  protected readonly session = computed(() =>
    this.realtime.sessions().find((s) => s.id === this.id()),
  );

  protected readonly transcript = signal<TranscriptEvent[]>([]);
  protected readonly rendered = computed(() => {
    const events = this.transcript().slice(-MAX_RENDER);
    const { questionIds, answers } = this.correlateAnswers(events);
    return events
      .map((e) => this.render(e, questionIds, answers))
      .filter((r): r is RenderedEvent => r !== null);
  });

  protected readonly tab = signal<'transcript' | 'changes'>('transcript');
  protected readonly diff = signal<GitOverview | null>(null);
  protected readonly patch = signal<{ path: string; text: string } | null>(null);

  protected readonly instruction = signal('');
  protected readonly answer = signal('');
  /** Labels toggled on in a multi-select question. */
  protected readonly selected = signal<ReadonlySet<string>>(new Set());
  protected readonly busy = signal(false);
  protected readonly notice = signal('');

  private readonly log = viewChild<ElementRef<HTMLElement>>('log');
  /** Identity of the current pending prompt (changes ⇒ reset the selection). */
  private readonly pendingKey = computed(
    () => this.session()?.pending?.requestId ?? this.session()?.pending?.prompt ?? '',
  );

  /** Dedup + ordering state for the transcript (uuid-keyed). WS deltas that
   * race the history fetch are buffered and merged once it lands. */
  private readonly seen = new Set<string>();
  private buffered: TranscriptEvent[] = [];
  private historyLoaded = false;

  protected readonly rel = relativeTime;
  protected readonly label = stateLabel;
  protected readonly surface = surfaceLabel;

  constructor() {
    // (Re)load whenever the routed session changes — including the in-place
    // component reuse of a /session/A → /session/B navigation.
    effect(() => {
      const id = this.id();
      this.transcript.set([]);
      this.diff.set(null);
      this.patch.set(null);
      this.answer.set('');
      this.instruction.set('');
      this.selected.set(new Set());
      this.notice.set('');
      this.seen.clear();
      this.buffered = [];
      this.historyLoaded = false;
      void this.loadHistory(id);
    });

    // Append live deltas for this session (buffer until history has landed).
    effect(() => {
      const delta = this.realtime.transcriptDelta();
      if (!delta || delta.sessionId !== this.id() || !delta.events.length) return;
      if (this.historyLoaded) this.append(delta.events);
      else this.buffered.push(...delta.events);
    });

    // A new pending prompt starts with a clean option selection.
    effect(() => {
      this.pendingKey();
      this.selected.set(new Set());
    });

    // Auto-scroll on new content — but never yank a user who scrolled up.
    effect(() => {
      this.rendered();
      const before = this.log()?.nativeElement;
      const atBottom = !before || before.scrollHeight - before.scrollTop - before.clientHeight < 48;
      queueMicrotask(() => {
        const el = this.log()?.nativeElement;
        if (el && atBottom) el.scrollTop = el.scrollHeight;
      });
    });
  }

  private async loadHistory(id: string): Promise<void> {
    try {
      const history = await this.api.getTranscript(id);
      if (id !== this.id()) return; // navigated away while the fetch ran
      for (const e of history) if (e.uuid) this.seen.add(e.uuid);
      this.transcript.set(history);
      this.historyLoaded = true;
      const buffered = this.buffered;
      this.buffered = [];
      this.append(buffered);
    } catch (e) {
      if (id === this.id()) {
        this.notice.set(`Could not load transcript: ${(e as Error).message}`);
        this.historyLoaded = true; // still accept live deltas
      }
    }
  }

  /** Append transcript events, dropping any uuid we've already rendered. */
  private append(events: TranscriptEvent[]): void {
    const fresh = events.filter((e) => !e.uuid || !this.seen.has(e.uuid));
    for (const e of fresh) if (e.uuid) this.seen.add(e.uuid);
    if (fresh.length) this.transcript.update((prev) => [...prev, ...fresh]);
  }

  async openChanges(): Promise<void> {
    this.tab.set('changes');
    if (!this.diff()) {
      try {
        this.diff.set(await this.api.getDiff(this.id()));
      } catch (e) {
        this.notice.set(`Could not load diff: ${(e as Error).message}`);
      }
    }
  }

  async showPatch(path: string): Promise<void> {
    try {
      const res = await this.api.getFilePatch(this.id(), path);
      this.patch.set({ path, text: res.patch ?? '(no diff)' });
    } catch (e) {
      this.notice.set(`Could not load patch: ${(e as Error).message}`);
    }
  }

  async send(): Promise<void> {
    const text = this.instruction().trim();
    if (!text) return;
    if (await this.guard(() => this.api.sendMessage(this.id(), text), 'Instruction sent.')) {
      this.instruction.set('');
    }
  }

  /** Enter sends; Shift+Enter inserts a newline. */
  onComposerKeydown(event: KeyboardEvent): void {
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      void this.send();
    }
  }

  /** Take over this session (resume in place) so it becomes owned and drivable. */
  async continueSession(): Promise<void> {
    this.busy.set(true);
    this.notice.set('Taking over session…');
    try {
      const id = this.id();
      const res = await this.api.continueSession(id);
      if (res?.id && res.id !== id) {
        this.router.navigate(['/session', res.id]);
      } else {
        this.notice.set('You can now drive this session — type an instruction below.');
      }
    } catch (e) {
      this.notice.set(`Continue failed: ${(e as Error).message}`);
    } finally {
      this.busy.set(false);
    }
  }

  async respond(decision: 'allow' | 'deny'): Promise<void> {
    const req = this.session()?.pending?.requestId;
    await this.guard(
      () => this.api.respondPermission(this.id(), decision, req),
      `Permission ${decision === 'allow' ? 'allowed' : 'denied'}.`,
    );
  }

  /** Single-select: clicking an option sends it as the answer. */
  async chooseOption(labelText: string): Promise<void> {
    await this.sendAnswer(labelText);
  }

  toggleOption(labelText: string): void {
    this.selected.update((prev) => {
      const next = new Set(prev);
      if (next.has(labelText)) next.delete(labelText);
      else next.add(labelText);
      return next;
    });
  }

  /** Multi-select: send every toggled label, in the question's option order. */
  async submitSelected(): Promise<void> {
    const pending = this.session()?.pending;
    const picked = this.selected();
    const ordered = (pending?.options ?? [])
      .map((o) => o.label)
      .filter((l) => picked.has(l));
    if (!ordered.length) return;
    if (await this.sendAnswer(ordered.join(', '))) {
      this.selected.set(new Set());
    }
  }

  async submitAnswer(): Promise<void> {
    const text = this.answer().trim();
    if (!text) return;
    if (await this.sendAnswer(text)) {
      this.answer.set('');
    }
  }

  private async sendAnswer(text: string): Promise<boolean> {
    const req = this.session()?.pending?.requestId;
    return this.guard(() => this.api.answerQuestion(this.id(), text, req), 'Answer sent.');
  }

  async lifecycle(action: 'stop' | 'respawn' | 'rm'): Promise<void> {
    if (action === 'rm' && !confirm('Delete this session and its worktree?')) return;
    await this.guard(() => this.api.lifecycle(this.id(), action), `${action} requested.`);
  }

  /** stop/respawn/rm only apply to app-owned sessions or daemon background jobs;
   * foreign interactive/CLI sessions can't be driven by `claude stop|respawn|rm`. */
  manageable(s: Session): boolean {
    return s.owned || s.kind === 'background';
  }

  /** Highlight the option a recorded answer chose. Exact segment matching
   * avoids "No" lighting up inside "None"; long labels also match as
   * substrings (result texts often wrap the label in prose). */
  answerMatches(answer: string, labelText: string): boolean {
    const needle = labelText.trim().toLowerCase();
    if (!needle) return false;
    const segments = answer.split(/[,\n]/).map((s) => s.trim().toLowerCase());
    if (segments.includes(needle)) return true;
    return needle.length >= 8 && answer.toLowerCase().includes(needle);
  }

  /** Run an action once (re-entrant calls while busy are ignored — key
   * auto-repeat, double-taps). Returns true on success so callers only clear
   * the user's typed input/selection when it actually went through. */
  private async guard(fn: () => Promise<unknown>, ok: string): Promise<boolean> {
    if (this.busy()) return false;
    this.busy.set(true);
    this.notice.set('');
    try {
      await fn();
      this.notice.set(ok);
      return true;
    } catch (e) {
      this.notice.set(`Failed: ${(e as Error).message}`);
      return false;
    } finally {
      this.busy.set(false);
    }
  }

  /** Pre-pass: which tool_use ids are questions, and the answer text their
   * later tool_result carried (so question cards can show what was chosen). */
  private correlateAnswers(events: TranscriptEvent[]): {
    questionIds: Set<string>;
    answers: Map<string, string>;
  } {
    const questionIds = new Set<string>();
    const answers = new Map<string, string>();
    for (const ev of events) {
      const content = ev.message?.content;
      if (!Array.isArray(content)) continue;
      for (const block of content as ContentBlock[]) {
        if (block.type === 'tool_use' && block.id && isQuestionTool(block.name)) {
          questionIds.add(block.id);
        } else if (block.type === 'tool_result' && block.tool_use_id) {
          if (questionIds.has(block.tool_use_id)) {
            answers.set(block.tool_use_id, this.stringify(block.content).trim());
          }
        }
      }
    }
    return { questionIds, answers };
  }

  private render(
    ev: TranscriptEvent,
    questionIds: Set<string>,
    answers: Map<string, string>,
  ): RenderedEvent | null {
    const tools: RenderedTool[] = [];
    const questions: RenderedQuestion[] = [];
    const plans: string[] = [];
    let text = '';
    let error = false;
    const content = ev.message?.content;
    if (typeof content === 'string') {
      text = content;
    } else if (Array.isArray(content)) {
      for (const block of content as ContentBlock[]) {
        if (block.type === 'text' || block.type === 'thinking') {
          text += (block.text ?? '') + '\n';
        } else if (block.type === 'tool_use') {
          const name = block.name ?? 'tool';
          if (isQuestionTool(name)) {
            questions.push(...this.parseQuestions(block, answers));
          } else if (name === 'ExitPlanMode') {
            const plan = (block.input as { plan?: unknown } | undefined)?.plan;
            if (typeof plan === 'string' && plan.trim()) plans.push(renderMarkdown(plan));
            else tools.push({ name, summary: '' });
          } else {
            tools.push({ name, summary: toolSummary(name, block.input) });
          }
        } else if (block.type === 'tool_result') {
          // Question results surface as the answer chip on their card.
          if (block.tool_use_id && questionIds.has(block.tool_use_id)) continue;
          error = error || !!block.is_error;
          text += this.stringify(block.content) + '\n';
        }
      }
    } else if (typeof ev.content === 'string') {
      text = ev.content;
    }

    // Nothing visible (e.g. a user event holding only a question's
    // tool_result, which shows as the answer chip) → no empty bubble.
    if (!text.trim() && !tools.length && !questions.length && !plans.length) return null;

    const role = ev.message?.role ?? ev.type;
    const isAssistant = ev.type === 'assistant';
    const html = isAssistant && text.trim() ? renderMarkdown(text.trim()) : '';
    const base = { text, html, tools, questions, plans, error };
    if (ev.type === 'user') return { who: 'You', cls: 'user', ...base };
    if (isAssistant) return { who: 'Claude', cls: 'assistant', ...base };
    if (ev.type === 'system') return { who: 'System', cls: 'system', ...base };
    return { who: role, cls: 'system', ...base };
  }

  /** Parse AskUserQuestion (`{questions: [...]}`) or ask_user (flat) input. */
  private parseQuestions(
    block: ContentBlock,
    answers: Map<string, string>,
  ): RenderedQuestion[] {
    const input = block.input as
      | { questions?: unknown[]; question?: unknown }
      | undefined;
    const raw: unknown[] = Array.isArray(input?.questions)
      ? input.questions
      : input?.question
        ? [input]
        : [];
    // One tool_result answers the whole block — attach it once (last card)
    // instead of repeating it under every question.
    const answer = block.id ? answers.get(block.id) : undefined;
    const parsed = raw
      .map((q) => this.parseQuestion(q))
      .filter((q): q is RenderedQuestion => q !== null);
    if (parsed.length && answer) parsed[parsed.length - 1].answer = answer;
    return parsed;
  }

  private parseQuestion(q: unknown): RenderedQuestion | null {
    if (typeof q !== 'object' || q === null) return null;
    const rec = q as Record<string, unknown>;
    if (typeof rec['question'] !== 'string') return null;
    const options: QuestionOption[] = Array.isArray(rec['options'])
      ? (rec['options'] as unknown[])
          .map((o): QuestionOption | null => {
            if (typeof o === 'string') return { label: o };
            if (typeof o === 'object' && o !== null) {
              const or = o as Record<string, unknown>;
              if (typeof or['label'] === 'string') {
                return {
                  label: or['label'],
                  description:
                    typeof or['description'] === 'string' ? or['description'] : undefined,
                };
              }
            }
            return null;
          })
          .filter((o): o is QuestionOption => o !== null)
      : [];
    return {
      question: rec['question'],
      header: typeof rec['header'] === 'string' ? rec['header'] : undefined,
      multiSelect: rec['multiSelect'] === true,
      options,
    };
  }

  private stringify(v: unknown): string {
    if (typeof v === 'string') return v;
    if (Array.isArray(v)) {
      return v
        .map((b) =>
          typeof b === 'object' && b && 'text' in b ? String((b as { text: unknown }).text) : '',
        )
        .join('');
    }
    return v == null ? '' : JSON.stringify(v);
  }
}
