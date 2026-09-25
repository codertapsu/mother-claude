import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { StreamBubble } from '../../core/live-stream';
import { IconComponent } from '../../shared/icon.component';
import { renderMarkdown } from '../../shared/markdown';
import { toolIcon } from './tool-icons';

/**
 * An owned session's message while it is being written: thinking (folded to
 * its last line), text rendered as markdown as it streams, and the tool it
 * is starting. `aria-busy` until the message is complete; the flushed
 * transcript record then replaces it.
 */
@Component({
  selector: 'mc-stream-bubble',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent],
  host: { '[attr.aria-busy]': '!bubble().done', 'aria-label': 'Claude is writing' },
  template: `
    @for (b of blocks(); track b.index) {
      @switch (b.kind) {
        @case ('thinking') {
          <p class="thinking">
            <mc-icon name="sparkle" [size]="13" />
            <span>{{ b.tail }}</span>
          </p>
        }
        @case ('tool') {
          <p class="tool">
            <mc-icon [name]="icon(b.tool?.name)" [size]="14" />
            <strong>{{ b.tool?.name || 'Tool' }}</strong>
            <span class="tsum">{{ b.tool?.summary || b.text }}</span>
            <span class="spin" aria-hidden="true"></span>
          </p>
        }
        @default {
          <div class="md text" [innerHTML]="b.html"></div>
        }
      }
    } @empty {
      <p class="thinking">
        <mc-icon name="sparkle" [size]="13" /> <span>{{ statusText() }}</span>
      </p>
    }
    @if (!bubble().done) {
      <span class="caret" aria-hidden="true"></span>
    }
  `,
  styles: [
    `
      :host {
        display: block;
        min-width: 0;
      }
      p {
        margin: 0;
      }
      .thinking {
        display: flex;
        align-items: center;
        gap: 0.35rem;
        font-size: 0.8rem;
        color: var(--muted);
        font-style: italic;
        overflow: hidden;
        white-space: nowrap;
      }
      .thinking span {
        overflow: hidden;
        text-overflow: ellipsis;
      }
      .tool {
        display: flex;
        align-items: center;
        gap: 0.4rem;
        font-size: 0.8rem;
        color: var(--muted);
        min-width: 0;
      }
      .tool strong {
        color: var(--text);
      }
      .tsum {
        flex: 1;
        min-width: 0;
        font-family: var(--mono);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .text {
        overflow-wrap: anywhere;
      }
      .caret {
        display: inline-block;
        width: 0.5rem;
        height: 1rem;
        vertical-align: text-bottom;
        background: var(--accent-text);
        animation: mc-blink 1s steps(2, start) infinite;
      }
      .spin {
        width: 0.8rem;
        height: 0.8rem;
        border-radius: 50%;
        border: 2px solid var(--accent-text);
        border-right-color: transparent;
        animation: mc-spin 0.7s linear infinite;
      }
      @keyframes mc-blink {
        to {
          visibility: hidden;
        }
      }
    `,
  ],
})
export class StreamBubbleComponent {
  readonly bubble = input.required<StreamBubble>();

  protected readonly blocks = computed(() =>
    this.bubble().blocks.map((b) => ({
      ...b,
      html: b.kind === 'text' ? renderMarkdown(b.text) : '',
      tail: b.kind === 'thinking' ? lastLine(b.text) || 'Thinking…' : '',
    })),
  );
  protected readonly statusText = computed(() => {
    const s = this.bubble().status;
    return s ? `${s[0].toUpperCase()}${s.slice(1)}…` : 'Thinking…';
  });

  protected icon(name: string | undefined) {
    return toolIcon(name);
  }
}

function lastLine(text: string): string {
  const lines = text.trim().split('\n');
  return lines[lines.length - 1]?.trim() ?? '';
}
