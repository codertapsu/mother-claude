import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  ElementRef,
  computed,
  effect,
  inject,
  signal,
  untracked,
  viewChild,
} from '@angular/core';

import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { storageGet, storageSet } from '../../core/platform';
import {
  DANGEROUS_MODES,
  Integration,
  MAX_ROLES,
  MERGE_NODE,
  NodePosition,
  machineWords,
} from '../../core/teams';
import { IconComponent } from '../../shared/icon.component';
import { PERMISSION_MODES, effortLabel, modeLabel } from '../../shared/util';
import { EditorIssue, TemplateEditor } from './template-editor';
import {
  GRID,
  NODE_H,
  NODE_W,
  Selection,
  arrange,
  bounds,
  boxOf,
  canConnect,
  drawnFlow,
  edgePath,
  freeSpot,
  leadOf,
  linkPath,
  mergerOf,
  moveNode,
  rolesOf,
  routeEdge,
  titleOf,
} from './template-graph';
import { NodeView, TemplateNodeComponent } from './template-node.component';

/** One edge: the lead assigning tasks, a handoff, or who merges. */
interface EdgeView {
  key: string;
  kind: 'assign' | 'handoff' | 'merge';
  d: string;
  head: string;
  mid: NodePosition;
  /** How wide its note may be. */
  room: number;
  note: string;
  title: string;
  /** What clicking it selects. */
  sel: Selection;
  selected: boolean;
  /** It touches the selected node. */
  lit: boolean;
  /** The merge edge from the lead, `mergeBy` not set. */
  dflt: boolean;
  bad: boolean;
}

/** What a pointer is doing: panning, moving a node, drawing a handoff. */
type Gesture = { id: number; x: number; y: number; moved: boolean } & (
  | { kind: 'pan'; ox: number; oy: number; clear: boolean }
  | { kind: 'node'; key: string; ox: number; oy: number }
  | { kind: 'link'; key: string }
);

const MIN_ZOOM = 0.3;
const MAX_ZOOM = 2;
const HINT_KEY = 'mc_template_hint';
const ARROWS: Readonly<Record<string, readonly [number, number]>> = {
  ArrowLeft: [-1, 0],
  ArrowRight: [1, 0],
  ArrowUp: [0, -1],
  ArrowDown: [0, 1],
};
const clampZoom = (z: number) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));
const str = (v: unknown) => (typeof v === 'string' ? v : '');
const reducedMotion = () =>
  typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
let nextId = 0;

function problemsText(n: number): string {
  if (!n) return '';
  return n === 1 ? tr('1 problem') : tr('{n} problems', { n });
}

/** A computer for a node's chip: its name, or the OS of `os:windows`. */
function machineShort(spec: string): string {
  const s = spec.trim();
  if (!s.startsWith('os:')) return s;
  const os = s.slice(3).trim();
  return os === 'windows' ? 'Windows' : os === 'macos' ? 'Mac' : os === 'linux' ? 'Linux' : os;
}

function modeHint(mode: string): string {
  const known = PERMISSION_MODES.find((m) => m.value === mode);
  if (known) return tr(known.hint);
  return mode === 'dontAsk' ? tr('Never ask (dangerous)') : '';
}

/**
 * The template's canvas (docs/TEAMS.md "Templates"): a node per role and one
 * for Merge, the lead's faint "assigns tasks" lines, the flow's handoffs and
 * who merges. Nodes move by pointer (snapping to the grid) or arrow keys,
 * handoffs are drawn from a node's dot, the view pans and zooms. Every drag
 * has a way through the panel too; nothing here is editable in a built-in
 * template.
 */
