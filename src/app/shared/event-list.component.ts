import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { UiEvent } from '../core/models';
import { IconComponent, IconName } from './icon.component';
import { renderMarkdown } from './markdown';
import { MediaImageComponent } from './media-image.component';

interface Row {
  ev: UiEvent;
  html: string;
}

const SYSTEM_ICONS: Partial<Record<UiEvent['kind'], IconName>> = {
  notice: 'info',
  compact: 'refresh',
  error: 'alert',
  task: 'activity',
  relayed: 'phone',
};

/**
 * A plain, readable rendering of normalised transcript events (§4.4): prompts,
 * markdown answers, collapsible thinking, tool rows with expandable results,
 * images, todos, questions, plans and system notices. Only the newest `max`
 * events render.
 */
@Component({
  selector: 'mc-event-list',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, MediaImageComponent],
  template: `
    @for (r of rows(); track r.ev.id) {
      @let e = r.ev;
      @switch (e.kind) {
        @case ('prompt') {
          <div class="ev prompt">
            <span class="who">You</span>
            <div class="bubble">{{ e.text }}</div>
          </div>
        }
        @case ('text') {
          <div class="ev text md" [innerHTML]="r.html"></div>
        }
        @case ('thinking') {
          <details class="ev thinking">
            <summary>Thinking</summary>
            <div class="md" [innerHTML]="r.html"></div>
          </details>
        }
        @case ('tool') {
          <div class="ev tool">
            <mc-icon name="terminal" [size]="14" />
            <span class="tname">{{ e.tool?.name }}</span>
            <span class="tsum">{{ e.tool?.summary }}</span>
          </div>
        }
        @case ('result') {
          @if (e.result; as res) {
            @if (res.text || res.patch) {
              <details class="ev result" [class.err]="res.isError">
                <summary>
                  {{ res.isError ? 'Error' : 'Result' }}
                  @if (res.patch; as p) {
                    · {{ p.path }}
                  }
                  @if (res.truncated) {
                    <span class="muted"> (truncated)</span>
                  }
                </summary>
                @if (res.text) {
                  <pre>{{ res.text }}</pre>
                }
              </details>
            }
          }
        }
        @case ('image') {
          @if (e.image?.ref; as ref) {
            <div class="ev image"><mc-media-image [ref]="ref" /></div>
          }
        }
        @case ('todos') {
          <div class="ev todos">
            @for (t of e.todos ?? []; track $index) {
              <div class="todo" [attr.data-status]="t.status">
                <mc-icon
                  [name]="
                    t.status === 'completed'
                      ? 'check'
                      : t.status === 'in_progress'
                        ? 'activity'
                        : 'clock'
                  "
                  [size]="14"
                />
                <span>{{
                  t.status === 'in_progress' && t.activeForm ? t.activeForm : t.content
                }}</span>
              </div>
            }
          </div>
        }
        @case ('question') {
          <div class="ev question">
            @for (q of e.question?.questions ?? []; track $index) {
              <p class="q"><mc-icon name="question" [size]="14" /> {{ q.question }}</p>
              @if (e.question?.answers?.[q.question]; as a) {
                <p class="a">↳ {{ a }}</p>
              }
            }
          </div>
        }
        @case ('plan') {
          <div class="ev plan">
            <p class="ptitle"><mc-icon name="map" [size]="14" /> Plan</p>
            <div class="md" [innerHTML]="r.html"></div>
          </div>
        }
        @default {
          <div class="ev system" [attr.data-kind]="e.kind">
            <mc-icon [name]="icon(e)" [size]="14" />
            <span>{{ e.text || e.task?.summary || e.kind }}</span>
          </div>
        }
      }
    } @empty {
      <p class="muted">Nothing here yet.</p>
    }
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.55rem;
        min-width: 0;
      }
      .ev {
        min-width: 0;
      }
      .prompt {
        align-self: flex-end;
        max-width: 88%;
        display: flex;
        flex-direction: column;
        align-items: flex-end;
        gap: 0.15rem;
      }
      .who {
        font-size: 0.7rem;
        color: var(--faint);
      }
      .bubble {
        background: var(--accent-soft);
        border: 1px solid var(--accent);
        border-radius: 0.9rem 0.9rem 0.2rem 0.9rem;
        padding: 0.5rem 0.75rem;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
      }
      .thinking summary,
      .result summary {
        cursor: pointer;
        font-size: 0.8rem;
        color: var(--muted);
      }
      .thinking .md {
        color: var(--muted);
        font-size: 0.85rem;
        margin-top: 0.3rem;
      }
      .tool {
        display: flex;
        align-items: center;
        gap: 0.4rem;
        font-size: 0.8rem;
        color: var(--muted);
        min-width: 0;
      }
      .tname {
        font-weight: 600;
        color: var(--text);
      }
      .tsum {
        font-family: var(--mono);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        min-width: 0;
      }
      .result pre {
        margin: 0.3rem 0 0;
        padding: 0.5rem 0.6rem;
        background: var(--code-bg);
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        font-size: 0.75rem;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        max-height: 20rem;
        overflow: auto;
      }
      .result.err summary {
        color: var(--red);
      }
      .todos {
        background: var(--panel-2);
        border-radius: var(--radius-sm);
        padding: 0.5rem 0.65rem;
      }
      .todo {
        display: flex;
        gap: 0.4rem;
        align-items: flex-start;
        font-size: 0.85rem;
        padding: 0.1rem 0;
      }
      .todo mc-icon {
        margin-top: 0.15rem;
        color: var(--faint);
      }
      .todo[data-status='completed'] {
        color: var(--muted);
        text-decoration: line-through;
      }
      .todo[data-status='completed'] mc-icon {
        color: var(--green);
      }
      .todo[data-status='in_progress'] mc-icon {
        color: var(--amber);
      }
      .question .q,
      .ptitle {
        display: flex;
        gap: 0.35rem;
        align-items: center;
        margin: 0;
        font-weight: 500;
      }
      .question .a {
        margin: 0.2rem 0 0.4rem 1.2rem;
        color: var(--accent-text);
      }
      .plan {
        border: 1px solid var(--accent);
        border-radius: var(--radius-sm);
        padding: 0.6rem 0.75rem;
      }
      .system {
        display: flex;
        gap: 0.4rem;
        align-items: center;
        font-size: 0.8rem;
        color: var(--muted);
      }
      .system[data-kind='error'] {
        color: var(--red);
      }
      .system[data-kind='relayed'] {
        color: var(--accent-text);
      }
    `,
  ],
})
export class EventListComponent {
  readonly events = input.required<UiEvent[]>();
  /** Render only the newest N events. */
  readonly max = input(150);

  protected readonly rows = computed<Row[]>(() => {
    const list = this.events();
    const start = Math.max(0, list.length - this.max());
    return list.slice(start).map((ev) => ({
      ev,
      html:
        ev.kind === 'text' || ev.kind === 'thinking'
          ? renderMarkdown(ev.text ?? '')
          : ev.kind === 'plan'
            ? renderMarkdown(ev.plan?.text ?? ev.text ?? '')
            : '',
    }));
  });

  protected icon(e: UiEvent): IconName {
    return SYSTEM_ICONS[e.kind] ?? 'info';
  }
}
