import { Component, computed, inject } from '@angular/core';
import { RouterLink } from '@angular/router';

import { AccountsService } from '../../core/accounts.service';
import { AuthService } from '../../core/auth.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { MachinesService } from '../../core/machines.service';
import { NavEntry, moreEntries } from '../../core/nav';
import { NavBadges } from '../../core/nav-badges';
import { NowService } from '../../core/now.service';
import { primaryWindows, windowPct } from '../../core/plan-usage';
import { PushService } from '../../core/push.service';
import { RealtimeService, connectionLabel } from '../../core/realtime.service';
import { ThemeService } from '../../core/theme.service';
import { usageLine } from '../../shared/accounts';
import { IconComponent } from '../../shared/icon.component';
import { PageHeadComponent } from '../../shared/page-head.component';

/**
 * The phone's "More" tab: what you look after — Computers, Accounts,
 * Settings — and Help, from the map (core/nav.ts), each with one line of
 * how it stands: which computers are online and what needs fixing, how
 * much of each account's plan is used, the notifications and the theme.
 * Only what this device already knows (and Machines' summary) is read.
 */
@Component({
  selector: 'mc-more',
  imports: [RouterLink, IconComponent, PageHeadComponent, TPipe],
  template: `
    <div class="page narrow">
      <mc-page-head [heading]="'More' | t" />
      <div class="card list">
        @for (e of entries(); track e.id) {
          <a class="list-row entry" [routerLink]="e.path" [attr.aria-label]="badges.label(e)">
            <span class="ico"><mc-icon [name]="e.icon" /></span>
            <span class="text">
              <span class="label">{{ e.label | t }}</span>
              @if (line(e.id); as l) {
                <span class="muted small state">{{ l }}</span>
              }
            </span>
            @if (badges.dot(e.badge)) {
              <span class="count-badge dot" aria-hidden="true"></span>
            }
            <mc-icon name="chevronRight" [size]="16" />
          </a>
        }
      </div>
      <p class="muted small conn">{{ 'Live updates: {state}' | t: { state: connLabel() } }}</p>
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
      .count-badge.dot {
        min-width: 0;
        width: 0.6rem;
        height: 0.6rem;
        padding: 0;
      }
      .conn {
        text-align: center;
        margin-top: 1rem;
      }
    `,
  ],
})
export class MoreComponent {
  private readonly auth = inject(AuthService);
  private readonly accounts = inject(AccountsService);
  private readonly machines = inject(MachinesService);
  private readonly realtime = inject(RealtimeService);
  private readonly push = inject(PushService);
  private readonly theme = inject(ThemeService);
  private readonly clock = inject(NowService);
  protected readonly badges = inject(NavBadges);

  protected readonly connLabel = computed(() => connectionLabel(this.realtime.connection()));
  protected readonly entries = computed<NavEntry[]>(() => moreEntries(this.auth.isDesktop()));

  /** Computers: which are online, and what needs fixing. */
  private readonly computersLine = computed(() => {
    const v = this.machines.view();
    if (!v) return '';
    if (!v.enabled && !v.peers.length) {
      return v.setupStep !== null
        ? tr('Setup: step {n} of 7', { n: v.setupStep })
        : tr('Not set up yet');
    }
    const online = v.peers.filter((p) => p.online);
    const n = online.length;
    const parts = [
      !v.peers.length
        ? tr('No computer paired yet')
        : n === 1
          ? tr('{name} online', { name: online[0].name })
          : n && n === v.peers.length
            ? tr('{n} computers online', { n })
            : n
              ? tr('{n} of {m} computers online', { n, m: v.peers.length })
              : tr('No computer online'),
    ];
    if (!v.enabled) parts.push(tr('Off'));
    const fix = v.health.state === 'blocker' ? v.health.blockers : 0;
    if (fix) parts.push(fix === 1 ? tr('1 to fix') : tr('{n} to fix', { n: fix }));
    return parts.join(' · ');
  });

  /** Accounts: how much of each one's plan is used. */
  private readonly accountsLine = computed(() => {
    const now = this.clock.now();
    const list = this.accounts.list();
    if (list.length > 1) {
      const used = list
        .map((a) => {
          const w = primaryWindows(this.accounts.usageOf(a.id)?.windows ?? [])[0];
          return this.accounts.usageOf(a.id)?.available && w
            ? `${a.name} ${windowPct(w.window, now)}%`
            : '';
        })
        .filter(Boolean);
      return used.length ? used.join(' · ') : tr('{n} accounts', { n: list.length });
    }
    return usageLine(this.realtime.planUsage(), now);
  });

  /** Settings: the notifications and the theme. */
  private readonly settingsLine = computed(() => {
    const parts: string[] = [];
    switch (this.push.state()) {
      case 'on':
        parts.push(tr('Notifications on'));
        break;
      case 'off':
        parts.push(tr('Notifications off'));
        break;
      case 'denied':
        parts.push(tr('Notifications blocked'));
        break;
      default:
        break;
    }
    const pref = this.theme.pref();
    parts.push(pref === 'dark' ? tr('Dark') : pref === 'light' ? tr('Light') : tr('System||theme'));
    return parts.join(' · ');
  });

  constructor() {
    // Computers' line and dot: Machines' summary (the desktop's shell keeps
    // it already; a phone reads it here).
    this.machines.start();
  }

  protected line(id: string): string {
    switch (id) {
      case 'computers':
        return this.computersLine();
      case 'accounts':
        return this.accountsLine();
      case 'settings':
        return this.settingsLine();
      case 'help':
        return tr('How to use every feature, step by step');
      default:
        return '';
    }
  }
}
