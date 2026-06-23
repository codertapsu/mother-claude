import { Component, ElementRef, computed, effect, inject, signal, viewChild } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { ActivatedRoute, RouterLink } from '@angular/router';

import { ApiService } from '../../core/api.service';
import { RealtimeService } from '../../core/realtime.service';
import { ContentBlock, GitOverview, TranscriptEvent } from '../../core/models';
import { relativeTime, stateLabel, surfaceLabel } from '../../shared/util';

interface RenderedEvent {
  who: string;
  cls: string;
  text: string;
  tools: string[];
  error: boolean;
}

const MAX_RENDER = 800;

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

  protected readonly id = this.route.snapshot.paramMap.get('id') ?? '';
  protected readonly session = computed(() =>
    this.realtime.sessions().find((s) => s.id === this.id),
  );

  protected readonly transcript = signal<TranscriptEvent[]>([]);
  protected readonly rendered = computed(() =>
    this.transcript()
      .slice(-MAX_RENDER)
      .map((e) => this.render(e))
      .filter((r) => r !== null),
  );

  protected readonly tab = signal<'transcript' | 'changes'>('transcript');
  protected readonly diff = signal<GitOverview | null>(null);
  protected readonly patch = signal<{ path: string; text: string } | null>(null);

  protected readonly instruction = signal('');
  protected readonly answer = signal('');
  protected readonly busy = signal(false);
  protected readonly notice = signal('');

  private readonly log = viewChild<ElementRef<HTMLElement>>('log');

  protected readonly rel = relativeTime;
  protected readonly label = stateLabel;
  protected readonly surface = surfaceLabel;

  constructor() {
    void this.loadHistory();

    // Append live deltas for this session.
    effect(() => {
      const delta = this.realtime.transcriptDelta();
      if (delta && delta.sessionId === this.id && delta.events.length) {
        this.transcript.update((prev) => [...prev, ...delta.events]);
      }
    });

    // Auto-scroll the transcript to the bottom on new content.
    effect(() => {
      this.rendered();
      queueMicrotask(() => {
        const el = this.log()?.nativeElement;
        if (el) el.scrollTop = el.scrollHeight;
      });
    });
  }

  private async loadHistory(): Promise<void> {
    try {
      this.transcript.set(await this.api.getTranscript(this.id));
    } catch (e) {
      this.notice.set(`Could not load transcript: ${(e as Error).message}`);
    }
  }

  async openChanges(): Promise<void> {
    this.tab.set('changes');
    if (!this.diff()) {
      try {
        this.diff.set(await this.api.getDiff(this.id));
      } catch (e) {
        this.notice.set(`Could not load diff: ${(e as Error).message}`);
      }
    }
  }

  async showPatch(path: string): Promise<void> {
    try {
      const res = await this.api.getFilePatch(this.id, path);
      this.patch.set({ path, text: res.patch ?? '(no diff)' });
    } catch (e) {
      this.notice.set(`Could not load patch: ${(e as Error).message}`);
    }
  }

  async send(): Promise<void> {
    const text = this.instruction().trim();
    if (!text) return;
    await this.guard(() => this.api.sendMessage(this.id, text), 'Instruction sent.');
    this.instruction.set('');
  }

  async respond(decision: 'allow' | 'deny'): Promise<void> {
    const req = this.session()?.pending?.requestId;
    await this.guard(
      () => this.api.respondPermission(this.id, decision, req),
      `Permission ${decision}d.`,
    );
  }

  async submitAnswer(): Promise<void> {
    const text = this.answer().trim();
    if (!text) return;
    const req = this.session()?.pending?.requestId;
    await this.guard(() => this.api.answerQuestion(this.id, text, req), 'Answer sent.');
    this.answer.set('');
  }

  async lifecycle(action: 'stop' | 'respawn' | 'rm'): Promise<void> {
    if (action === 'rm' && !confirm('Delete this session and its worktree?')) return;
    await this.guard(() => this.api.lifecycle(this.id, action), `${action} requested.`);
  }

  private async guard(fn: () => Promise<unknown>, ok: string): Promise<void> {
    this.busy.set(true);
    this.notice.set('');
    try {
      await fn();
      this.notice.set(ok);
    } catch (e) {
      this.notice.set(`Failed: ${(e as Error).message}`);
    } finally {
      this.busy.set(false);
    }
  }

  private render(ev: TranscriptEvent): RenderedEvent | null {
    const tools: string[] = [];
    let text = '';
    let error = false;
    const content = ev.message?.content;
    if (typeof content === 'string') {
      text = content;
    } else if (Array.isArray(content)) {
      for (const block of content as ContentBlock[]) {
        if (block.type === 'text' || block.type === 'thinking') text += (block.text ?? '') + '\n';
        else if (block.type === 'tool_use') tools.push(block.name ?? 'tool');
        else if (block.type === 'tool_result') {
          error = error || !!block.is_error;
          text += this.stringify(block.content) + '\n';
        }
      }
    } else if (typeof ev.content === 'string') {
      text = ev.content;
    }

    const role = ev.message?.role ?? ev.type;
    if (ev.type === 'user') return { who: 'You', cls: 'user', text, tools, error };
    if (ev.type === 'assistant') return { who: 'Claude', cls: 'assistant', text, tools, error };
    if (ev.type === 'system') return { who: 'System', cls: 'system', text, tools, error };
    if (!text && !tools.length) return null;
    return { who: role, cls: 'system', text, tools, error };
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
