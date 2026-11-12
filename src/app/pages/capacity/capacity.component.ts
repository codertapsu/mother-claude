import { Component, DestroyRef, computed, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiError, ApiService, errorMessage } from '../../core/api.service';
import {
  CapacityView,
  QueueEntry,
  RemoteRoleView,
  RouteNote,
  bytesLabel,
  choiceLabel,
  normalizeCapacity,
  projectName,
  rateLimitText,
  schedulingWindows,
} from '../../core/capacity';
import { storageGet, storageSet, urlParam } from '../../core/platform';
import { RealtimeService } from '../../core/realtime.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { IconComponent } from '../../shared/icon.component';
import { PlanMeterComponent } from '../../shared/plan-meter.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';
import { formatUsd, sessionName } from '../../shared/util';
import { CapacityBudgetsComponent } from './capacity-budgets.component';
import { CapacitySettingsComponent } from './capacity-settings.component';
import { RoutingPolicyComponent } from './routing-policy.component';
import { RoutingReportComponent } from './routing-report.component';

type Tab = 'now' | 'budgets' | 'routing' | 'reports' | 'settings';

const TABS: { id: Tab; label: string }[] = [
  { id: 'now', label: 'Now' },
  { id: 'budgets', label: 'Budgets' },
  { id: 'routing', label: 'Routing' },
  { id: 'reports', label: 'Reports' },
  { id: 'settings', label: 'Settings' },
];
const TAB_KEY = 'mc_capacity_tab';
/** How often the page asks again while it is visible. */
export const POLL_MS = 5_000;

/** The section to open: `#/capacity?tab=budgets`, else the last one seen. */
function savedTab(): Tab {
  for (const t of [urlParam('tab'), storageGet(TAB_KEY)]) {
    if (TABS.some((x) => x.id === t)) return t as Tab;
  }
  return 'now';
}

/**
 * Capacity (docs/ROUTING.md): how many role sessions may run and why the
 * others wait, the signals behind it (memory, plan windows, rate limits,
 * your cap), budgets, the routing policy and what tasks cost. One code path
 * for the desktop and the phone: a phone reads everything and changes
 * budgets and caps; the routing policy is edited on the desktop.
 */
@Component({
  selector: 'mc-capacity',
  imports: [
    RouterLink,
    IconComponent,
    PlanMeterComponent,
    TimeAgoComponent,
    CapacityBudgetsComponent,
    CapacitySettingsComponent,
    RoutingPolicyComponent,
    RoutingReportComponent,
    TPipe,
  ],
  templateUrl: './capacity.component.html',
  styleUrl: './capacity.component.scss',
})
export class CapacityComponent {
  private api = inject(ApiService);
  private realtime = inject(RealtimeService);

  protected readonly tabs = TABS;
  protected readonly tab = signal<Tab>(savedTab());
  protected readonly view = signal<CapacityView | null>(null);
  protected readonly error = signal('');
  protected readonly unsupported = signal(false);
  protected readonly loading = signal(false);

  protected readonly waiting = computed(() => this.view()?.queue.filter((q) => !q.admitted) ?? []);
  protected readonly started = computed(() => this.view()?.queue.filter((q) => q.admitted) ?? []);
  protected readonly host = computed(() => this.view()?.host);
  /** Share of the memory in use, for the bar. */
  protected readonly memoryPct = computed(() => {
    const h = this.host();
    if (!h?.totalMemory) return 0;
    return Math.min(100, Math.round(((h.totalMemory - h.availableMemory) / h.totalMemory) * 100));
  });
  private readonly names = computed(
    () => new Map(this.realtime.sessions().map((s) => [s.id, sessionName(s)])),
  );

  protected readonly bytes = bytesLabel;
  protected readonly usd = formatUsd;
  protected readonly choice = choiceLabel;
  protected readonly project = projectName;
  protected readonly rateText = rateLimitText;
  protected readonly windows = schedulingWindows;

  constructor() {
    void this.load();
    const timer = setInterval(() => {
      if (typeof document === 'undefined' || document.visibilityState === 'visible') {
        void this.load(true);
      }
    }, POLL_MS);
    inject(DestroyRef).onDestroy(() => clearInterval(timer));
  }

  protected select(t: Tab): void {
    this.tab.set(t);
    storageSet(TAB_KEY, t);
  }

  /** A session's name, when the dashboard knows it. */
  protected nameOf(id: string): string | undefined {
    return this.names().get(id);
  }

  protected known(id: string): boolean {
    return this.names().has(id);
  }

  /** "P0", "P2 · lead". */
  protected priority(q: QueueEntry): string {
    return q.lead ? tr('lead') : `P${q.priority}`;
  }

  /** A remote role's badge: what its computer last said of it. */
  protected remoteState(r: RemoteRoleView): string {
    switch (r.state) {
      case 'running':
        return r.busy ? tr('working there') : tr('running there');
      case 'waiting':
        return tr('waiting there');
      default:
        return tr('stopped');
    }
  }

  /** "for reviewer (implement) in app", "for a task in app". */
  protected routeFor(r: RouteNote): string {
    const project = projectName(r.project);
    if (r.role) {
      return r.class
        ? tr('for {role} ({class}) in {project}', { role: r.role, class: r.class, project })
        : tr('for {role} in {project}', { role: r.role, project });
    }
    return r.class
      ? tr('for a task ({class}) in {project}', { class: r.class, project })
      : tr('for a task in {project}', { project });
  }

  protected applied(v: CapacityView): void {
    this.view.set(v);
  }

  async load(quiet = false): Promise<void> {
    if (!quiet) this.loading.set(true);
    try {
      const v = normalizeCapacity(await this.api.getCapacity());
      if (v) {
        this.view.set(v);
        this.error.set('');
      } else {
        this.error.set(tr('The server sent something this page could not read.'));
      }
      this.unsupported.set(false);
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) {
        this.unsupported.set(true);
      } else if (!quiet || !this.view()) {
        this.error.set(tr('Could not load capacity: {error}', { error: errorMessage(e) }));
      }
    } finally {
      this.loading.set(false);
    }
  }
}