@Component({
  selector: 'mc-template-canvas',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, TemplateNodeComponent, TPipe],
  template: `
    @if (showHint()) {
      <p class="hint" [id]="hintId">
        <mc-icon name="info" [size]="15" />
        <span>{{
          'Drag a role to move it. Drag from the dot on its right edge to another role to send its finished work there next, or to Merge to choose who merges.'
            | t
        }}</span>
        <button
          class="btn ghost small icon-only"
          type="button"
          [attr.aria-label]="'Hide this tip' | t"
          [title]="'Hide this tip' | t"
          (click)="hideHint()"
        >
          <mc-icon name="x" [size]="14" />
        </button>
      </p>
    }
    <p class="visually-hidden" [id]="keysId">
      {{
        ed.editable()
          ? ('Tab to a role to select it. The arrow keys move it, further with Shift, and Delete deletes it. Plus and minus zoom, 0 fits the template in view.'
            | t)
          : ('Tab to a role to select it. Plus and minus zoom, 0 fits the template in view.' | t)
      }}
    </p>
    <div class="frame">
      <div
        #vp
        class="viewport"
        tabindex="0"
        role="group"
        [attr.aria-label]="'Diagram of the template' | t"
        [attr.aria-describedby]="showHint() ? hintId + ' ' + keysId : keysId"
        [class.panning]="panning()"
        [class.linking]="!!linking()"
        [class.smooth]="smooth()"
        [style.background-size]="gridSize()"
        [style.background-position]="gridPos()"
        (pointerdown)="down($event)"
        (pointermove)="move($event)"
        (pointerup)="up($event)"
        (pointercancel)="cancel($event)"
        (wheel)="wheel($event)"
        (keydown)="key($event)"
        (scroll)="scrolled()"
      >
        <div class="world" [class.smooth]="smooth()" [style.transform]="worldTransform()">
          <svg class="edges" width="1" height="1" aria-hidden="true">
            @for (e of edges(); track e.key) {
              <g
                class="edge"
                [class.assign]="e.kind === 'assign'"
                [class.handoff]="e.kind === 'handoff'"
                [class.merge]="e.kind === 'merge'"
                [class.lit]="e.lit"
                [class.selected]="e.selected"
                [class.dflt]="e.dflt"
                [class.bad]="e.bad"
              >
                @if (e.kind === 'assign' || e.dflt) {
                  <path class="line" [attr.d]="e.d" />
                } @else {
                  <path class="line draw" pathLength="1" [attr.d]="e.d" />
                }
                @if (e.kind === 'handoff') {
                  <path class="flow" [attr.d]="e.d" />
                }
                <polygon class="head" [attr.points]="e.head" />
                @if (e.kind !== 'assign') {
                  <path class="hit" [attr.d]="e.d" [attr.data-edge]="e.key">
                    <title>{{ e.title }}</title>
                  </path>
                }
              </g>
            }
            @if (linkLine(); as l) {
              <g class="edge temp">
                <path class="line" [attr.d]="l.d" />
                <polygon class="head" [attr.points]="l.head" />
              </g>
            }
          </svg>
          @for (e of labels(); track e.key) {
            <span
              class="label"
              [class.selected]="e.selected"
              [attr.data-edge]="e.key"
              [title]="e.title"
              [style.max-width.px]="e.room"
              [style.transform]="
                'translate(' + e.mid.x + 'px,' + e.mid.y + 'px) translate(-50%, -135%)'
              "
              >{{ e.note }}</span
            >
          }
          @for (n of nodes(); track n.key) {
            <mc-template-node [v]="n" (focus)="focused(n.key)" (keydown)="nodeKey($event, n.key)" />
          }
          @if (removable(); as e) {
            <button
              class="edge-x"
              type="button"
              [style.transform]="
                'translate(' +
                e.mid.x +
                'px,' +
                e.mid.y +
                'px) scale(' +
                1 / zoom() +
                ') translate(-50%, 35%)'
              "
              [attr.aria-label]="'Remove this handoff' | t"
              [title]="'Remove this handoff' | t"
              (click)="removeSelected()"
            >
              <mc-icon name="x" [size]="13" />
            </button>
          }
        </div>
        @if (!ed.roleCount()) {
          <div class="empty">
            <strong>{{ 'No roles yet' | t }}</strong>
            <span>{{ 'Add the first role: it becomes the lead, who plans the work.' | t }}</span>
            @if (ed.editable()) {
              <button class="btn primary small" type="button" (click)="addRole()">
                <mc-icon name="plus" [size]="14" /> {{ 'Add a role' | t }}
              </button>
            }
          </div>
        }
      </div>
      <div class="tools">
        @if (ed.editable()) {
          <div class="group">
            <button
              class="btn small"
              type="button"
              [disabled]="ed.full()"
              [attr.aria-label]="'Add role' | t"
              [title]="ed.full() ? ('A template has at most {n} roles.' | t: { n: maxRoles }) : ''"
              (click)="addRole()"
            >
              <mc-icon name="plus" [size]="14" /><span class="word">{{ 'Add role' | t }}</span>
            </button>
            <button
              class="btn small"
              type="button"
              [attr.aria-label]="'Auto-arrange' | t"
              [title]="'Place the roles in layers, from the lead to Merge' | t"
              (click)="autoArrange()"
            >
              <mc-icon name="workflow" [size]="14" /><span class="word">{{
                'Auto-arrange' | t
              }}</span>
            </button>
          </div>
        }
        <div class="group zoom" role="group" [attr.aria-label]="'Zoom' | t">
          <button
            class="btn ghost small icon-only"
            type="button"
            [attr.aria-label]="'Zoom out' | t"
            [title]="'Zoom out' | t"
            (click)="zoomBy(1 / 1.25)"
          >
            <mc-icon name="minus" [size]="14" />
          </button>
          <span class="pct">{{ pct() }}</span>
          <button
            class="btn ghost small icon-only"
            type="button"
            [attr.aria-label]="'Zoom in' | t"
            [title]="'Zoom in' | t"
            (click)="zoomBy(1.25)"
          >
            <mc-icon name="plus" [size]="14" />
          </button>
          <button
            class="btn ghost small"
            type="button"
            [title]="'Fit the whole template in view' | t"
            (click)="fit()"
          >
            {{ 'Fit' | t }}
          </button>
        </div>
      </div>
      <div class="legend" aria-hidden="true">
        <span
          ><svg width="22" height="8"><path class="lg-assign" d="M1 4h20" /></svg
          >{{ 'assigns tasks' | t }}</span
        >
        <span
          ><svg width="22" height="8"><path class="lg-handoff" d="M1 4h20" /></svg
          >{{ 'hands its work on' | t }}</span
        >
        <span
          ><svg width="22" height="8"><path class="lg-merge" d="M1 4h20" /></svg
          >{{ 'merges' | t }}</span
        >
      </div>
    </div>
  `,
  styles: [
    `
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
        min-width: 0;
        min-height: 0;
      }
      .hint {
        display: flex;
        align-items: flex-start;
        gap: 0.5rem;
        margin: 0;
        padding: 0.35rem 0.4rem 0.35rem 0.7rem;
        border-radius: var(--radius-sm);
        background: var(--accent-soft);
        font-size: 0.82rem;
        line-height: 1.4;
      }
      .hint > mc-icon {
        color: var(--accent-text);
        margin-top: 0.12rem;
      }
      .hint > span {
        flex: 1;
        align-self: center;
      }
      .frame {
        position: relative;
        flex: 1;
        min-height: 18rem;
        border: 1px solid var(--border);
        border-radius: var(--radius);
        overflow: hidden;
        background: var(--bg);
      }
      .viewport {
        position: absolute;
        inset: 0;
        overflow: hidden;
        outline: none;
        touch-action: none;
        user-select: none;
        -webkit-user-select: none;
        background-image: radial-gradient(circle, var(--border-strong) 1px, transparent 1.5px);
      }
      .viewport:focus-visible {
        box-shadow: inset 0 0 0 2px var(--focus);
      }
      .viewport.panning {
        cursor: grabbing;
      }
      .viewport.linking {
        cursor: crosshair;
      }
      .world {
        position: absolute;
        left: 0;
        top: 0;
        transform-origin: 0 0;
      }
      .smooth,
      .world.smooth {
        transition:
          transform 0.32s cubic-bezier(0.2, 0.8, 0.2, 1),
          background-size 0.32s cubic-bezier(0.2, 0.8, 0.2, 1),
          background-position 0.32s cubic-bezier(0.2, 0.8, 0.2, 1);
      }
      .edges {
        position: absolute;
        left: 0;
        top: 0;
        overflow: visible;
        pointer-events: none;
      }
      .line,
      .flow,
      .hit {
        fill: none;
      }
      .line {
        stroke-width: 2;
        transition:
          stroke-width 0.2s ease,
          filter 0.2s ease;
      }
      .assign .line {
        stroke: var(--faint);
        stroke-width: 1.4;
        stroke-dasharray: 4 5;
        opacity: 0.75;
        animation: fade-in 0.5s ease both;
      }
      .assign .head {
        fill: var(--faint);
        opacity: 0.75;
      }
      .handoff .line {
        stroke: var(--accent);
      }
      .handoff .head,
      .temp .head {
        fill: var(--accent);
      }
      .merge .line {
        stroke: var(--green);
      }
      .merge .head {
        fill: var(--green);
      }
      .merge.dflt .line {
        stroke-dasharray: 6 5;
        opacity: 0.7;
      }
      .bad .line {
        stroke: var(--red);
      }
      .bad .head {
        fill: var(--red);
      }
      .draw {
        stroke-dasharray: 1;
        animation: draw 0.55s ease-out both;
      }
      .flow {
        stroke: var(--accent-text);
        stroke-width: 2.4;
        stroke-linecap: round;
        stroke-dasharray: 0.5 13;
        animation:
          flow 1.2s linear infinite,
          fade-in 0.3s 0.5s ease both;
      }
      .hit {
        stroke: transparent;
        stroke-width: 14;
        pointer-events: stroke;
        cursor: pointer;
      }
      .lit .line {
        stroke-width: 2.6;
        filter: drop-shadow(0 0 3px var(--accent));
      }
      .merge.lit .line {
        filter: drop-shadow(0 0 3px var(--green));
      }
      .selected .line {
        stroke-width: 3.2;
      }
      .temp .line {
        stroke: var(--accent);
        stroke-dasharray: 5 4;
      }
      @keyframes draw {
        from {
          stroke-dashoffset: 1;
        }
        to {
          stroke-dashoffset: 0;
        }
      }
      @keyframes flow {
        to {
          stroke-dashoffset: -13.5;
        }
      }
      @keyframes fade-in {
        from {
          opacity: 0;
        }
      }
      .label {
        position: absolute;
        left: 0;
        top: 0;
        padding: 0.05rem 0.5rem;
        border-radius: 1rem;
        border: 1px solid var(--accent);
        background: var(--panel);
        color: var(--accent-text);
        font-size: 0.7rem;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        cursor: pointer;
        animation: fade-in 0.3s 0.4s ease both;
      }
      .label.selected {
        background: var(--accent);
        color: var(--on-accent);
      }
      .edge-x {
        position: absolute;
        left: 0;
        top: 0;
        z-index: 4;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        width: 28px;
        height: 28px;
        padding: 0;
        border-radius: 50%;
        border: 1px solid var(--red-border);
        background: var(--panel);
        color: var(--red);
        box-shadow: var(--shadow);
        cursor: pointer;
        transform-origin: 0 0;
      }
      .tools {
        position: absolute;
        top: 0.5rem;
        left: 0.5rem;
        right: 0.5rem;
        display: flex;
        flex-wrap: wrap;
        justify-content: space-between;
        gap: 0.4rem;
        pointer-events: none;
      }
      .group {
        display: flex;
        align-items: center;
        gap: 0.35rem;
        pointer-events: auto;
      }
      .group > .btn {
        background: var(--panel);
      }
      .zoom {
        margin-left: auto;
        gap: 0;
        padding: 2px;
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        background: var(--panel);
      }
      .zoom > .btn {
        background: transparent;
      }
      .pct {
        min-width: 2.9rem;
        text-align: center;
        font-size: 0.75rem;
        color: var(--muted);
        font-variant-numeric: tabular-nums;
      }
      .legend {
        position: absolute;
        left: 0.5rem;
        bottom: 0.5rem;
        display: flex;
        gap: 0.8rem;
        padding: 0.2rem 0.55rem;
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        background: var(--panel);
        color: var(--muted);
        font-size: 0.7rem;
        pointer-events: none;
      }
      .legend span {
        display: inline-flex;
        align-items: center;
        gap: 0.3rem;
      }
      .legend path {
        fill: none;
        stroke-width: 2;
      }
      .lg-assign {
        stroke: var(--faint);
        stroke-dasharray: 3 3;
      }
      .lg-handoff {
        stroke: var(--accent);
      }
      .lg-merge {
        stroke: var(--green);
      }
      .empty {
        position: absolute;
        inset: 0;
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: 0.45rem;
        padding: 1rem;
        text-align: center;
        color: var(--muted);
      }
      .empty strong {
        color: var(--text);
      }
      @media (max-width: 899px) {
        .frame {
          flex: none;
          height: max(18rem, 56dvh);
        }
      }
      @media (max-width: 560px) {
        .legend,
        .word {
          display: none;
        }
      }
      @media (max-width: 420px) {
        .pct {
          display: none;
        }
      }
      @media (prefers-reduced-motion: reduce) {
        .smooth,
        .world.smooth,
        .line {
          transition: none;
        }
        .draw,
        .flow,
        .label,
        .assign .line {
          animation: none;
        }
      }
    `,
  ],
})
export class TemplateCanvasComponent {
  protected readonly ed = inject(TemplateEditor);
  private readonly vp = viewChild.required<ElementRef<HTMLElement>>('vp');

