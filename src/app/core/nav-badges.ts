import { Injectable, computed, inject } from '@angular/core';

import { AttentionService } from './attention.service';
import { tr } from './i18n/i18n';
import { MachinesService } from './machines.service';
import { NavBadge } from './nav';
import { RealtimeService } from './realtime.service';

/**
 * What the map's badges show right now (core/nav.ts): the requests waiting,
 * the agents running, Machines' health. The sidebar, the phone's bar, More
 * and a place's tabs all ask here, so a badge reads the same everywhere.
 */
@Injectable({ providedIn: 'root' })
export class NavBadges {
  private readonly attention = inject(AttentionService);
  private readonly realtime = inject(RealtimeService);
  private readonly machines = inject(MachinesService);

  /** Running agents across every session (the Agents badge). */
  readonly agentsRunning = computed(() =>
    this.realtime
      .sessions()
      .reduce((n, s) => n + (s.agents?.running ?? 0) + (s.agents?.backgroundRunning ?? 0), 0),
  );

  /** The number a badge shows (0: no number). */
  count(badge: NavBadge | undefined): number {
    switch (badge) {
      case 'pending':
        return this.attention.pendingCount();
      case 'agents':
        return this.agentsRunning();
      default:
        return 0;
    }
  }

  /** The badge is a dot: something to look at, with no number. */
  dot(badge: NavBadge | undefined): boolean {
    return badge === 'machines' && this.machines.alarm();
  }

  /** A link's name for screen readers, with what its badge says (null:
   * its text says it all). `label` is English. */
  label(entry: { label: string; badge?: NavBadge }): string | null {
    const label = tr(entry.label);
    const n = this.count(entry.badge);
    if (entry.badge === 'pending' && n) return tr('{label}, {n} waiting', { label, n });
    if (entry.badge === 'agents' && n) return tr('{label}, {n} running', { label, n });
    if (this.dot(entry.badge)) return tr('{label}, needs attention', { label });
    return null;
  }
}
