import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  computed,
  effect,
  inject,
  signal,
  untracked,
  viewChild,
} from '@angular/core';

import { DefaultsService, FALLBACK_EFFORTS } from '../../core/defaults.service';
import { NoticeService } from '../../core/notice.service';
import { NowService } from '../../core/now.service';
import { PermissionMode, SessionSummary } from '../../core/models';
import { CapLimitsComponent } from '../../shared/cap-limits.component';
import { capLimits, capThinking, capUltracode } from '../../shared/caps';
import { ChannelBadgeComponent } from '../../shared/channel-badge.component';
import { ChipComponent } from '../../shared/chip.component';
import { ContextMeterComponent } from '../../shared/context-meter.component';
import { CostComponent } from '../../shared/cost.component';
import { IconComponent } from '../../shared/icon.component';
import {
  effortText,
  explainFast,
  explainThinking,
  explainUltracode,
  fastText,
  noEffortLevels,
  thinkingLabel,
  thinkingTitle,
  ultracodeText,
  ultracodeUnknownTitle,
} from '../../shared/session-settings';
import { SheetComponent } from '../../shared/sheet.component';
import { PERMISSION_MODES, effortLabel, modeLabel, switchPlace } from '../../shared/util';
import { PickerKind, SessionContext } from './session-context';

type ExplainKind = PickerKind | 'ultracode' | 'thinking' | 'fast';

interface Why {
  title: string;
  lines: string[];
  /** Show what can't be done here and the fixes (continue here, …). */
  limits: boolean;
  /** Offer to withdraw an ultracode request the model can't run. */
  cancelUltracode?: boolean;
}

/**
 * The session's second header row (it scrolls horizontally and scrolls away
 * with the page): where it runs and how it's reached, then model, mode and
 * effort — native pickers when the session can change them, otherwise a
 * disabled chip whose tap explains why (with the fixes) — ultracode and
 * thinking (switches on sessions that take them, an on/off choice while
 * thinking's setting is unknown, otherwise chips whose tap says how the
 * value is known — or, for ultracode not known yet in a session running
 * elsewhere, why and when it will be; an ultracode request the model can't
 * run can be withdrawn from its sheet), fast mode, context, cost, branch,
 * PR, running agents, todos, queue and the last error.
 */