  private readonly uid = nextId++;
  protected readonly hintId = `mc-template-hint-${this.uid}`;
  protected readonly keysId = `mc-template-keys-${this.uid}`;
  protected readonly maxRoles = MAX_ROLES;
  protected readonly zoom = signal(1);
  protected readonly pan = signal<NodePosition>({ x: 0, y: 0 });
  /** The view moves smoothly (Fit, the zoom buttons). */
  protected readonly smooth = signal(false);
  protected readonly panning = signal(false);
  /** A node being moved, and where it is. */
  protected readonly dragging = signal<{ key: string; x: number; y: number } | null>(null);
  /** A handoff being drawn: from which role, to where, over which node. */
  protected readonly linking = signal<{
    from: string;
    x: number;
    y: number;
    over: string | null;
  } | null>(null);
  /** Nodes gliding to new places (Auto-arrange): from where, how far. */
  private readonly tween = signal<{ from: Record<string, NodePosition>; k: number } | null>(null);
  /** The first nodes come in one after another. */
  private readonly stagger = signal(true);
  private readonly hintHidden = signal(storageGet(HINT_KEY) === '1');
  private readonly size = signal({ w: 0, h: 0 });

  private gesture: Gesture | null = null;
  private readonly pointers = new Map<number, NodePosition>();
  private pinch: { dist: number; zoom: number; mid: NodePosition; pan: NodePosition } | null = null;
  /** The load the view was last fitted for. */
  private placedFor = -1;
  /** The view follows the canvas's size until the user moves it. */
  private auto = true;
  private smoothTimer: ReturnType<typeof setTimeout> | undefined;
  private staggerTimer: ReturnType<typeof setTimeout> | undefined;
  private frame = 0;

