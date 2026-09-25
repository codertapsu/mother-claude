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
import { PermissionMode } from '../../core/models';
import { CapLimitsComponent } from '../../shared/cap-limits.component';
import { ChannelBadgeComponent } from '../../shared/channel-badge.component';
import { ChipComponent } from '../../shared/chip.component';
import { ContextMeterComponent } from '../../shared/context-meter.component';
import { CostComponent } from '../../shared/cost.component';
import { IconComponent } from '../../shared/icon.component';
import { SheetComponent } from '../../shared/sheet.component';
import { PERMISSION_MODES, effortLabel, modeLabel } from '../../shared/util';
import { PickerKind, SessionContext } from './session-context';

interface Why {
  title: string;
  text: string;
}

/**
 * The session's second header row (it scrolls horizontally and scrolls away
 * with the page): where it runs and how it's reached, then model, mode and
 * effort — native pickers when the session can change them, otherwise a
 * disabled chip whose tap explains why (with the fixes) — context, cost,
 * branch, PR, running agents, todos, queue and the last error.
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
          <label class="pick" title="Effort">
            <mc-icon name="activity" [size]="13" />
            <span class="pv">{{ s.effort ? effort(s.effort) : 'Effort' }}</span>
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
          <button class="pick off" type="button" (click)="explain('effort')">
            <mc-icon name="activity" [size]="13" />
            <span class="pv">{{ effort(s.effort) }}</span>
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
        <p>{{ why()?.text }}</p>
        <mc-cap-limits [session]="s" (done)="why.set(null)" />
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

  private readonly modelSel = viewChild<ElementRef<HTMLSelectElement>>('modelSel');
  private readonly modeSel = viewChild<ElementRef<HTMLSelectElement>>('modeSel');
  private readonly effortSel = viewChild<ElementRef<HTMLSelectElement>>('effortSel');

  protected readonly why = signal<Why | null>(null);
  protected readonly modes = PERMISSION_MODES;
  protected readonly mode = modeLabel;
  protected readonly effort = effortLabel;
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

  protected explain(kind: PickerKind): void {
    const s = this.ctx.session();
    const what = kind === 'model' ? 'model' : kind === 'effort' ? 'effort' : 'permission mode';
    const reason = !s?.live
      ? 'The session is not running.'
      : s.control.channel === 'hooks'
        ? `The ${what} of a VS Code or terminal session can only be changed there.`
        : `This session doesn't let Mother Claude change its ${what}.`;
    this.why.set({ title: `Change ${what}`, text: reason });
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