@Component({
  selector: 'mc-session-chips',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    CapLimitsComponent,
    ChannelBadgeComponent,
    ChipComponent,
    ContextMeterComponent,
    CostComponent,
    IconComponent,
    SheetComponent,
  ],
  template: `
    @if (ctx.session(); as s) {
      <div class="chips" role="group" aria-label="Session settings and stats">
        <mc-channel-badge [surface]="s.surface" [control]="s.control" [session]="s" />

        <!-- Model -->
        @if (s.control.setModel && s.live) {
          <label class="pick" title="Model">
            <mc-icon name="sparkle" [size]="13" />
            <span class="pv">{{ s.model?.label || 'Model' }}</span>
            <mc-icon name="chevronDown" [size]="12" />
            <select #modelSel aria-label="Model" (change)="onModel($event)">
              <option value="" disabled [selected]="true">{{ s.model?.label || 'Choose…' }}</option>
              @for (m of models(); track m.value) {
                <option [value]="m.value">{{ m.label }}</option>
              }
            </select>
          </label>
        } @else if (s.model) {
          <button class="pick off" type="button" (click)="explain('model')">
            <mc-icon name="sparkle" [size]="13" />
            <span class="pv">{{ s.model.label }}</span>
          </button>
        }

        <!-- Permission mode -->
        @if (s.control.setMode !== 'none' && s.live) {
          <label
            class="pick"
            [class.danger]="s.permissionMode === 'bypassPermissions'"
            [title]="
              s.control.setMode === 'on-prompt'
                ? 'Permission mode (applies with your next message)'
                : 'Permission mode'
            "
          >
            <mc-icon name="shield" [size]="13" />
            <span class="pv">{{ mode(s.permissionMode) }}</span>
            <mc-icon name="chevronDown" [size]="12" />
            <select #modeSel aria-label="Permission mode" (change)="onMode($event)">
              <option value="" disabled [selected]="true">{{ mode(s.permissionMode) }}</option>
              @for (m of modes; track m.value) {
                <option [value]="m.value" [disabled]="m.dangerous && !ctx.canDangerous()">
                  {{ m.label }} — {{ m.hint }}
                </option>
              }
            </select>
          </label>
        } @else {
          <button
            class="pick off"
            type="button"
            [class.danger]="s.permissionMode === 'bypassPermissions'"
            (click)="explain('mode')"
          >
            <mc-icon name="shield" [size]="13" />
            <span class="pv">{{ mode(s.permissionMode) }}</span>
          </button>
        }

        <!-- Effort -->
        @if (s.control.setEffort && s.live) {
          <label class="pick" [title]="'Effort: ' + (effortWhy() || 'not reported')">
            <mc-icon name="gauge" [size]="13" />
            <span class="pv">{{
              s.effort ? effort(s.effort) : noEffort() ? 'No effort' : 'Effort'
            }}</span>
            <mc-icon name="chevronDown" [size]="12" />
            <select #effortSel aria-label="Effort" (change)="onEffort($event)">
              <option value="" disabled [selected]="true">
                {{ effort(s.effort) || 'Choose…' }}
              </option>
              @for (e of efforts(); track e) {
                <option [value]="e">{{ effort(e) }}</option>
              }
            </select>
          </label>
        } @else if (s.effort) {
          <button
            class="pick off"
            type="button"
            [attr.aria-label]="'Effort: ' + effortWhy()"
            (click)="explain('effort')"
          >
            <mc-icon name="gauge" [size]="13" />
            <span class="pv">{{ effort(s.effort) }}</span>
          </button>
        }

        <!-- Ultracode: a switch where it can be changed, else how it's known
             (an unavailable request stays a chip, whose tap can withdraw it) -->
        @if (ultracodeSwitch()) {
          <button
            class="pick"
            type="button"
            role="switch"
            aria-label="Ultracode"
            [title]="ultracodeTitle()"
            [class.on]="s.ultracode === 'on'"
            [attr.aria-checked]="s.ultracode === 'on'"
            [disabled]="busy() === 'ultracode' || !ctx.canAct()"
            (click)="toggleUltracode()"
          >
            <mc-icon name="workflow" [size]="13" />
            <span class="pv">Ultracode {{ s.ultracode === 'on' ? 'on' : 'off' }}</span>
          </button>
        } @else if (s.ultracode) {
          <button
            class="pick"
            type="button"
            [class.off]="!ultracodeCancel()"
            [class.on]="s.ultracode === 'on'"
            [class.warn]="s.ultracode === 'unavailable'"
            [title]="ultracodeTitle()"
            [attr.aria-label]="ultracodeTitle()"
            (click)="explain('ultracode')"
          >
            <mc-icon name="workflow" [size]="13" />
            <span class="pv">Ultracode {{ s.ultracode }}</span>
          </button>
        } @else if (ultracodeUnknown()) {
          <!-- Not known yet in a session running elsewhere (VS Code notes a
               switch only with the next message): say so, and why. -->
          <button
            class="pick off"
            type="button"
            [title]="ultracodeUnknownTitle()"
            [attr.aria-label]="ultracodeUnknownTitle()"
            (click)="explain('ultracode')"
          >
            <mc-icon name="workflow" [size]="13" />
            <span class="pv">Ultracode ?</span>
          </button>
        }

        <!-- Thinking: a switch where it can be changed (a choice while the
             setting is unknown), else how it's known -->
        @if (thinkingSwitch()) {
          @if (s.thinking) {
            <button
              class="pick"
              type="button"
              role="switch"
              aria-label="Thinking"
              [class.on]="thinkingOn()"
              [attr.aria-checked]="thinkingOn()"
              [title]="thinkingChipTitle()"
              [disabled]="thinkingLocked()"
              (click)="toggleThinking()"
            >
              <mc-icon name="lightbulb" [size]="13" />
              <span class="pv">Thinking {{ thinkingOn() ? 'on' : 'off' }}</span>
            </button>
          } @else {
            <label class="pick" [class.locked]="thinkingLocked()" [title]="thinkingDefaultTitle">
              <mc-icon name="lightbulb" [size]="13" />
              <span class="pv">Thinking default</span>
              <mc-icon name="chevronDown" [size]="12" />
              <select
                aria-label="Thinking"
                [disabled]="thinkingLocked()"
                (change)="onThinking($event)"
              >
                <option value="" disabled [selected]="true">Claude Code's default</option>
                <option value="on">On</option>
                <option value="off">Off</option>
              </select>
            </label>
          }
        } @else if (thinkingChipText()) {
          <button
            class="pick off"
            type="button"
            [title]="thinkingChipTitle()"
            [attr.aria-label]="thinkingChipTitle()"
            (click)="explain('thinking')"
          >
            <mc-icon name="lightbulb" [size]="13" />
            <span class="pv">{{ thinkingChipText() }}</span>
          </button>
        }

        @if (s.fast) {
          <button
            class="pick off"
            type="button"
            [title]="fastTitle()"
            [attr.aria-label]="fastTitle()"
            (click)="explain('fast')"
          >
            <mc-icon name="zap" [size]="13" />
            <span class="pv">Fast</span>
          </button>
        }

        @if (s.context) {
          <mc-context-meter [context]="s.context" />
        }
        @if (s.costUsd !== undefined && s.costUsd !== null) {
          <mc-chip><mc-cost [usd]="s.costUsd" [final]="!!s.costFinal" /></mc-chip>
        }
        @if (s.agents.running || s.agents.backgroundRunning || s.agents.workflowsRunning) {
          <button class="pick" type="button" (click)="ctx.tab.set('agents')">
            <mc-icon name="agents" [size]="13" />
            <span class="pv">{{ agentsText() }}</span>
          </button>
        }
        @if (s.todos; as t) {
          @if (t.total) {
            <mc-chip icon="check" [tone]="t.done === t.total ? 'green' : 'muted'"
              >{{ t.done }}/{{ t.total }} todos</mc-chip
            >
          }
        }
        @if (s.gitBranch) {
          <mc-chip icon="branch">{{ s.gitBranch }}</mc-chip>
        }
        @if (s.prLink) {
          <a class="pick" [href]="s.prLink.url" target="_blank" rel="noopener noreferrer">
            <mc-icon name="external" [size]="13" />
            <span class="pv">PR #{{ s.prLink.number }}</span>
          </a>
        }
        @if (s.queued) {
          <mc-chip tone="blue" icon="clock">{{ s.queued }} queued</mc-chip>
        }
        @if (s.remoteControl) {
          <mc-chip tone="accent" icon="globe">Remote Control on</mc-chip>
        }
        @if (s.lastError) {
          <mc-chip tone="red" icon="alert">{{ s.lastError.kind }}</mc-chip>
        }
      </div>

      <mc-sheet [open]="!!why()" [title]="why()?.title ?? ''" (closed)="why.set(null)">
        @for (line of why()?.lines ?? []; track $index) {
          <p>{{ line }}</p>
        }
        @if (why()?.cancelUltracode) {
          <p>
            <button
              class="btn small"
              type="button"
              [disabled]="busy() === 'ultracode' || !ctx.canAct()"
              (click)="cancelUltracode()"
            >
              <mc-icon name="x" [size]="14" /> Cancel the ultracode request
            </button>
          </p>
        }
        @if (why()?.limits) {
          <mc-cap-limits [session]="s" (done)="why.set(null)" />
        }
      </mc-sheet>
    }
  `,
  styles: [
    `
      :host {
        display: block;
        min-width: 0;
      }
      .chips {
        display: flex;
        align-items: center;
        gap: 0.35rem;
        overflow-x: auto;
        scrollbar-width: none;
        padding: 0.35rem 0.1rem;
      }
      .chips::-webkit-scrollbar {
        display: none;
      }
      .chips > * {
        flex: 0 0 auto;
      }
      /* Phones: wrap onto more lines instead of scrolling, so every chip
         (effort, ultracode, thinking, …) is visible without a sideways swipe. */
      @media (max-width: 767px) {
        .chips {
          flex-wrap: wrap;
          overflow-x: visible;
        }
      }
      .pick {
        position: relative;
        appearance: none;
        display: inline-flex;
        align-items: center;
        gap: 0.3rem;
        min-height: 32px;
        padding: 0.1rem 0.55rem;
        border-radius: 0.5rem;
        border: 1px solid var(--border-strong);
        background: var(--panel);
        color: var(--text);
        font: inherit;
        font-size: 0.78rem;
        white-space: nowrap;
        text-decoration: none;
        cursor: pointer;
      }
      .pick:focus-within {
        outline: 2px solid var(--focus);
        outline-offset: 1px;
      }
      .pick mc-icon {
        color: var(--muted);
      }
      .pick.off {
        border-style: dashed;
        color: var(--muted);
      }
      .pick.danger {
        border-color: var(--red-border);
        color: var(--red);
      }
      .pick.on {
        border-color: var(--accent);
        background: var(--accent-soft);
        color: var(--accent-text);
      }
      .pick.on mc-icon {
        color: var(--accent-text);
      }
      .pick.warn {
        border-color: var(--amber-border);
        color: var(--amber);
      }
      .pick:disabled,
      .pick.locked {
        opacity: 0.6;
        cursor: not-allowed;
      }
      .pick.locked select {
        cursor: not-allowed;
      }
      .pick select {
        position: absolute;
        inset: 0;
        width: 100%;
        height: 100%;
        min-height: 0;
        opacity: 0;
        cursor: pointer;
        font-size: 16px;
      }
      @media (max-width: 767px), (pointer: coarse) {
        .pick {
          min-height: 36px;
        }
      }
    `,
  ],
})
export class SessionChipsComponent {
  protected readonly ctx = inject(SessionContext);
  private defaults = inject(DefaultsService);
  private notices = inject(NoticeService);
  private clock = inject(NowService);

