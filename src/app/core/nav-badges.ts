import { Injectable, computed, inject } from '@angular/core';

import { AttentionService } from './attention.service';
import { tr } from './i18n/i18n';
import { MachinesService } from './machines.service';
import { NavBadge } from './nav';
import { RealtimeService } from './realtime.service';
import { SyncService } from './sync.service';

/**
 * What the map's badges show right now (core/nav.ts): the requests waiting
 * (and a dot when only something to fix waits), the agents running, your
 * computers' health, the projects kept in sync. The sidebar, the phone's
 * bar, More and a place's tabs all ask here, so a badge reads the same
 * everywhere.
 */
@Injectable({ providedIn: 'root' })
export class NavBadges {
  private readonly attention = inject(AttentionService);
  private readonly realtime = inject(RealtimeService);
  private readonly machines = inject(MachinesService);
  private readonly sync = inject(SyncService);

  /** Running agents across every session (the Agents badge). */
  readonly agentsRunning = computed(() =>
    this.realtime
      .sessions()
      .reduce((n, s) => n + (s.agents?.running ?? 0) + (s.agents?.backgroundRunning ?? 0), 0),
  );

  /** Projects kept in sync with another computer, or asked to be (the
   * Code sync tab's count; known while Computers is in use). */
  readonly synced = computed(
    () => this.sync.pairs().filter((p) => p.status.state !== 'off').length,
  );

  /** The number a badge shows (0: no number). */
  count(badge: NavBadge | undefined): number {
    switch (badge) {
      case 'pending':
        return this.attention.pendingCount();
      case 'agents':
        return this.agentsRunning();
      case 'sync':
        return this.synced();
      default:
        return 0;
    }
  }

  /** The badge is a dot: something to look at, with no number — on the
   * Inbox, something to fix while no request waits. */
  dot(badge: NavBadge | undefined): boolean {
    if (badge === 'pending') {
      return !this.attention.pendingCount() && this.attention.toFixCount() > 0;
    }
    return badge === 'machines' && this.machines.alarm();
  }

  /** A link's name for screen readers, with what its badge says (null:
   * its text says it all). `label` is English. */
  label(entry: { label: string; badge?: NavBadge }): string | null {
    const label = tr(entry.label);
    const n = this.count(entry.badge);
    if (entry.badge === 'pending') {
      const fix = this.attention.toFixCount();
      if (n && fix) return tr('{label}, {n} waiting, {m} to fix', { label, n, m: fix });
      if (n) return tr('{label}, {n} waiting', { label, n });
      if (fix) return tr('{label}, {m} to fix', { label, m: fix });
      return null;
    }
    if (entry.badge === 'agents' && n) return tr('{label}, {n} running', { label, n });
    if (entry.badge === 'sync' && n) {
      return n === 1
        ? tr('{label}, 1 project', { label })
        : tr('{label}, {n} projects', { label, n });
    }
    if (this.dot(entry.badge)) return tr('{label}, needs attention', { label });
    return null;
  }
}
