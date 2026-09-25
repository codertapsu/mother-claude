import { Component, DestroyRef, computed, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiService, errorMessage } from '../../core/api.service';
import { PendingItem, SessionSummary } from '../../core/models';
import { RealtimeService } from '../../core/realtime.service';
import { IconComponent } from '../../shared/icon.component';
import { PendingCardComponent, PendingResolution } from '../../shared/pending-card.component';
import { StatusPillComponent } from '../../shared/status-pill.component';
import { sessionName, surfaceLabel } from '../../shared/util';

interface InboxGroup {
  sessionId: string;
  session?: SessionSummary;
  items: PendingItem[];
}

/**
 * Every pending request across all sessions — permissions, questions, plans —
 * each with the full answer controls. Grouped by session, oldest first.
 * Driven by the live store; `GET /api/inbox` fills in until the first
 * snapshot arrives.
 */
@Component({
  selector: 'mc-inbox',
  imports: [RouterLink, IconComponent, PendingCardComponent, StatusPillComponent],
  template: `
    <div class="page narrow">
      <div class="page-head">
        <h1>Inbox</h1>
        @if (items().length) {
          <span class="count-badge">{{ items().length }}</span>
        }
      </div>
      <p class="lede">Everything waiting for you, from every session.</p>

      @if (error()) {
        <p class="notice error">{{ error() }}</p>
      }

      @for (g of groups(); track g.sessionId) {
        <section class="group">
          <a class="ghead" [routerLink]="['/session', g.sessionId]">
            @if (g.session; as s) {
              <mc-status-pill [status]="s.status" [live]="s.live" />
              <span class="gname">{{ name(s) }}</span>
              <span class="muted small gsurf">{{ surface(s.surface) }}</span>
            } @else {
              <span class="gname mono">{{ g.sessionId.slice(0, 8) }}</span>
            }
            <mc-icon name="chevronRight" [size]="16" />
          </a>
          <div class="stack">
            @for (item of g.items; track item.id) {
              <mc-pending-card
                [item]="item"
                [session]="g.session"
                (resolved)="onResolved($event)"
              />
            }
          </div>
        </section>
      } @empty {
        @if (ready()) {
          <div class="card empty-state">
            <mc-icon name="check" [size]="28" />
            <strong>Nothing needs you</strong>
            Permission prompts, questions and plans from any session show up here.
          </div>
        } @else {
          <div class="card empty-state"><strong>Loading…</strong></div>
        }
      }
    </div>
  `,
  styles: [
    `
      .page-head .count-badge {
        font-size: 0.8rem;
      }
      .group {
        margin-top: 1.1rem;
      }
      .ghead {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        min-height: var(--tap);
        margin-bottom: 0.35rem;
        color: var(--text);
        text-decoration: none;
      }
      .gname {
        font-weight: 600;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        min-width: 0;
      }
      .gsurf {
        white-space: nowrap;
      }
      .ghead mc-icon {
        margin-left: auto;
        color: var(--faint);
      }
      .empty-state mc-icon {
        color: var(--green);
        display: block;
        margin: 0 auto 0.5rem;
        width: 28px;
      }
    `,
  ],
})
export class InboxComponent {
  private realtime = inject(RealtimeService);
  private api = inject(ApiService);

  /** Items answered here, hidden until the store drops them. */
  private readonly hidden = signal<ReadonlySet<string>>(new Set());
  /** REST fallback until the socket's first snapshot. */
  private readonly fetched = signal<PendingItem[] | null>(null);
  protected readonly error = signal('');

  protected readonly ready = computed(
    () => this.realtime.sessionsLoaded() || this.fetched() !== null,
  );

  protected readonly items = computed(() => {
    const source = this.realtime.sessionsLoaded()
      ? this.realtime.pendingItems()
      : (this.fetched() ?? []);
    const hidden = this.hidden();
    return source.filter((i) => !hidden.has(i.id));
  });

  protected readonly groups = computed<InboxGroup[]>(() => {
    const byId = new Map(this.realtime.sessions().map((s) => [s.id, s]));
    const groups = new Map<string, InboxGroup>();
    for (const item of [...this.items()].sort((a, b) => a.createdAt - b.createdAt)) {
      let g = groups.get(item.sessionId);
      if (!g) {
        g = { sessionId: item.sessionId, session: byId.get(item.sessionId), items: [] };
        groups.set(item.sessionId, g);
      }
      g.items.push(item);
    }
    return [...groups.values()];
  });

  protected readonly name = sessionName;
  protected readonly surface = surfaceLabel;

  constructor() {
    void this.load();
    const sub = this.realtime.refetch$.subscribe(() => {
      if (!this.realtime.sessionsLoaded()) void this.load();
      this.hidden.set(new Set());
    });
    inject(DestroyRef).onDestroy(() => sub.unsubscribe());
  }

  /** Let the card show its confirmation for a moment, then drop it. */
  protected onResolved(r: PendingResolution): void {
    setTimeout(() => this.hidden.update((prev) => new Set(prev).add(r.item.id)), 1200);
  }

  private async load(): Promise<void> {
    try {
      const inbox = await this.api.getInbox();
      this.fetched.set(inbox.items ?? []);
      this.error.set('');
    } catch (e) {
      if (!this.realtime.sessionsLoaded()) this.error.set(errorMessage(e));
    }
  }
}