  private readonly modelSel = viewChild<ElementRef<HTMLSelectElement>>('modelSel');
  private readonly modeSel = viewChild<ElementRef<HTMLSelectElement>>('modeSel');
  private readonly effortSel = viewChild<ElementRef<HTMLSelectElement>>('effortSel');

  protected readonly why = signal<Why | null>(null);
  /** The switch whose change is on its way. */
  protected readonly busy = signal<'ultracode' | 'thinking' | null>(null);
  protected readonly modes = PERMISSION_MODES;
  protected readonly mode = modeLabel;
  protected readonly effort = effortLabel;
  /** Claude Code applies no effort level: the model has none (Haiku). */
  protected readonly noEffort = computed(() => {
    const s = this.ctx.session();
    return !!s && noEffortLevels(s);
  });
  protected readonly effortWhy = computed(() => {
    const s = this.ctx.session();
    if (!s) return '';
    return effortText(s) || (noEffortLevels(s) ? 'none, this model has no effort levels' : '');
  });
  /** Ultracode can be switched here (and its state is known). */
  protected readonly ultracodeSwitch = computed(() => {
    const s = this.ctx.session();
    return !!s?.live && capUltracode(s.control) && (s.ultracode === 'on' || s.ultracode === 'off');
  });
  /** A request the model or workspace can't run, which the session lets
   * us withdraw (it would otherwise come back with a capable model). */
  protected readonly ultracodeCancel = computed(() => {
    const s = this.ctx.session();
    return !!s?.live && s.ultracode === 'unavailable' && capUltracode(s.control);
  });
  protected readonly ultracodeTitle = computed(() => {
    const s = this.ctx.session();
    if (!s?.ultracode) return 'Ultracode';
    // Only a transcript value's age ("5m ago") needs the ticking clock.
    return `Ultracode: ${ultracodeText(s, s.ultracodeAt ? this.clock.now() : Date.now())}`;
  });
  /** Ultracode isn't known yet in a live session running elsewhere (an
   * owned one reports it in a moment; an ended one says so on Info). */
  protected readonly ultracodeUnknown = computed(() => {
    const s = this.ctx.session();
    return !!s?.live && !s.owned && !s.ultracode;
  });
  protected readonly ultracodeUnknownTitle = computed(() => {
    const s = this.ctx.session();
    if (!s) return '';
    return ultracodeUnknownTitle(s, s.ultracodeBeforeAt ? this.clock.now() : Date.now());
  });
  /** Thinking can be switched here: a switch when its state is known, else
   * an explicit on/off choice (what an unknown default would toggle to is
   * anyone's guess). */
  protected readonly thinkingSwitch = computed(() => {
    const s = this.ctx.session();
    return !!s?.live && capThinking(s.control);
  });
  protected readonly thinkingOn = computed(() => {
    const t = this.ctx.session()?.thinking;
    return !!t && t.mode !== 'off';
  });
  protected readonly thinkingLocked = computed(
    () => this.busy() === 'thinking' || !this.ctx.canAct(),
  );
  protected readonly thinkingDefaultTitle =
    "Thinking: Claude Code's default for this model, which it doesn't report. Choose on or off.";
  /** The read-only thinking chip: the setting when known, else how much
   * thinking was used (nothing when neither). */
  protected readonly thinkingChipText = computed(() => {
    const s = this.ctx.session();
    return s ? thinkingLabel(s) : '';
  });
  protected readonly thinkingChipTitle = computed(() => {
    const s = this.ctx.session();
    return s ? thinkingTitle(s) : '';
  });
  protected readonly fastTitle = computed(() => {
    const s = this.ctx.session();
    return s ? `Fast mode: ${fastText(s)}` : '';
  });
  protected readonly models = computed(() => this.defaults.defaults()?.models ?? []);
  protected readonly efforts = computed(() => {
    const list = this.defaults.defaults()?.efforts;
    return list?.length ? list : FALLBACK_EFFORTS;
  });
  protected readonly agentsText = computed(() => {
    const a = this.ctx.session()?.agents;
    if (!a) return '';
    const parts = [];
    if (a.running) parts.push(`${a.running} agent${a.running === 1 ? '' : 's'}`);
    if (a.workflowsRunning) parts.push(`${a.workflowsRunning} workflow`);
    if (a.backgroundRunning) parts.push(`${a.backgroundRunning} bg`);
    return parts.join(' · ');
  });

