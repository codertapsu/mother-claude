import {
  ChangeDetectionStrategy,
  Component,
  Injector,
  afterNextRender,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
} from '@angular/core';

import { StreamBubble } from '../../core/live-stream';
import { TranscriptStore } from '../../core/transcript-store';
import { IconComponent } from '../../shared/icon.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';
import { StreamBubbleComponent } from './stream-bubble.component';
import { TranscriptRowComponent } from './transcript-row.component';
import { Row, buildRows, sameRow } from './transcript-rows';

/** Within this many px of the bottom counts as "at the bottom". */
const STICK_PX = 80;

/**
 * The chat: the transcript's rows, the owned live bubble, "Load earlier" at
 * the top and an "N new ↓" pill.
 *
 * Scrolling: it sticks to the bottom only while the reader is within 80 px of
 * it (new rows and stream deltas then keep it pinned); scrolled up, new rows
 * only bump the pill. "Load earlier" keeps the reader's place by anchoring on
 * the first visible row. The scroll container is the page's (passed in), so
 * the chip row and tabs scroll away with the conversation on phones.
 */
@Component({
  selector: 'mc-transcript',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, StreamBubbleComponent, TimeAgoComponent, TranscriptRowComponent],
  template: `
    @if (store(); as t) {
      @if (t.hasEarlier()) {
        <button
          class="btn small ghost earlier"
          type="button"
          [disabled]="t.loadingEarlier()"
          (click)="loadEarlier()"
        >
          {{ t.loadingEarlier() ? 'Loading…' : 'Load earlier' }}
        </button>
      }
      @if (t.loading()) {
        <div class="skeleton" aria-label="Loading the conversation">
          <span class="r"></span><span></span><span class="s"></span><span></span>
        </div>
      } @else if (!rows().length && !bubbles().length) {
        @if (startingSince()) {
          <div class="card starting" role="status">
            <span class="spin" aria-hidden="true"></span>
            <div>
              <strong>Starting Claude…</strong>
              <span class="muted small">
                <mc-time [ms]="startingSince()" mode="elapsed" />
              </span>
            </div>
            @if (canCancel()) {
              <button class="btn small" type="button" (click)="cancelStart.emit()">Cancel</button>
            }
          </div>
        } @else {
          <p class="empty muted">No messages yet.</p>
        }
      }
      <div class="rows">
        @for (r of rows(); track r.id) {
          <mc-transcript-row
            [row]="r"
            [sessionId]="sessionId()"
            [cwd]="cwd()"
            [relayVia]="relayVia()"
            [agentLinks]="agentLinks()"
            [running]="running()"
          />
        }
        @for (b of bubbles(); track b.messageId) {
          <mc-stream-bubble [bubble]="b" />
        }
      </div>
      @if (t.error()) {
        <p class="error-text small" role="alert">
          {{ t.error() }}
          <button class="btn ghost small" type="button" (click)="t.refresh()">Retry</button>
        </p>
      }
    }
    @if (newCount()) {
      <button class="newpill" type="button" (click)="jump()">
        {{ newCount() }} new <mc-icon name="arrowDown" [size]="14" />
      </button>
    }
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
        min-width: 0;
        padding: 0.5rem 0 0.75rem;
      }
      .rows {
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
        min-width: 0;
      }
      .earlier {
        align-self: center;
      }
      .empty {
        text-align: center;
        margin: 2rem 0;
      }
      .starting {
        display: flex;
        align-items: center;
        gap: 0.75rem;
      }
      .starting div {
        flex: 1;
        display: flex;
        flex-direction: column;
      }
      .skeleton {
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
      }
      .skeleton span {
        display: block;
        height: 0.9rem;
        border-radius: 0.3rem;
        background: var(--panel-3);
        animation: mc-shimmer 1.2s ease-in-out infinite alternate;
      }
      .skeleton .r {
        width: 55%;
        height: 2.2rem;
        margin-left: auto;
        border-radius: 0.8rem;
      }
      .skeleton .s {
        width: 70%;
      }
      .newpill {
        position: sticky;
        bottom: 0.5rem;
        align-self: center;
        z-index: 2;
        display: inline-flex;
        align-items: center;
        gap: 0.3rem;
        min-height: 36px;
        padding: 0.3rem 0.85rem;
        border-radius: 1.2rem;
        border: 1px solid var(--accent);
        background: var(--panel);
        color: var(--accent-text);
        font: inherit;
        font-size: 0.8rem;
        font-weight: 600;
        box-shadow: var(--shadow);
        cursor: pointer;
      }
      .spin {
        width: 1.2rem;
        height: 1.2rem;
        border-radius: 50%;
        border: 2px solid var(--accent-text);
        border-right-color: transparent;
        animation: mc-spin 0.8s linear infinite;
      }
    `,
  ],
})
export class TranscriptComponent {
  private injector = inject(Injector);

