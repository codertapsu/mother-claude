import { NgZone, Signal, signal } from '@angular/core';
import { Subscription } from 'rxjs';

import { StreamFrame, StreamKind, UiEvent } from './models';
import type { RealtimeService } from './realtime.service';

export interface StreamBlock {
  index: number;
  kind: StreamKind;
  text: string;
  tool?: { id?: string; name?: string; summary?: string };
}

/** One assistant message being written right now (owned sessions only). */
export interface StreamBubble {
  messageId: string;
  blocks: StreamBlock[];
  /** Last `status` frame, e.g. "requesting" or "thinking". */
  status?: string;
  updatedAt: number;
  done: boolean;
}

/** Status values that mean the message (or turn) is over. */
const DONE_STATUSES = new Set([
  'done',
  'end',
  'ended',
  'stop',
  'stopped',
  'message_stop',
  'result',
  'idle',
  'turn_end',
  'interrupted',
]);

/** Bubbles that saw no delta for this long disappear on their own. */
const STALE_MS = 45_000;

/**
 * Owned sessions stream token-by-token (`stream` frames keyed by the
 * assistant `messageId` and content-block `index`). This assembles those
 * deltas into live bubbles, flushing at most once per animation frame so a
 * fast stream can't re-render the page per token. When the flushed
 * transcript record arrives (a UiEvent with the same `messageId`) its bubble
 * is dropped, so the text never shows twice.
 */
export class LiveStream {
  private readonly bubblesS = signal<StreamBubble[]>([]);
  readonly bubbles: Signal<StreamBubble[]> = this.bubblesS.asReadonly();

  private readonly pending = new Map<string, StreamBubble>();
  private frame = 0;
  private readonly sub: Subscription;
  private sweep?: ReturnType<typeof setInterval>;

  constructor(
    realtime: RealtimeService,
    private readonly sessionId: string,
    private readonly zone: NgZone,
  ) {
    this.sub = realtime.stream$.subscribe((f) => {
      if (f.session === sessionId) this.apply(f);
    });
    this.zone.runOutsideAngular(() => {
      this.sweep = setInterval(() => {
        const now = Date.now();
        if (this.bubblesS().some((b) => now - b.updatedAt > STALE_MS)) {
          this.zone.run(() =>
            this.bubblesS.update((list) => list.filter((b) => now - b.updatedAt <= STALE_MS)),
          );
        }
      }, 10_000);
    });
  }

  /** New transcript events arrived: drop the bubbles they replace. */
  settle(events: readonly UiEvent[]): void {
    const list = this.bubblesS();
    if (!list.length) return;
    const ids = new Set(events.map((e) => e.messageId).filter((x): x is string => !!x));
    const anyAssistantText = events.some(
      (e) => e.role === 'assistant' && (e.kind === 'text' || e.kind === 'tool'),
    );
    const keep = list.filter(
      (b) => !ids.has(b.messageId) && !(anyAssistantText && b.done && !ids.size),
    );
    if (keep.length !== list.length) this.bubblesS.set(keep);
    for (const id of ids) this.pending.delete(id);
  }

  clear(): void {
    this.pending.clear();
    this.bubblesS.set([]);
  }

  destroy(): void {
    this.sub.unsubscribe();
    clearInterval(this.sweep);
    if (this.frame) cancelAnimationFrame(this.frame);
  }

  private apply(f: StreamFrame): void {
    const key = f.messageId || 'current';
    const current =
      this.pending.get(key) ??
      structuredCloneBubble(this.bubblesS().find((b) => b.messageId === key)) ??
      ({ messageId: key, blocks: [], updatedAt: Date.now(), done: false } as StreamBubble);
    current.updatedAt = Date.now();
    if (f.kind === 'status') {
      current.status = f.status;
      if (f.status && DONE_STATUSES.has(f.status)) current.done = true;
    } else {
      const index = typeof f.index === 'number' ? f.index : current.blocks.length ? -1 : 0;
      let block =
        index >= 0
          ? current.blocks.find((b) => b.index === index)
          : current.blocks[current.blocks.length - 1];
      if (!block || block.kind !== f.kind) {
        block = { index: index >= 0 ? index : current.blocks.length, kind: f.kind, text: '' };
        current.blocks.push(block);
        current.blocks.sort((a, b) => a.index - b.index);
      }
      if (f.delta) block.text += f.delta;
      if (f.tool) {
        block.tool = { id: f.tool.id, name: f.tool.name, summary: f.tool.summary };
      }
    }
    this.pending.set(key, current);
    this.scheduleFlush();
  }

  private scheduleFlush(): void {
    if (this.frame) return;
    this.zone.runOutsideAngular(() => {
      this.frame = requestAnimationFrame(() => {
        this.frame = 0;
        const updates = [...this.pending.values()];
        this.pending.clear();
        this.zone.run(() =>
          this.bubblesS.update((list) => {
            const next = [...list];
            for (const u of updates) {
              const i = next.findIndex((b) => b.messageId === u.messageId);
              const copy = { ...u, blocks: u.blocks.map((b) => ({ ...b })) };
              if (i >= 0) next[i] = copy;
              else next.push(copy);
            }
            return next;
          }),
        );
      });
    });
  }
}

function structuredCloneBubble(b: StreamBubble | undefined): StreamBubble | undefined {
  return b ? { ...b, blocks: b.blocks.map((x) => ({ ...x })) } : undefined;
}