  constructor() {
    void this.defaults.load();
    // `/model`, `/effort`, `/mode` from the composer open the picker here.
    effect(() => {
      const kind = this.ctx.openPicker();
      if (!kind || kind === 'rename') return;
      untracked(() => {
        this.ctx.openPicker.set(null);
        this.openPicker(kind);
      });
    });
  }

  private openPicker(kind: PickerKind): void {
    const el = (
      kind === 'model' ? this.modelSel() : kind === 'effort' ? this.effortSel() : this.modeSel()
    )?.nativeElement;
    if (!el) {
      this.explain(kind);
      return;
    }
    el.focus();
    try {
      (el as HTMLSelectElement & { showPicker?: () => void }).showPicker?.();
    } catch {
      this.notices.info(`Pick a ${kind === 'mode' ? 'permission mode' : kind} in the header.`);
    }
  }

  protected explain(kind: ExplainKind): void {
    const s = this.ctx.session();
    if (!s) return;
    const now = this.clock.now();
    if (kind === 'fast') {
      this.why.set({
        title: 'Fast mode',
        lines: [
          explainFast(s),
          ...(s.costEstimated ? ['The cost shown for this session is an estimate.'] : []),
        ],
        limits: false,
      });
      return;
    }
    const what = EXPLAIN_WHAT[kind];
    const lines =
      kind === 'ultracode'
        ? explainUltracode(s, now)
        : kind === 'thinking'
          ? explainThinking(s, now)
          : kind === 'effort' && s.effort
            ? [`Effort: ${effortText(s)}.`]
            : [];
    this.why.set({
      title:
        kind === 'ultracode' ? 'Ultracode' : kind === 'thinking' ? 'Thinking' : `Change ${what}`,
      lines: [...lines, whyNot(s, kind, what)].filter(Boolean),
      // The fixes, when there are any: an empty list would say "No limits:
      // everything works" under a setting that just said it can't change.
      limits: capLimits(s.control).length > 0,
      cancelUltracode: kind === 'ultracode' && this.ultracodeCancel(),
    });
  }

