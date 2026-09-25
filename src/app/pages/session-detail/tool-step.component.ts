import { ChangeDetectionStrategy, Component, computed, input, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { hunksToRows } from '../../shared/diff';
import { DiffViewComponent } from '../../shared/diff-view.component';
import { IconComponent } from '../../shared/icon.component';
import { ToolPreviewComponent } from '../../shared/tool-preview.component';
import { toolIcon } from './tool-icons';
import { ToolStep } from './transcript-rows';

/**
 * One tool call in the transcript: icon, name and summary with its state
 * (running, done, failed). Expanding shows what it was asked to do (the
 * input preview), its output and, for edits, the patch. An Agent call links
 * to that agent's own transcript.
 */
@Component({
  selector: 'mc-tool-step',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, DiffViewComponent, IconComponent, ToolPreviewComponent],
  template: `
    @let t = step().tool.tool;
    @let res = step().result?.result;
    <button
      type="button"
      class="head"
      [attr.aria-expanded]="open()"
      [attr.data-state]="state()"
      (click)="open.set(!open())"
    >
      <mc-icon class="ticon" [name]="icon()" [size]="14" />
      <span class="tname">{{ t?.name || 'Tool' }}</span>
      <span class="tsum">{{ t?.summary }}</span>
      <span class="state" [attr.aria-label]="stateLabel()">
        @switch (state()) {
          @case ('running') {
            <span class="spin" aria-hidden="true"></span>
          }
          @case ('error') {
            <mc-icon name="xCircle" [size]="14" />
          }
          @case ('done') {
            <mc-icon name="check" [size]="14" />
          }
        }
      </span>
      <mc-icon class="chev" [name]="open() ? 'chevronDown' : 'chevronRight'" [size]="14" />
    </button>
    @if (open()) {
      <div class="more">
        @if (t?.input !== undefined && t?.input !== null) {
          <mc-tool-preview [name]="t?.name || ''" [input]="t?.input" [cwd]="cwd()" />
        }
        @if (res?.patch; as p) {
          <p class="plabel mono">{{ p.path }}</p>
          <mc-diff-view [rows]="patchRows()" />
        }
        @if (res?.text) {
          <pre class="out" [class.err]="res?.isError">{{ res?.text }}</pre>
          @if (res?.truncated) {
            <p class="muted small">Output shortened.</p>
          }
        } @else if (!step().result) {
          <p class="muted small">{{ running() ? 'Still running…' : 'No result recorded.' }}</p>
        }
        @if (agentId(); as a) {
          <a class="btn small" [routerLink]="['/session', sessionId(), 'agent', a]">
            <mc-icon name="agents" [size]="14" /> Open this agent's transcript
          </a>
        }
      </div>
    }
  `,
  styles: [
    `
      :host {
        display: block;
        min-width: 0;
      }
      .head {
        appearance: none;
        width: 100%;
        display: flex;
        align-items: center;
        gap: 0.4rem;
        min-height: 32px;
        padding: 0.15rem 0.3rem;
        border: 0;
        border-radius: var(--radius-sm);
        background: none;
        color: var(--muted);
        font: inherit;
        font-size: 0.8rem;
        text-align: left;
        cursor: pointer;
      }
      .head:hover {
        background: var(--panel-2);
      }
      .ticon {
        color: var(--faint);
      }
      .tname {
        font-weight: 600;
        color: var(--text);
        white-space: nowrap;
      }
      .tsum {
        flex: 1;
        min-width: 0;
        font-family: var(--mono);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .state {
        display: inline-flex;
      }
      [data-state='done'] .state {
        color: var(--green);
      }
      [data-state='error'] .state,
      [data-state='error'] .tname {
        color: var(--red);
      }
      [data-state='running'] .state {
        color: var(--accent-text);
      }
      .chev {
        color: var(--faint);
      }
      .more {
        display: flex;
        flex-direction: column;
        gap: 0.45rem;
        padding: 0.35rem 0 0.5rem 1.6rem;
      }
      .out {
        margin: 0;
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
      .out.err {
        border-color: var(--red-border);
      }
      .plabel {
        margin: 0;
        color: var(--muted);
      }
      .spin {
        width: 0.8rem;
        height: 0.8rem;
        border-radius: 50%;
        border: 2px solid currentColor;
        border-right-color: transparent;
        animation: mc-spin 0.7s linear infinite;
      }
    `,
  ],
})
export class ToolStepComponent {
  readonly step = input.required<ToolStep>();
  readonly sessionId = input('');
  readonly cwd = input('');
  readonly running = input(false);
  readonly agentId = input<string | undefined>(undefined);
  /** Start expanded. */
  readonly initiallyOpen = input(false);

  protected readonly open = signal(false);
  protected readonly icon = computed(() => toolIcon(this.step().tool.tool?.name));
  protected readonly state = computed<'running' | 'error' | 'done' | 'waiting'>(() => {
    const r = this.step().result;
    if (r) return r.result?.isError ? 'error' : 'done';
    return this.running() ? 'running' : 'waiting';
  });
  protected readonly stateLabel = computed(() => {
    switch (this.state()) {
      case 'running':
        return 'running';
      case 'error':
        return 'failed';
      case 'done':
        return 'done';
      default:
        return 'no result';
    }
  });
  protected readonly patchRows = computed(() =>
    hunksToRows(this.step().result?.result?.patch?.hunks),
  );

  constructor() {
    queueMicrotask(() => {
      if (this.initiallyOpen()) this.open.set(true);
    });
  }
}
