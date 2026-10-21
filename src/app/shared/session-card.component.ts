import { ChangeDetectionStrategy, Component, computed, inject, input, output } from '@angular/core';
import { RouterLink } from '@angular/router';

import { PendingRef, SessionSummary } from '../core/models';
import { NowService } from '../core/now.service';
import { ViewportService } from '../core/viewport.service';
import { AccountChipComponent } from './account-chip.component';
import { ActivityLineComponent } from './activity-line.component';
import { ChannelBadgeComponent } from './channel-badge.component';
import { ChipComponent } from './chip.component';
import { ContextMeterComponent } from './context-meter.component';
import { CostComponent } from './cost.component';
import { IconComponent, IconName } from './icon.component';
import { PendingCardComponent } from './pending-card.component';
import { effortText, fastText, thinkingChip, ultracodeText } from './session-settings';
import { StatusPillComponent } from './status-pill.component';
import { TimeAgoComponent } from './time-ago.component';
import { Tone, effortShortLabel, modeLabel, sessionName, shortPath } from './util';

/** One part of the packed settings chip (the smallest screens). */
interface PackedPart {
  kind: 'effort' | 'ultracode' | 'fast';
  text: string;
  /** Said before `text` to screen readers only. */
  sr: string;
}

/**
 * One session in a list: status, live activity line, chips (surface/channel,
 * the Claude account while there are several, model, effort, ultracode,
 * thinking, fast, mode, agents, todos, context, cost, branch, PR) and —
 * when there is exactly one pending request — a
 * compact card with quick actions for the simple cases (else "Review").
 * Only the title is a link, so the buttons inside stay valid, separately
 * focusable controls. The setting chips stay short (how a value is known is
 * in their tooltip and screen-reader text), and appear only when there is
 * something to say: ultracode when on (not when off, unknown or unavailable:
 * the session page says which), thinking when off or used. On the smallest
 * screens effort, ultracode and fast share one chip and thinking shows only
 * its figure, so the settings add at most one row of chips. While the list
 * selects (Sessions › Select) a checkbox leads the top row.
 */