  protected async toggleUltracode(): Promise<void> {
    const s = this.ctx.session();
    if (s) await this.setUltracode(s.ultracode !== 'on');
  }

  /** Withdraw a request the model can't run (from the "why" sheet). */
  protected async cancelUltracode(): Promise<void> {
    this.why.set(null);
    await this.setUltracode(false);
  }

  private async setUltracode(enabled: boolean): Promise<void> {
    if (this.busy()) return;
    this.busy.set('ultracode');
    try {
      await this.ctx.setUltracode(enabled);
    } finally {
      this.busy.set(null);
    }
  }

  protected toggleThinking(): Promise<void> {
    return this.setThinking(!this.thinkingOn());
  }

  /** The choice offered while the thinking setting is unknown. */
  protected onThinking(e: Event): void {
    const sel = e.target as HTMLSelectElement;
    const v = sel.value;
    sel.selectedIndex = 0;
    if (v === 'on' || v === 'off') void this.setThinking(v === 'on');
  }

  private async setThinking(enabled: boolean): Promise<void> {
    if (this.busy()) return;
    this.busy.set('thinking');
    try {
      await this.ctx.setThinking(enabled);
    } finally {
      this.busy.set(null);
    }
  }

  protected onModel(e: Event): void {
    const sel = e.target as HTMLSelectElement;
    const v = sel.value;
    sel.selectedIndex = 0;
    if (v) void this.ctx.setModel(v);
  }