  protected readonly showHint = computed(() => this.ed.editable() && !this.hintHidden());
  protected readonly worldTransform = computed(
    () => `translate(${this.pan().x}px, ${this.pan().y}px) scale(${this.zoom()})`,
  );
  protected readonly gridSize = computed(() => {
    const s = GRID * 2 * this.zoom();
    return `${s}px ${s}px`;
  });
  protected readonly gridPos = computed(() => `${this.pan().x}px ${this.pan().y}px`);
  protected readonly pct = computed(() => `${Math.round(this.zoom() * 100)}%`);

  /** Where each node shows: its place, gliding, or under the pointer. */
  protected readonly positions = computed(() => {
    let out = this.ed.positions();
    const tw = this.tween();
    if (tw) {
      out = Object.fromEntries(
        Object.entries(out).map(([k, p]) => {
          const f = tw.from[k];
          return [k, f ? { x: f.x + (p.x - f.x) * tw.k, y: f.y + (p.y - f.y) * tw.k } : p];
        }),
      );
    }
    const d = this.dragging();
    return d ? { ...out, [d.key]: { x: d.x, y: d.y } } : out;
  });

  /** Each node's problems: a role's, Merge's, and a handoff's that has no
   * edge to show on (one of its ends is missing, or it points to itself). */
  private readonly problems = computed(() => {
    const t = this.ed.template();
    const ids = new Set(rolesOf(t).map((r) => r.id));
    const drawn = new Set(drawnFlow(t).map((h) => `${h.from}\u0000${h.to}`));
    const out = new Map<string, { errors: string[]; warnings: string[] }>();
    const add = (key: string, i: EditorIssue) => {
      const p = out.get(key) ?? { errors: [], warnings: [] };
      (i.level === 'error' ? p.errors : p.warnings).push(i.message);
      out.set(key, p);
    };
    for (const i of this.ed.issues()) {
      const g = i.target;
      if (g.kind === 'role' && ids.has(g.id)) add(g.id, i);
      else if (g.kind === 'merge') add(MERGE_NODE, i);
      else if (g.kind === 'handoff' && !drawn.has(`${g.from}\u0000${g.to}`)) {
        if (ids.has(g.from)) add(g.from, i);
        else if (ids.has(g.to)) add(g.to, i);
      }
    }
    return out;
  });

