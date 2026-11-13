import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { TPipe } from '../../core/i18n/t.pipe';
import { IconComponent } from '../../shared/icon.component';
import { NODE_H, NODE_W } from './template-graph';

/** What one node of the template canvas shows. */
export interface NodeView {
  /** The role's id, or `MERGE_NODE`. */
  key: string;
  kind: 'role' | 'merge';
  x: number;
  y: number;
  title: string;
  lead: boolean;
  /** "sonnet · High", in words. */
  model: string;
  mode: string;
  modeHint: string;
  dangerous: boolean;
  worktree: boolean;
  readOnly: boolean;
  /** The computer it runs on: short, for its chip ('' for this one), and
   * in words. */
  computer: string;
  where: string;
  /** The merge node: the branch, the gate, review on, who merges. */
  branch: string;
  gate: string;
  review: boolean;
  merger: string;
  errors: string[];
  warnings: string[];
  selected: boolean;
  /** While a handoff is drawn: this node takes it, is under the pointer,
   * or can't take it. */
  link: '' | 'ok' | 'over' | 'no';
  dragging: boolean;
  /** It has the dot a handoff is drawn from. */
  handle: boolean;
  /** Its entrance waits this long (ms): the first ones come in one by one. */
  delay: number;
  label: string;
}

/**
 * One node of the template canvas (`TemplateCanvasComponent` places it and
 * handles its pointer and keys): a role — title, id, lead, model and effort,
 * permission mode, worktree, read-only and computer, its problems — or the
 * merge node.
 */