@Component({
  selector: 'mc-session-card',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    RouterLink,
    AccountChipComponent,
    ActivityLineComponent,
    ChannelBadgeComponent,
    ChipComponent,
    ContextMeterComponent,
    CostComponent,
    IconComponent,
    PendingCardComponent,
    StatusPillComponent,
    TimeAgoComponent,
  ],
  host: {
    '[attr.data-status]': 'session().status',
    '[attr.data-selected]': "selectable() && selected() ? '' : null",
  },
  template: `
    @let s = session();
    <div class="top">
      @if (selectable()) {
        <label class="pick">
          <input
            type="checkbox"
            [checked]="selected()"
            [attr.aria-label]="'Select ' + name()"
            (change)="toggled.emit()"
          />
        </label>
      }
      <mc-status-pill [status]="s.status" [live]="s.live" />
      <a class="name" [routerLink]="['/session', s.id]">{{ name() }}</a>
      <mc-time class="when" [ms]="s.lastActivityAt" />
    </div>

    @if (showActivity()) {
      <mc-activity-line [activity]="s.activity" />
    } @else if (s.lastMessage || s.lastPrompt) {
      <p class="last">{{ s.lastMessage || s.lastPrompt }}</p>
    }

    <div class="chips">
      <mc-channel-badge [surface]="s.surface" [control]="s.control" [session]="s" />
      @if (s.account; as a) {
        <mc-account-chip [account]="a" />
      }
      @if (s.model) {
        <mc-chip>{{ s.model.label }}</mc-chip>
      }
      @if (viewport.narrow()) {
        <!-- The smallest screens: effort, ultracode and fast share a chip and
             thinking shows only its figure, so they add at most one row. -->
        @if (packed(); as p) {
          <mc-chip class="packed" [icon]="p.icon" [tone]="p.tone" [attr.title]="p.title">
            @for (part of p.parts; track part.kind; let first = $first) {
              @if (!first) {
                <span class="sep" aria-hidden="true"> · </span>
              }
              <span [class]="part.kind"
                ><span class="visually-hidden">{{ part.sr }}</span
                >{{ part.text }}</span
              >
            }
          </mc-chip>
        }
        @if (thinking(); as t) {
          <mc-chip icon="lightbulb" [attr.title]="t.title"
            ><span class="visually-hidden">Thinking </span>{{ t.short }}</mc-chip
          >
        }
      } @else {
        @if (s.effort) {
          <mc-chip icon="gauge" [attr.title]="'Effort: ' + effort()"
            ><span class="visually-hidden">Effort </span>{{ effortShort() }}</mc-chip
          >
        }
        @if (s.ultracode === 'on') {
          <mc-chip icon="workflow" tone="accent" [attr.title]="'Ultracode: ' + ultracode()"
            >Ultracode</mc-chip
          >
        }
        @if (thinking(); as t) {
          <mc-chip icon="lightbulb" [attr.title]="t.title">{{ t.text }}</mc-chip>
        }
        @if (s.fast) {
          <mc-chip icon="zap" tone="blue" [attr.title]="fast()">Fast</mc-chip>
        }
      }
      @if (s.permissionMode && s.permissionMode !== 'default') {
        <mc-chip [tone]="s.permissionMode === 'bypassPermissions' ? 'red' : 'muted'">{{
          mode()
        }}</mc-chip>
      }
      @if (s.agents.running) {
        <mc-chip icon="agents" tone="green">{{ s.agents.running }} running</mc-chip>
      }
      @if (s.todos; as t) {
        @if (t.total) {
          <mc-chip icon="check" [tone]="t.done === t.total ? 'green' : 'muted'"
            >{{ t.done }}/{{ t.total }}</mc-chip
          >
        }
      }
      @if (s.context) {
        <mc-context-meter [context]="s.context" />
      }
      @if (s.costUsd !== undefined && s.costUsd !== null) {
        <mc-chip><mc-cost [usd]="s.costUsd" [final]="!!s.costFinal" /></mc-chip>
      }
      @if (s.gitBranch) {
        <mc-chip icon="branch">{{ s.gitBranch }}</mc-chip>
      }
      @if (s.prLink) {
        <mc-chip tone="blue">PR #{{ s.prLink.number }}</mc-chip>
      }
      @if (s.queued) {
        <mc-chip tone="blue">{{ s.queued }} queued</mc-chip>
      }
      @if (s.lastError) {
        <mc-chip tone="red" icon="alert">{{ s.lastError.kind }}</mc-chip>
      }
    </div>

    <div class="where">
      <mc-icon name="folder" [size]="13" />
      <span class="ellipsis">{{ s.projectName }} · {{ path() }}</span>
    </div>

    @if (single(); as p) {
      <mc-pending-card [item]="p" [session]="s" [compact]="true" />
    } @else if (s.pending.length > 1) {
      <a class="more-pending" [routerLink]="['/session', s.id]">
        <mc-icon name="inbox" [size]="15" /> {{ s.pending.length }} requests waiting · Review
        <mc-icon name="chevronRight" [size]="15" />
      </a>
    }
  `,
  styles: [
    `
      :host {
        position: relative;
        display: flex;
        flex-direction: column;
        gap: 0.45rem;
        min-width: 0;
        background: var(--panel);
        border: 1px solid var(--border);
        border-radius: var(--radius);
        padding: 0.75rem 0.9rem;
        transition: border-color 0.12s ease;
      }
      :host(:hover) {
        border-color: var(--border-strong);
      }
      :host(:focus-within) {
        border-color: var(--accent);
      }
      :host([data-status='waiting']) {
        border-color: var(--amber-border);
      }
      :host([data-selected]) {
        border-color: var(--accent);
        background: color-mix(in srgb, var(--accent-soft) 45%, var(--panel));
      }
      /* A tap target of the full size, without growing the row. */
      .pick {
        flex: 0 0 auto;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        min-width: var(--tap);
        min-height: var(--tap);
        margin: -0.75rem -0.55rem -0.75rem -0.85rem;
        cursor: pointer;
      }
      .pick input {
        width: 1.05rem;
        height: 1.05rem;
      }
      .top {
        display: flex;
        align-items: center;
        gap: 0.55rem;
        min-width: 0;
      }
      .name {
        flex: 1;
        min-width: 0;
        font-weight: 600;
        color: var(--text);
        text-decoration: none;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .name:hover {
        color: var(--accent-text);
      }
      .when {
        font-size: 0.75rem;
        color: var(--faint);
        white-space: nowrap;
      }
      .last {
        margin: 0;
        font-size: 0.85rem;
        color: var(--muted);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .chips {
        display: flex;
        flex-wrap: wrap;
        gap: 0.3rem;
        align-items: center;
      }
      .packed .ultracode {
        color: var(--accent-text);
      }
      .packed .fast {
        color: var(--blue);
      }
      .where {
        display: flex;
        align-items: center;
        gap: 0.35rem;
        font-size: 0.75rem;
        color: var(--faint);
        min-width: 0;
      }
      .more-pending {
        display: inline-flex;
        align-items: center;
        gap: 0.35rem;
        align-self: flex-start;
        min-height: 36px;
        font-size: 0.85rem;
        font-weight: 500;
        color: var(--amber);
        text-decoration: none;
      }
    `,
  ],
})
export class SessionCardComponent {
  private clock = inject(NowService);
  protected readonly viewport = inject(ViewportService);
  readonly session = input.required<SessionSummary>();
  /** Show a checkbox (Sessions › Select). */
  readonly selectable = input(false);
  readonly selected = input(false);
  /** The checkbox was ticked or cleared. */
  readonly toggled = output<void>();