  protected onEffort(e: Event): void {
    const sel = e.target as HTMLSelectElement;
    const v = sel.value;
    sel.selectedIndex = 0;
    if (v) void this.ctx.setEffort(v);
  }

  protected onMode(e: Event): void {
    const sel = e.target as HTMLSelectElement;
    const v = sel.value as PermissionMode;
    sel.selectedIndex = 0;
    if (v) void this.ctx.setMode(v, modeLabel(v));
  }
}

/** Why a session setting can't be changed from here ('' when it can, or
 * when the explanation above already says why). */
function whyNot(s: SessionSummary, kind: Exclude<ExplainKind, 'fast'>, what: string): string {
  if (!s.live) return 'The session is not running.';
  if (kind === 'ultracode' && !s.owned) {
    // Where it is switched instead (it applies to that session only).
    const place = switchPlace(s.surface);
    if (place === 'in the terminal') {
      return 'Switch it in the terminal: /effort ultracode on (or off). It applies to that session only.';
    }
    if (place) return `Switch it ${place} itself; it applies to that session only.`;
  }
  if (s.control.channel === 'hooks') {
    return `The ${what} of a VS Code or terminal session can only be changed there.`;
  }
  if (kind === 'ultracode') {
    // "Unavailable" says why itself (and the sheet offers to withdraw it).
    if (s.ultracode === 'unavailable' || capUltracode(s.control)) return '';
    // A session that takes thinking switches takes ultracode ones too, as
    // far as the session goes: it is the model or workspace that can't.
    if (s.control.channel === 'owned' && capThinking(s.control)) {
      return "This session's model or workspace can't run ultracode: it needs a model that supports it and workflows switched on. Once it can run, it can be switched here.";
    }
  }
  if (kind === 'thinking' && capThinking(s.control)) return '';
  return `This session doesn't let Mother Claude change its ${what}.`;
}

const EXPLAIN_WHAT: Record<Exclude<ExplainKind, 'fast'>, string> = {
  model: 'model',
  effort: 'effort',
  mode: 'permission mode',
  rename: 'name',
  ultracode: 'ultracode',
  thinking: 'thinking setting',
};
