import { Component, computed, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';

import { ApiService, errorMessage } from '../../core/api.service';
import { SessionSummary } from '../../core/models';
import { storageGet, storageSet } from '../../core/platform';
import { RealtimeService } from '../../core/realtime.service';
import { FleetSummaryComponent } from '../../shared/fleet-summary.component';
import { IconComponent } from '../../shared/icon.component';
import { LimitOfferComponent } from '../../shared/limit-offer.component';
import { SessionCardComponent } from '../../shared/session-card.component';
import { thinkingChip } from '../../shared/session-settings';
import { effortLabel, effortShortLabel, surfaceLabel } from '../../shared/util';

type Filter = 'live' | 'needs' | 'all';

interface Group {
  key: string;
  title: string;
  items: SessionSummary[];
}

const FILTER_KEY = 'mc_sessions_filter';
const HISTORY_PAGE = 25;

function needsYou(s: SessionSummary): boolean {
  return s.status === 'waiting' || (s.pending?.length ?? 0) > 0;
}

function byRecent(a: SessionSummary, b: SessionSummary): number {
  return (b.lastActivityAt ?? b.startedAt ?? 0) - (a.lastActivityAt ?? a.startedAt ?? 0);
}

/**
 * Every session in the active scope (live, or active in the last 24 h),
 * searchable and filterable, grouped by what they need; ended sessions page
 * in on demand. A card with one simple pending request can be answered
 * right here.
 */
@Component({
  selector: 'mc-sessions',
  imports: [
    RouterLink,
    FormsModule,
    FleetSummaryComponent,
    IconComponent,
    LimitOfferComponent,
    SessionCardComponent,
  ],
  templateUrl: './sessions.component.html',
  styleUrl: './sessions.component.scss',
})
export class SessionsComponent {
  private realtime = inject(RealtimeService);
  private api = inject(ApiService);

  protected readonly loaded = this.realtime.sessionsLoaded;
  protected readonly connection = this.realtime.connection;
  protected readonly all = this.realtime.sessions;

  protected readonly query = signal('');
  protected readonly filter = signal<Filter>(readFilter());
  protected readonly surface = signal('');
  protected readonly project = signal('');

  protected readonly history = signal<SessionSummary[]>([]);
  protected readonly historyOpen = signal(false);
  protected readonly historyLoading = signal(false);
  protected readonly historyError = signal('');
  private historyCursor: string | undefined;
  protected readonly historyDone = signal(false);

  protected readonly liveCount = computed(() => this.all().filter((s) => s.live).length);
  protected readonly needsCount = computed(() => this.all().filter(needsYou).length);

  protected readonly surfaces = computed(() =>
    [...new Set([...this.all(), ...this.history()].map((s) => s.surface))].sort(),
  );
  protected readonly projects = computed(() =>
    [...new Set([...this.all(), ...this.history()].map((s) => s.projectName).filter(Boolean))].sort(
      (a, b) => a.localeCompare(b),
    ),
  );

  private readonly matches = computed(() => {
    const q = this.query().trim().toLowerCase();
    const surface = this.surface();
    const project = this.project();
    return (s: SessionSummary): boolean => {
      if (surface && s.surface !== surface) return false;
      if (project && s.projectName !== project) return false;
      return !q || sessionMatches(s, q);
    };
  });

  protected readonly visible = computed(() => {
    const match = this.matches();
    const f = this.filter();
    return this.all().filter(
      (s) => match(s) && (f === 'all' || (f === 'live' ? s.live : needsYou(s))),
    );
  });

  protected readonly groups = computed<Group[]>(() => {
    const list = this.visible();
    const needs = list.filter(needsYou);
    const rest = list.filter((s) => !needsYou(s));
    const groups: Group[] = [
      {
        key: 'needs',
        title: 'Needs you',
        items: needs.sort((a, b) => oldestPending(a) - oldestPending(b)),
      },
      {
        key: 'working',
        title: 'Working',
        items: rest.filter((s) => s.status === 'working' || s.status === 'starting').sort(byRecent),
      },
      {
        key: 'idle',
        title: 'Idle',
        items: rest.filter((s) => s.status === 'idle').sort(byRecent),
      },
      {
        key: 'ended',
        title: 'Ended recently',
        items: rest.filter((s) => ['ended', 'failed', 'stopped'].includes(s.status)).sort(byRecent),
      },
    ];
    // Unknown statuses from a newer server still show up.
    const known = new Set(groups.flatMap((g) => g.items.map((s) => s.id)));
    const other = list.filter((s) => !known.has(s.id));
    if (other.length) groups.push({ key: 'other', title: 'Other', items: other.sort(byRecent) });
    return groups.filter((g) => g.items.length);
  });

  protected readonly historyVisible = computed(() => {
    const inStore = new Set(this.all().map((s) => s.id));
    const match = this.matches();
    return this.history().filter((s) => !inStore.has(s.id) && match(s));
  });

  protected readonly surfaceLabel = surfaceLabel;

  setFilter(f: Filter): void {
    this.filter.set(f);
    storageSet(FILTER_KEY, f);
  }

  clearFilters(): void {
    this.query.set('');
    this.surface.set('');
    this.project.set('');
    this.setFilter('all');
  }

  async openHistory(): Promise<void> {
    this.historyOpen.set(true);
    if (!this.history().length) await this.loadMoreHistory();
  }

  async loadMoreHistory(): Promise<void> {
    if (this.historyLoading() || this.historyDone()) return;
    this.historyLoading.set(true);
    this.historyError.set('');
    try {
      const page = await this.api.listSessions({
        scope: 'all',
        limit: HISTORY_PAGE,
        before: this.historyCursor,
      });
      const seen = new Set(this.history().map((s) => s.id));
      const fresh = (page.sessions ?? []).filter((s) => !seen.has(s.id));
      this.history.update((prev) => [...prev, ...fresh]);
      this.historyCursor = page.nextBefore;
      if (!page.nextBefore) this.historyDone.set(true);
    } catch (e) {
      this.historyError.set(errorMessage(e));
    } finally {
      this.historyLoading.set(false);
    }
  }
}

function oldestPending(s: SessionSummary): number {
  return Math.min(...(s.pending ?? []).map((p) => p.createdAt), s.lastActivityAt ?? Infinity);
}

function readFilter(): Filter {
  const v = storageGet(FILTER_KEY);
  return v === 'live' || v === 'needs' ? v : 'all';
}

/** The search box: `q` (lower case) appears in anything the card shows
 * about the session, or in its id, folder or last prompt. */
export function sessionMatches(s: SessionSummary, q: string): boolean {
  return [
    s.title,
    s.name,
    s.projectName,
    s.cwd,
    s.gitBranch,
    s.lastPrompt,
    s.model?.label,
    s.effort,
    effortLabel(s.effort),
    // What the card's chips say ("X-high", "Thinking off").
    effortShortLabel(s.effort),
    thinkingChip(s)?.text,
    s.ultracode === 'on' ? 'ultracode' : '',
    s.fast ? 'fast' : '',
    s.id,
  ]
    .filter(Boolean)
    .some((v) => String(v).toLowerCase().includes(q));
}