@Component({
  selector: 'mc-template-node',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, TPipe],
  host: {
    class: 'node',
    role: 'button',
    tabindex: '0',
    '[attr.data-node]': 'v().key',
    '[attr.aria-pressed]': 'v().selected',
    '[attr.aria-label]': 'v().label',
    '[class.selected]': 'v().selected',
    '[class.dragging]': 'v().dragging',
    '[class.link-ok]': "v().link === 'ok'",
    '[class.link-over]': "v().link === 'over'",
    '[class.link-no]': "v().link === 'no'",
    '[style.transform]': 'place()',
    '[style.width.px]': 'width',
    '[style.height.px]': 'height',
  },
  template: `
    <div
      class="card"
      [class.lead]="v().lead"
      [class.merge]="v().kind === 'merge'"
      [class.bad]="v().errors.length"
      [class.warned]="!v().errors.length && v().warnings.length"
      [style.animation-delay.ms]="v().delay"
    >
      <div class="top">
        @if (v().kind === 'merge') {
          <mc-icon class="merge-ico" name="branch" [size]="14" />
        }
        <strong class="title">{{ v().title }}</strong>
        @if (v().lead) {
          <span
            class="pill"
            [title]="'The lead plans the work and gives each role its tasks.' | t"
            >{{ 'Lead' | t }}</span
          >
        }
        @if (v().errors.length || v().warnings.length) {
          <span
            class="count"
            [class.warn]="!v().errors.length"
            [title]="problems()"
            aria-hidden="true"
            >{{ v().errors.length + v().warnings.length }}</span
          >
        }
      </div>
      @if (v().kind === 'merge') {
        <div class="line">
          {{
            v().branch
              ? ('into {branch}' | t: { branch: v().branch })
              : ('into the checked-out branch' | t)
          }}
        </div>
        <div class="line mono" [title]="v().gate">
          {{ v().gate ? ('gate: {command}' | t: { command: v().gate }) : ('no gate' | t) }}
        </div>
        <div class="line">
          @if (v().merger) {
            {{ 'by {role}' | t: { role: v().merger } }} ·
          }
          {{ v().review ? ('tasks reviewed' | t) : ('no review' | t) }}
        </div>
      } @else {
        <div class="line mono id">{{ v().key }}</div>
        <div class="line">{{ v().model }}</div>
        <div class="chips">
          <span class="chip" [class.danger]="v().dangerous" [title]="v().modeHint">
            @if (v().dangerous) {
              <mc-icon name="alert" [size]="11" />
            }
            {{ v().mode }}
          </span>
          @if (v().worktree) {
            <span
              class="chip"
              [title]="'Works in a git worktree of its own; the lead merges its branch.' | t"
              ><mc-icon name="branch" [size]="11" />{{ 'worktree' | t }}</span
            >
          }
          @if (v().readOnly) {
            <span class="chip" [title]="'Changes no files.' | t">{{ 'read-only' | t }}</span>
          }
          @if (v().computer) {
            <span class="chip" [title]="'Runs on {computer}' | t: { computer: v().where }"
              ><mc-icon name="monitor" [size]="11" />{{ v().computer }}</span
            >
          }
        </div>
      }
    </div>
    <span class="inlet" aria-hidden="true"></span>
    @if (v().handle) {
      <span
        class="handle"
        data-handle
        aria-hidden="true"
        [title]="'Drag to another role to send its finished work there next' | t"
      ></span>
    }
  `,
  styles: [
    `
      :host {
        position: absolute;
        left: 0;
        top: 0;
        outline: none;
        cursor: grab;
        touch-action: none;
        user-select: none;
        -webkit-user-select: none;
      }
      :host(.dragging) {
        z-index: 3;
        cursor: grabbing;
      }
      :host(.selected) {
        z-index: 2;
      }
      .card {
        position: relative;
        display: flex;
        flex-direction: column;
        gap: 0.12rem;
        height: 100%;
        padding: 0.5rem 0.7rem 0.55rem;
        overflow: hidden;
        background: var(--panel);
        border: 1px solid var(--border-strong);
        border-radius: var(--radius);
        transition:
          box-shadow 0.2s ease,
          border-color 0.2s ease,
          transform 0.2s ease,
          opacity 0.2s ease;
        animation: node-in 0.36s cubic-bezier(0.2, 0.9, 0.3, 1.15) both;
      }
      .card.lead {
        border-left: 4px solid var(--accent);
      }
      .card.merge {
        border-color: var(--green-border);
        border-left: 4px solid var(--green);
      }
      .card.warned {
        border-color: var(--amber-border);
      }
      .card.bad {
        border-color: var(--red);
      }
      :host(:hover) .card {
        border-color: var(--accent);
      }
      :host(:focus-visible) .card {
        outline: 2px solid var(--focus);
        outline-offset: 2px;
      }
      :host(.selected) .card {
        border-color: var(--accent);
        box-shadow:
          0 0 0 1px var(--accent),
          0 0 26px 3px var(--accent-soft);
      }
      :host(.dragging) .card {
        transform: scale(1.04);
        box-shadow: var(--shadow);
      }
      :host(.link-ok) .card {
        border-style: dashed;
        border-color: var(--accent);
      }
      :host(.link-over) .card {
        transform: scale(1.03);
        box-shadow:
          0 0 0 2px var(--accent),
          0 0 30px 6px var(--accent-soft);
      }
      :host(.link-no) .card {
        opacity: 0.5;
      }
      .top {
        display: flex;
        align-items: center;
        gap: 0.35rem;
        min-width: 0;
      }
      .title {
        flex: 1;
        min-width: 0;
        font-size: 0.9rem;
      }
      .title,
      .line {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .merge-ico {
        color: var(--green);
      }
      .pill,
      .count,
      .chip {
        flex: 0 0 auto;
        border-radius: 1rem;
        line-height: 1.35;
        white-space: nowrap;
      }
      .pill {
        font-size: 0.66rem;
        font-weight: 650;
        padding: 0.02rem 0.45rem;
        background: var(--accent);
        color: var(--on-accent);
      }
      .count {
        min-width: 1.2rem;
        text-align: center;
        font-size: 0.7rem;
        font-weight: 700;
        padding: 0 0.3rem;
        background: var(--red);
        color: var(--on-accent);
      }
      .count.warn {
        background: var(--amber-soft);
        color: var(--amber);
        box-shadow: inset 0 0 0 1px var(--amber-border);
      }
      .line {
        font-size: 0.78rem;
        color: var(--muted);
      }
      .line.mono {
        font-size: 0.72rem;
      }
      .id {
        color: var(--faint);
      }
      .chips {
        display: flex;
        gap: 0.25rem;
        margin-top: auto;
        overflow: hidden;
      }
      .chip {
        display: inline-flex;
        align-items: center;
        gap: 0.2rem;
        font-size: 0.66rem;
        padding: 0.02rem 0.42rem;
        border: 1px solid var(--border);
        color: var(--muted);
      }
      .chip.danger {
        color: var(--red);
        border-color: var(--red-border);
        background: var(--red-soft);
      }
      .inlet,
      .handle {
        position: absolute;
        top: 50%;
        border-radius: 50%;
      }
      .inlet {
        left: -4px;
        width: 8px;
        height: 8px;
        margin-top: -4px;
        background: var(--border-strong);
      }
      .handle {
        right: -8px;
        width: 16px;
        height: 16px;
        margin-top: -8px;
        background: var(--panel);
        border: 2px solid var(--accent);
        cursor: crosshair;
        transition:
          transform 0.15s ease,
          background 0.15s ease;
      }
      /* A finger-sized target around the dot. */
      .handle::after {
        content: '';
        position: absolute;
        inset: -12px;
        border-radius: 50%;
      }
      :host(:hover) .handle,
      :host(.selected) .handle {
        transform: scale(1.25);
        background: var(--accent);
      }
      @keyframes node-in {
        from {
          opacity: 0;
          transform: translateY(6px) scale(0.9);
        }
      }
      @media (prefers-reduced-motion: reduce) {
        .card,
        .handle {
          animation: none;
          transition: none;
        }
      }
    `,
  ],
})
export class TemplateNodeComponent {
  readonly v = input.required<NodeView>();
  protected readonly width = NODE_W;
  protected readonly height = NODE_H;
  protected readonly place = computed(() => `translate(${this.v().x}px, ${this.v().y}px)`);
  protected readonly problems = computed(() =>
    [...this.v().errors, ...this.v().warnings].join('\n'),
  );
}