  protected readonly nodes = computed<NodeView[]>(() => {
    const t = this.ed.template();
    const pos = this.positions();
    const sel = this.ed.selection();
    const link = this.linking();
    const drag = this.dragging();
    const editable = this.ed.editable();
    const stagger = this.stagger();
    const problems = this.problems();
    const linkState = (key: string): NodeView['link'] => {
      if (!link || key === link.from) return '';
      if (link.over === key) return 'over';
      return canConnect(t, link.from, key) ? 'ok' : 'no';
    };
    const none = { errors: [] as string[], warnings: [] as string[] };
    const roles = rolesOf(t);
    const views: NodeView[] = roles.map((r, i) => {
      const p = problems.get(r.id) ?? none;
      const mode = str(r.permissionMode) || 'default';
      const title = str(r.title).trim() ? str(r.title) : r.id;
      const model = [str(r.model) || tr('default model'), effortLabel(str(r.effort))]
        .filter(Boolean)
        .join(' · ');
      const lead = r.lead === true;
      return {
        key: r.id,
        kind: 'role',
        ...pos[r.id],
        title,
        lead,
        model,
        mode: modeLabel(mode),
        modeHint: modeHint(mode),
        dangerous: DANGEROUS_MODES.includes(mode),
        worktree: r.worktree === true,
        readOnly: r.readOnly === true,
        computer: machineShort(str(r.machine)),
        where: machineWords(str(r.machine)),
        branch: '',
        gate: '',
        review: false,
        merger: '',
        errors: p.errors,
        warnings: p.warnings,
        selected: sel?.kind === 'role' && sel.id === r.id,
        link: linkState(r.id),
        dragging: drag?.key === r.id,
        handle: editable,
        delay: stagger ? Math.min(i * 45, 450) : 0,
        label: [
          `${title} (${r.id})`,
          lead ? tr('lead') : '',
          problemsText(p.errors.length + p.warnings.length),
        ]
          .filter(Boolean)
          .join(', '),
      };
    });
    const integ: Integration =
      t.integration && typeof t.integration === 'object' ? t.integration : {};
    const m = mergerOf(t);
    const merger = m ? titleOf(t, m.id) : '';
    const p = problems.get(MERGE_NODE) ?? none;
    views.push({
      key: MERGE_NODE,
      kind: 'merge',
      ...pos[MERGE_NODE],
      title: tr('Merge'),
      lead: false,
      model: '',
      mode: '',
      modeHint: '',
      dangerous: false,
      worktree: false,
      readOnly: false,
      computer: '',
      where: '',
      branch: str(integ.branch),
      gate: str(integ.gate),
      review: integ.review !== false,
      merger,
      errors: p.errors,
      warnings: p.warnings,
      selected: sel?.kind === 'merge',
      link: linkState(MERGE_NODE),
      dragging: drag?.key === MERGE_NODE,
      handle: false,
      delay: stagger ? Math.min(roles.length * 45, 450) : 0,
      label: [
        tr('Merge'),
        merger ? tr('by {role}', { role: merger }) : '',
        problemsText(p.errors.length + p.warnings.length),
      ]
        .filter(Boolean)
        .join(', '),
    });
    return views;
  });

  protected readonly edges = computed<EdgeView[]>(() => {
    const t = this.ed.template();
    const pos = this.positions();
    const sel = this.ed.selection();
    const issues = this.ed.issues();
    const boxes = new Map(Object.entries(pos).map(([k, p]) => [k, boxOf(p)]));
    const box = (k: string) => boxes.get(k) ?? null;
    /** The edge from `a` into `b`, around the other nodes. */
    const route = (a: string, b: string, side: 'above' | 'below', clearance?: number) =>
      routeEdge(
        boxes.get(a) ?? boxOf({ x: 0, y: 0 }),
        boxes.get(b) ?? boxOf({ x: 0, y: 0 }),
        [...boxes].filter(([k]) => k !== a && k !== b).map(([, v]) => v),
        side,
        clearance,
      );
    const touches = (a: string, b: string) =>
      sel?.kind === 'role' && (sel.id === a || sel.id === b);
    const flow = drawnFlow(t);
    const out: EdgeView[] = [];
    const lead = leadOf(t)?.id;
    if (lead && box(lead)) {
      for (const r of rolesOf(t)) {
        if (r.id === lead || !box(r.id) || flow.some((h) => h.from === lead && h.to === r.id)) {
          continue;
        }
        out.push({
          key: `a:${r.id}`,
          kind: 'assign',
          ...route(lead, r.id, 'above'),
          note: '',
          title: '',
          sel: null,
          selected: false,
          lit: touches(lead, r.id),
          dflt: false,
          bad: false,
        });
      }
    }
    for (const h of flow) {
      if (!box(h.from) || !box(h.to)) continue;
      const selected = sel?.kind === 'handoff' && sel.from === h.from && sel.to === h.to;
      const wrong = issues
        .filter(
          (i) => i.target.kind === 'handoff' && i.target.from === h.from && i.target.to === h.to,
        )
        .map((i) => i.message);
      const note = str(h.note);
      const pair = `${titleOf(t, h.from)} → ${titleOf(t, h.to)}`;
      out.push({
        key: `h:${h.from}>${h.to}`,
        kind: 'handoff',
        ...route(h.from, h.to, 'below'),
        note,
        title: [note ? `${pair}: ${note}` : pair, ...wrong].join('\n'),
        sel: { kind: 'handoff', from: h.from, to: h.to },
        selected,
        lit: selected || touches(h.from, h.to),
        dflt: false,
        bad: wrong.length > 0,
      });
    }
    const m = mergerOf(t);
    if (m && box(m.id) && box(MERGE_NODE)) {
      const selected = sel?.kind === 'merge';
      out.push({
        key: `m:${m.id}`,
        kind: 'merge',
        ...route(m.id, MERGE_NODE, 'below', 40),
        note: '',
        title: tr('{role} merges the work', { role: titleOf(t, m.id) }),
        sel: { kind: 'merge' },
        selected,
        lit: selected || touches(m.id, MERGE_NODE),
        dflt: !m.explicit,
        bad: false,
      });
    }
    return out;
  });

