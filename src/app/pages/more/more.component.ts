import { Component, computed, inject } from '@angular/core';
import { RouterLink } from '@angular/router';

import { AuthService } from '../../core/auth.service';
import { RealtimeService, connectionLabel } from '../../core/realtime.service';
import { IconComponent, IconName } from '../../shared/icon.component';

interface Entry {
  path: string;
  icon: IconName;
  label: string;
  hint: string;
  desktopOnly?: boolean;
}

const ENTRIES: Entry[] = [
  { path: '/new', icon: 'plus', label: 'New session', hint: 'Start an owned Claude Code session' },
  { path: '/teams', icon: 'layers', label: 'Teams', hint: 'A lead and role sessions on a board' },
  {
    path: '/accounts',
    icon: 'user',
    label: 'Accounts',
    hint: 'Several Claude accounts, sign-in, moving conversations',
  },
  {
    path: '/backup',
    icon: 'cloud',
    label: 'Backup & sync',
    hint: 'Back up, restore and carry conversations to another computer',
  },
  {
    path: '/remote',
    icon: 'phone',
    label: 'Remote',
    hint: 'Pair devices, internet access, foreign sessions',
    desktopOnly: true,
  },
  {
    path: '/settings',
    icon: 'settings',
    label: 'Settings',
    hint: 'Notifications, theme, plan usage, connection',
  },
  { path: '/api', icon: 'code', label: 'HTTP API', hint: 'Drive Claude from scripts and tools' },
  {
    path: '/capacity',
    icon: 'gauge',
    label: 'Capacity',
    hint: 'Role sessions, why they wait, budgets, routing and costs',
  },
  {
    path: '/browser',
    icon: 'globe',
    label: 'Browser',
    hint: 'Sessions in your browser: site modes, approvals',
  },
  { path: '/services', icon: 'activity', label: 'Services', hint: 'Daemon and MCP servers' },
  {
    path: '/help',
    icon: 'question',
    label: 'Help',
    hint: 'How to use every feature, step by step',
  },
];

/** The phone's "More" tab: everything that isn't a primary tab. */
@Component({
  selector: 'mc-more',
  imports: [RouterLink, IconComponent],
  template: `
    <div class="page narrow">
      <div class="card list">
        @for (e of entries(); track e.path) {
          <a class="list-row entry" [routerLink]="e.path">
            <span class="ico"><mc-icon [name]="e.icon" /></span>
            <span class="text">
              <span class="label">{{ e.label }}</span>
              <span class="muted small">{{ e.hint }}</span>
            </span>
            <mc-icon name="chevronRight" [size]="16" />
          </a>
        }
      </div>
      <p class="muted small conn">Live updates: {{ connLabel() }}</p>
    </div>
  `,
  styles: [
    `
      .list {
        padding-top: 0;
        padding-bottom: 0;
      }
      .entry {
        color: var(--text);
        text-decoration: none;
      }
      .ico {
        width: 2.2rem;
        height: 2.2rem;
        border-radius: 0.6rem;
        background: var(--accent-soft);
        color: var(--accent-text);
        display: inline-flex;
        align-items: center;
        justify-content: center;
        flex: 0 0 auto;
      }
      .text {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
      }
      .label {
        font-weight: 500;
      }
      .entry > mc-icon {
        color: var(--faint);
      }
      .conn {
        text-align: center;
        margin-top: 1rem;
      }
    `,
  ],
})
export class MoreComponent {
  private auth = inject(AuthService);
  private readonly connection = inject(RealtimeService).connection;
  protected readonly connLabel = computed(() => connectionLabel(this.connection()));
  protected readonly entries = computed(() =>
    ENTRIES.filter((e) => !e.desktopOnly || this.auth.isDesktop()),
  );
}
