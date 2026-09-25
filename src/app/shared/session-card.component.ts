import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { RouterLink } from '@angular/router';

import { SessionSummary } from '../core/models';
import { ActivityLineComponent } from './activity-line.component';
import { ChannelBadgeComponent } from './channel-badge.component';
import { ChipComponent } from './chip.component';
import { ContextMeterComponent } from './context-meter.component';
import { CostComponent } from './cost.component';
import { IconComponent } from './icon.component';
import { PendingCardComponent } from './pending-card.component';
import { StatusPillComponent } from './status-pill.component';
import { RelativeTimePipe } from './time.pipes';
import { modeLabel, sessionName, shortPath } from './util';

/**
 * One session in a list: status, live activity line, chips (model, mode,
 * agents, todos, context, cost, branch, PR, surface/channel) and — when there
 * is exactly one pending request — a compact inline answer card. The whole
 * card links to the session; inner controls stay clickable.
 */
@Component({
  selector: 'mc-session-card',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    RouterLink,
    ActivityLineComponent,
    ChannelBadgeComponent,
    ChipComponent,
    ContextMeterComponent,
    CostComponent,
    IconComponent,
    PendingCardComponent,
    StatusPillComponent,
    RelativeTimePipe,
  ],
  host: { '[attr.data-status]': 'session().status' },
  template: `
    @let s = session();
    <div class="top">
      <mc-status-pill [status]="s.status" [live]="s.live" />
      <a class="name" [routerLink]="['/session', s.id]">{{ name() }}</a>
      <span class="when">{{ s.lastActivityAt | relTime }}</span>
    </div>

    @if (showActivity()) {
      <mc-activity-line [activity]="s.activity" />
    } @else if (s.lastMessage || s.lastPrompt) {
      <p class="last">{{ s.lastMessage || s.lastPrompt }}</p>
    }

    <div class="chips">
      <mc-channel-badge class="above" [surface]="s.surface" [control]="s.control" />
      @if (s.model) {
        <mc-chip>{{ s.model.label }}</mc-chip>
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

    @if (s.pending.length === 1) {
      <mc-pending-card class="above" [item]="s.pending[0]" [session]="s" [compact]="true" />
    } @else if (s.pending.length > 1) {
      <a class="more-pending above" [routerLink]="['/session', s.id]">
        <mc-icon name="inbox" [size]="15" /> {{ s.pending.length }} requests waiting
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
      /* The whole card opens the session; inner controls sit above it. */
      .name::after {
        content: '';
        position: absolute;
        inset: 0;
        border-radius: inherit;
      }
      .name:focus-visible {
        outline: none;
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
      .where {
        display: flex;
        align-items: center;
        gap: 0.35rem;
        font-size: 0.75rem;
        color: var(--faint);
        min-width: 0;
      }
      /* Positioned after the stretched link, so they paint above it — without
         a z-index, which would trap popups in a stacking context. */
      .above {
        position: relative;
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
  readonly session = input.required<SessionSummary>();

  protected readonly name = computed(() => sessionName(this.session()));
  protected readonly path = computed(() => shortPath(this.session().cwd));
  protected readonly mode = computed(() => modeLabel(this.session().permissionMode));
  protected readonly showActivity = computed(() => {
    const s = this.session();
    return s.live || s.status === 'working' || s.status === 'waiting';
  });
}