  protected readonly labels = computed(() =>
    this.edges().filter((e) => e.kind === 'handoff' && e.note),
  );
  /** The selected handoff, with its ✕ (editable only). */
  protected readonly removable = computed(() =>
    this.ed.editable() ? this.edges().find((e) => e.kind === 'handoff' && e.selected) : undefined,
  );
  /** The handoff being drawn: to the pointer, or into the node it is over. */
  protected readonly linkLine = computed(() => {
    const l = this.linking();
    if (!l) return null;
    const pos = this.positions();
    const a = pos[l.from];
    if (!a) return null;
    const over = l.over ? pos[l.over] : null;
    return over ? edgePath(boxOf(a), boxOf(over)) : linkPath(boxOf(a), l);
  });

  constructor() {
    effect((onCleanup) => {
      const el = this.vp().nativeElement;
      const measure = () => this.size.set({ w: el.clientWidth, h: el.clientHeight });
      measure();
      const ro = typeof ResizeObserver === 'function' ? new ResizeObserver(measure) : null;
      ro?.observe(el);
      onCleanup(() => ro?.disconnect());
    });
    // Each template loaded fits into view once the canvas has a size, and
    // again as it grows (the sheet opening) until the user moves the view.
    effect(() => {
      const loads = this.ed.loads();
      const { w, h } = this.size();
      if (!w || !h) return;
      untracked(() => {
        if (this.placedFor !== loads) {
          this.placedFor = loads;
          this.auto = true;
          this.stagger.set(true);
          clearTimeout(this.staggerTimer);
          this.staggerTimer = setTimeout(() => this.stagger.set(false), 1200);
        }
        if (this.auto) this.place(true);
      });
    });
    inject(DestroyRef).onDestroy(() => {
      clearTimeout(this.smoothTimer);
      clearTimeout(this.staggerTimer);
      cancelAnimationFrame(this.frame);
    });
  }

  protected hideHint(): void {
    this.hintHidden.set(true);
    storageSet(HINT_KEY, '1');
  }

  // ─────────────────────────────────────────────────────────── the view

  /** Fit the nodes in view: the first time no smaller than 70 % (the lead
   * shows at the left then), Fit as small as it takes. */
  private place(first: boolean): void {
    const { w, h } = this.size();
    if (!w || !h) return;
    const b = bounds(Object.values(this.ed.positions()).map(boxOf));
    const top = 64;
    const side = 48;
    const bottom = 48;
    const aw = Math.max(1, w - 2 * side);
    const ah = Math.max(1, h - top - bottom);
    const fit = Math.min(aw / b.w, ah / b.h);
    const z = first ? Math.min(1, Math.max(0.7, fit)) : clampZoom(Math.min(fit, 1.25));
    const x = b.w * z <= aw ? side + (aw - b.w * z) / 2 - b.x * z : side - b.x * z;
    const y = b.h * z <= ah ? top + (ah - b.h * z) / 2 - b.y * z : top - b.y * z;
    if (!first) this.ease();
    this.zoom.set(z);
    this.pan.set({ x, y });
  }

  private ease(): void {
    this.smooth.set(true);
    clearTimeout(this.smoothTimer);
    this.smoothTimer = setTimeout(() => this.smooth.set(false), 360);
  }

  /** Zoom to `z` keeping the point (`cx`, `cy`) of the viewport where it is. */
  private zoomAt(cx: number, cy: number, z: number): void {
    const z0 = this.zoom();
    const z1 = clampZoom(z);
    const p = this.pan();
    this.pan.set({ x: cx - ((cx - p.x) * z1) / z0, y: cy - ((cy - p.y) * z1) / z0 });
    this.zoom.set(z1);
  }

  protected zoomBy(f: number): void {
    const { w, h } = this.size();
    this.auto = false;
    this.ease();
    this.zoomAt(w / 2, h / 2, this.zoom() * f);
  }

  protected fit(): void {
    this.auto = false;
    this.place(false);
  }

  private rect(): DOMRect {
    return this.vp().nativeElement.getBoundingClientRect();
  }