  protected readonly name = computed(() => sessionName(this.session()));
  protected readonly effort = computed(() => effortText(this.session()));
  protected readonly effortShort = computed(() => effortShortLabel(this.session().effort));
  protected readonly ultracode = computed(() => {
    const s = this.session();
    if (s.ultracode !== 'on') return '';
    // Only a transcript marker's age ("5m ago") needs the ticking clock.
    return ultracodeText(s, s.ultracodeAt ? this.clock.now() : Date.now());
  });
  protected readonly thinking = computed(() => thinkingChip(this.session()));
  protected readonly fast = computed(() => `Fast mode: ${fastText(this.session())}`);
  /** Effort, ultracode and fast mode as one chip ("Max · Ultracode · Fast"),
   * its title saying how each is known; null when none applies. */
  protected readonly packed = computed(() => {
    const s = this.session();
    const parts: PackedPart[] = [];
    const titles: string[] = [];
    if (s.effort) {
      parts.push({ kind: 'effort', text: this.effortShort(), sr: 'Effort ' });
      titles.push(`Effort: ${this.effort()}`);
    }
    if (s.ultracode === 'on') {
      parts.push({ kind: 'ultracode', text: 'Ultracode', sr: '' });
      titles.push(`Ultracode: ${this.ultracode()}`);
    }
    if (s.fast) {
      parts.push({ kind: 'fast', text: 'Fast', sr: '' });
      titles.push(this.fast());
    }
    if (!parts.length) return null;
    const icon: IconName = s.effort ? 'gauge' : s.ultracode === 'on' ? 'workflow' : 'zap';
    const only = parts.length === 1 ? parts[0].kind : '';
    const tone: Tone = only === 'ultracode' ? 'accent' : only === 'fast' ? 'blue' : 'muted';
    return { parts, icon, tone, title: titles.join(' · ') };
  });
  protected readonly single = computed<PendingRef | null>(() => {
    const s = this.session();
    return s.pending?.length === 1 ? { ...s.pending[0], sessionId: s.id } : null;
  });
  protected readonly path = computed(() => shortPath(this.session().cwd));
  protected readonly mode = computed(() => modeLabel(this.session().permissionMode));
  protected readonly showActivity = computed(() => {
    const s = this.session();
    return s.live || s.status === 'working' || s.status === 'waiting';
  });
}