  /** The page's scroll container. */
  readonly scroller = input.required<HTMLElement>();
  readonly store = input<TranscriptStore | null>(null);
  /** Owned live bubbles (main conversation only). */
  readonly bubbles = input<readonly StreamBubble[]>([]);
  readonly sessionId = input('');
  readonly cwd = input('');
  /** "via hook" / "via tmux" label for relayed messages. */
  readonly relayVia = input('');
  /** toolUseId → agent id, to link Agent calls to their transcripts. */
  readonly agentLinks = input<ReadonlyMap<string, string>>(new Map());
  /** Busy: calls without a result spin. */
  readonly running = input(false);
  /** A freshly spawned session (epoch ms): show "Starting Claude…". */
  readonly startingSince = input<number | undefined>(undefined);
  readonly canCancel = input(false);
  readonly cancelStart = output<void>();

  private cache = new Map<string, Row>();
  protected readonly rows = computed<Row[]>(() => {
    const fresh = buildRows(this.store()?.events() ?? []);
    const next = new Map<string, Row>();
    const out = fresh.map((r) => {
      const old = this.cache.get(r.id);
      const keep = old && sameRow(old, r) ? old : r;
      next.set(r.id, keep);
      return keep;
    });
    this.cache = next;
    return out;
  });
  protected readonly newCount = signal(0);

  private atBottom = true;
  private firstLoad = true;
  private prepending = false;
  private lastCount = 0;

  constructor() {
    // Track whether the reader is at the bottom; follow size changes (the
    // composer growing, a pending card appearing) while they are.
    effect((onCleanup) => {
      const el = this.scroller();
      const onScroll = () => {
        this.atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < STICK_PX;
        if (this.atBottom && this.newCount()) this.newCount.set(0);
      };
      el.addEventListener('scroll', onScroll, { passive: true });
      let ro: ResizeObserver | null = null;
      if ('ResizeObserver' in window) {
        ro = new ResizeObserver(() => {
          if (this.atBottom && !this.prepending) this.scrollToBottom();
        });
        ro.observe(el);
      }
      onCleanup(() => {
        el.removeEventListener('scroll', onScroll);
        ro?.disconnect();
      });
    });

    // A different session: start at the bottom again.
    effect(() => {
      this.store();
      untracked(() => {
        this.firstLoad = true;
        this.lastCount = 0;
        this.cache = new Map();
        this.newCount.set(0);
      });
    });

    // New rows or stream deltas: stay pinned, or count what's new.
    effect(() => {
      const count = this.rows().length;
      this.bubbles();
      const loading = this.store()?.loading() ?? true;
      untracked(() => {
        if (loading) return;
        const added = Math.max(0, count - this.lastCount);
        this.lastCount = count;
        if (this.prepending) return;
        if (this.firstLoad || this.atBottom) {
          this.firstLoad = false;
          afterNextRender(() => this.scrollToBottom(), { injector: this.injector });
        } else if (added) {
          this.newCount.update((n) => n + added);
        }
      });
    });
  }

  protected jump(): void {
    this.newCount.set(0);
    this.scrollToBottom(true);
  }

  protected async loadEarlier(): Promise<void> {
    const store = this.store();
    if (!store) return;
    const el = this.scroller();
    const anchor = this.firstVisibleRow(el);
    this.prepending = true;
    try {
      await store.loadEarlier();
    } finally {
      afterNextRender(
        () => {
          if (anchor) {
            const node = el.querySelector<HTMLElement>(`[data-row="${CSS.escape(anchor.id)}"]`);
            if (node) {
              const top = node.getBoundingClientRect().top - el.getBoundingClientRect().top;
              el.scrollTop += top - anchor.top;
            }
          }
          this.prepending = false;
        },
        { injector: this.injector },
      );
    }
  }

  private scrollToBottom(smooth = false): void {
    const el = this.scroller();
    const reduce =
      typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
    el.scrollTo({ top: el.scrollHeight, behavior: smooth && !reduce ? 'smooth' : 'auto' });
    this.atBottom = true;
  }

  private firstVisibleRow(el: HTMLElement): { id: string; top: number } | null {
    const box = el.getBoundingClientRect();
    for (const node of Array.from(el.querySelectorAll<HTMLElement>('[data-row]'))) {
      const r = node.getBoundingClientRect();
      if (r.bottom > box.top + 1) {
        return { id: node.dataset['row'] ?? '', top: r.top - box.top };
      }
    }
    return null;
  }
}