  /** A point of the screen in the canvas's own coordinates. */
  private toWorld(clientX: number, clientY: number): NodePosition {
    const r = this.rect();
    const p = this.pan();
    const z = this.zoom();
    return { x: (clientX - r.left - p.x) / z, y: (clientY - r.top - p.y) / z };
  }

  /** The node at a point of the canvas (the top one), other than `except`. */
  private nodeAt(w: NodePosition, except: string): string | null {
    const pos = this.positions();
    const keys = Object.keys(pos).reverse();
    return (
      keys.find((k) => {
        const p = pos[k];
        return (
          k !== except &&
          w.x >= p.x - 6 &&
          w.x <= p.x + NODE_W + 6 &&
          w.y >= p.y - 6 &&
          w.y <= p.y + NODE_H + 6
        );
      }) ?? null
    );
  }

  protected wheel(e: WheelEvent): void {
    e.preventDefault();
    this.auto = false;
    const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? 400 : 1;
    if (e.ctrlKey || e.metaKey) {
      const r = this.rect();
      this.zoomAt(
        e.clientX - r.left,
        e.clientY - r.top,
        this.zoom() * Math.exp(-e.deltaY * unit * 0.0025),
      );
    } else {
      this.pan.update((p) => ({ x: p.x - e.deltaX * unit, y: p.y - e.deltaY * unit }));
    }
  }

  /** Focusing a node scrolls the viewport: that becomes a pan instead. */
  protected scrolled(): void {
    const el = this.vp().nativeElement;
    const { scrollLeft: sx, scrollTop: sy } = el;
    if (!sx && !sy) return;
    el.scrollLeft = 0;
    el.scrollTop = 0;
    this.auto = false;
    this.pan.update((p) => ({ x: p.x - sx, y: p.y - sy }));
  }

  // ─────────────────────────────────────────────────────────── selecting

  private pick(key: string, el?: HTMLElement): void {
    this.ed.select(key === MERGE_NODE ? { kind: 'merge' } : { kind: 'role', id: key });
    el?.focus({ preventScroll: true });
  }

  /** A node reached with Tab is selected. */
  protected focused(key: string): void {
    const s = this.ed.selection();
    const already = key === MERGE_NODE ? s?.kind === 'merge' : s?.kind === 'role' && s.id === key;
    if (!already) this.pick(key);
  }

  private focusViewport(): void {
    this.vp().nativeElement.focus({ preventScroll: true });
  }

  protected removeSelected(): void {
    const s = this.ed.selection();
    if (s?.kind !== 'handoff') return;
    this.ed.disconnect(s.from, s.to);
    this.focusViewport();
  }

  // ─────────────────────────────────────────────────────────── editing

  protected addRole(): void {
    const { w, h } = this.size();
    const z = this.zoom();
    const p = this.pan();
    const centre = { x: (w / 2 - p.x) / z - NODE_W / 2, y: (h / 2 - p.y) / z - NODE_H / 2 };
    const taken = Object.values(this.ed.positions()).map(boxOf);
    this.ed.addRole(freeSpot(taken, centre));
  }

  protected autoArrange(): void {
    const before = this.ed.positions();
    this.auto = false;
    this.ed.apply(arrange(this.ed.template()));
    this.glide(before);
    this.place(false);
  }

  /** Nodes glide from where they were to their new places. */
  private glide(from: Record<string, NodePosition>): void {
    cancelAnimationFrame(this.frame);
    if (reducedMotion() || typeof requestAnimationFrame !== 'function') return;
    const start = performance.now();
    const step = (now: number) => {
      const k = Math.min(1, (now - start) / 380);
      this.tween.set(k < 1 ? { from, k: 1 - Math.pow(1 - k, 3) } : null);
      if (k < 1) this.frame = requestAnimationFrame(step);
    };
    this.tween.set({ from, k: 0 });
    this.frame = requestAnimationFrame(step);
  }

  // ─────────────────────────────────────────────────────────── keys

  /** On a node: arrows nudge it (Shift: further), Delete deletes it. */
  protected nodeKey(e: KeyboardEvent, key: string): void {
    const dir = ARROWS[e.key];
    if (dir) {
      if (!this.ed.editable()) return;
      e.preventDefault();
      const step = e.shiftKey ? GRID * 4 : GRID;
      const p = this.ed.positions()[key];
      if (p) {
        const to = { x: p.x + dir[0] * step, y: p.y + dir[1] * step };
        this.ed.apply(moveNode(this.ed.template(), key, to));
      }
      return;
    }
    if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      if (key !== MERGE_NODE && this.ed.deleteRole(key)) this.focusViewport();
      return;
    }
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      this.pick(key);
    }
  }

  /** On the canvas: +/− zoom, 0 fits, Delete removes the selected handoff,
   * Escape stops a drag. */
  protected key(e: KeyboardEvent): void {
    if (e.defaultPrevented) return;
    if (e.key === 'Escape' && this.gesture) {
      e.preventDefault();
      e.stopPropagation();
      this.abort();
      return;
    }
    if (e.key === '+' || e.key === '=') this.zoomBy(1.25);
    else if (e.key === '-' || e.key === '_') this.zoomBy(1 / 1.25);
    else if (e.key === '0') this.fit();
    else if (e.key === 'Delete' || e.key === 'Backspace') {
      const s = this.ed.selection();
      if (s?.kind === 'handoff' && this.ed.editable()) this.ed.disconnect(s.from, s.to);
      else if (s?.kind === 'role') this.ed.deleteRole(s.id);
      else return;
    } else return;
    e.preventDefault();
  }

  // ─────────────────────────────────────────────────────────── pointers

  private capture(id: number): void {
    try {
      this.vp().nativeElement.setPointerCapture(id);
    } catch {
      // Not an active pointer (a synthetic event): the events come here anyway.
    }
  }

  protected down(e: PointerEvent): void {
    if (e.pointerType === 'mouse' && e.button !== 0) return;
    const target = e.target as Element;
    if (target.closest('button, a, input, textarea, select')) return;
    this.pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    this.capture(e.pointerId);
    if (this.pointers.size === 2) {
      this.startPinch();
      return;
    }
    if (this.pointers.size > 2) return;
    const start = { id: e.pointerId, x: e.clientX, y: e.clientY, moved: false };
    const node = target.closest<HTMLElement>('[data-node]');
    const key = node?.getAttribute('data-node') ?? '';
    if (node && target.closest('[data-handle]') && this.ed.editable()) {
      e.preventDefault();
      this.pick(key, node);
      this.gesture = { ...start, kind: 'link', key };
      return;
    }
    if (node) {
      this.pick(key, node);
      const p = this.ed.positions()[key];
      this.gesture =
        this.ed.editable() && p
          ? { ...start, kind: 'node', key, ox: p.x, oy: p.y }
          : { ...start, kind: 'pan', ox: this.pan().x, oy: this.pan().y, clear: false };
      return;
    }
    const edge = target.closest('[data-edge]')?.getAttribute('data-edge');
    const hit = edge ? this.edges().find((x) => x.key === edge) : undefined;
    if (hit?.sel) this.ed.select(hit.sel);
    this.focusViewport();
    this.gesture = { ...start, kind: 'pan', ox: this.pan().x, oy: this.pan().y, clear: !hit };
  }

  protected move(e: PointerEvent): void {
    if (!this.pointers.has(e.pointerId)) return;
    this.pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (this.pinch) {
      this.pinchMove();
      return;
    }
    const g = this.gesture;
    if (!g || g.id !== e.pointerId) return;
    const dx = e.clientX - g.x;
    const dy = e.clientY - g.y;
    if (!g.moved) {
      if (Math.hypot(dx, dy) < 3) return;
      g.moved = true;
    }
    if (g.kind === 'pan') {
      this.auto = false;
      this.panning.set(true);
      this.pan.set({ x: g.ox + dx, y: g.oy + dy });
    } else if (g.kind === 'node') {
      const z = this.zoom();
      this.dragging.set({ key: g.key, x: g.ox + dx / z, y: g.oy + dy / z });
    } else {
      const w = this.toWorld(e.clientX, e.clientY);
      const over = this.nodeAt(w, g.key);
      const ok = over && canConnect(this.ed.template(), g.key, over);
      this.linking.set({ from: g.key, x: w.x, y: w.y, over: ok ? over : null });
    }
  }

  protected up(e: PointerEvent): void {
    this.pointers.delete(e.pointerId);
    if (this.pinch) {
      if (this.pointers.size < 2) this.pinch = null;
      return;
    }
    const g = this.gesture;
    if (!g || g.id !== e.pointerId) return;
    this.gesture = null;
    this.panning.set(false);
    if (g.kind === 'node') {
      const d = this.dragging();
      this.dragging.set(null);
      if (g.moved && d) this.ed.apply(moveNode(this.ed.template(), g.key, d));
    } else if (g.kind === 'link') {
      const l = this.linking();
      this.linking.set(null);
      if (l?.over) this.ed.connect(g.key, l.over);
    } else if (!g.moved && g.clear) {
      this.ed.select(null);
    }
  }

  protected cancel(e: PointerEvent): void {
    this.pointers.delete(e.pointerId);
    if (this.pointers.size < 2) this.pinch = null;
    if (this.gesture?.id === e.pointerId) this.abort();
  }

  /** Stop what the pointer was doing, changing nothing. */
  private abort(): void {
    this.gesture = null;
    this.dragging.set(null);
    this.linking.set(null);
    this.panning.set(false);
  }

  private startPinch(): void {
    this.abort();
    const [a, b] = [...this.pointers.values()];
    const r = this.rect();
    this.pinch = {
      dist: Math.hypot(a.x - b.x, a.y - b.y) || 1,
      zoom: this.zoom(),
      mid: { x: (a.x + b.x) / 2 - r.left, y: (a.y + b.y) / 2 - r.top },
      pan: this.pan(),
    };
  }

  private pinchMove(): void {
    const p = this.pinch;
    if (!p) return;
    this.auto = false;
    const [a, b] = [...this.pointers.values()];
    const r = this.rect();
    const z = clampZoom((p.zoom * Math.hypot(a.x - b.x, a.y - b.y)) / p.dist);
    const mid = { x: (a.x + b.x) / 2 - r.left, y: (a.y + b.y) / 2 - r.top };
    const wx = (p.mid.x - p.pan.x) / p.zoom;
    const wy = (p.mid.y - p.pan.y) / p.zoom;
    this.zoom.set(z);
    this.pan.set({ x: mid.x - wx * z, y: mid.y - wy * z });
  }
}
