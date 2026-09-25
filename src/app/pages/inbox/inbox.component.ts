import {
  Component,
  DestroyRef,
  ElementRef,
  computed,
  inject,
  signal,
  viewChildren,
} from '@angular/core';
import { RouterLink } from '@angular/router';

import { AuthService } from '../../core/auth.service';
import { PendingRef, SessionSummary } from '../../core/models';
import { NoticeService } from '../../core/notice.service';
import { PendingService } from '../../core/pending.service';
import { RealtimeService } from '../../core/realtime.service';
import { IconComponent } from '../../shared/icon.component';
import { Lingering, lingerList } from '../../shared/linger';
import { PendingCardComponent } from '../../shared/pending-card.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';

interface Entry {
  ref: PendingRef;
  gone: boolean;
  session?: SessionSummary;
}

/**
 * Everything waiting for a human, across every session, in two sections:
 * "Needs you" (answerable here, oldest first, with how long it has waited)
 * and "Waiting at the desk" (only answerable where the session runs, with
 * the reason and what would make it answerable here).
 *
 * Full items arrive through `subInbox` (and `GET /inbox` on first load).
 * After an answer the next card takes focus; on a keyboard J/K move between
 * cards and A/D/F answer the focused one.
 */
@Component({
  selector: 'mc-inbox',
  imports: [RouterLink, IconComponent, PendingCardComponent, TimeAgoComponent],
  template: `
    <div class="page narrow">
      <div class="page-head">
        <h1>Inbox</h1>
      </div>

      @if (!ready()) {
        <div class="skeleton" aria-label="Loading"><span></span><span></span></div>
      } @else if (!needs().length && !desk().length) {
        <div class="card empty-state">
          <mc-icon name="check" [size]="28" />
          <strong>Nothing needs you</strong>
          Permission prompts, questions and plans from every session show up here.
          @if (foreignHint()) {
            <p class="muted small hint">
              Approvals for VS Code and terminal sessions appear here once foreign-session control
              is installed
              @if (isDesktop()) {
                (<a routerLink="/remote" fragment="foreign">Remote › Foreign sessions</a>).
              } @else {
                (on the Mac: Remote › Foreign sessions).
              }
            </p>
          }
        </div>
      }

      @if (needs().length) {
        <section aria-labelledby="inbox-needs">
          <h2 id="inbox-needs">
            Needs you <span class="count-badge">{{ needs().length }}</span>
          </h2>
          <p class="muted small keys">
            Tip: on a keyboard, J/K move between requests; A allows, D denies, F adds feedback.
          </p>
          <div class="stack">
            @for (e of needs(); track e.ref.id; let i = $index) {
              <div class="entry">
                <p class="waited">
                  <mc-icon name="clock" [size]="12" />
                  <mc-time [ms]="e.ref.createdAt" mode="elapsed" prefix="waiting " />
                </p>
                <mc-pending-card
                  [item]="e.ref"
                  [session]="e.session"
                  [showSession]="true"
                  [gone]="e.gone"
                  bodyMax="min(55vh, 26rem)"
                  (resolved)="focusCard(i + 1)"
                  (move)="focusCard(i + $event)"
                />
              </div>
            }
          </div>
        </section>
      }

      @if (desk().length) {
        <section aria-labelledby="inbox-desk">
          <h2 id="inbox-desk">
            Waiting at the desk <span class="count-badge muted-badge">{{ desk().length }}</span>
          </h2>
          <p class="muted small">
            These can only be answered where the session runs. What would let you answer them here
            is under each one.
          </p>
          <div class="stack">
            @for (e of desk(); track e.ref.id) {
              <mc-pending-card
                [item]="e.ref"
                [session]="e.session"
                [showSession]="true"
                [gone]="e.gone"
              />
            }
          </div>
        </section>
      }
    </div>
  `,
  styles: [
    `
      h2 {
        display: flex;
        align-items: center;
        gap: 0.4rem;
      }
      h2 .count-badge {
        font-size: 0.7rem;
      }
      .muted-badge {
        background: var(--panel-3);
        color: var(--muted);
      }
      section {
        margin-top: 0.5rem;
      }
      .keys {
        margin: -0.25rem 0 0.6rem;
      }
      @media (pointer: coarse) {
        .keys {
          display: none;
        }
      }
      .entry {
        display: flex;
        flex-direction: column;
        gap: 0.2rem;
      }
      .waited {
        display: flex;
        align-items: center;
        gap: 0.3rem;
        margin: 0;
        font-size: 0.75rem;
        color: var(--amber);
      }
      .empty-state mc-icon {
        color: var(--green);
        display: block;
        margin: 0 auto 0.5rem;
        width: 28px;
      }
      .hint {
        margin: 0.75rem auto 0;
        max-width: 26rem;
      }
      .skeleton {
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
      }
      .skeleton span {
        height: 8rem;
        border-radius: var(--radius);
        background: var(--panel-2);
        animation: mc-shimmer 1.2s ease-in-out infinite alternate;
      }
    `,
  ],
})
export class InboxComponent {
  private realtime = inject(RealtimeService);
  private pendings = inject(PendingService);
  private notices = inject(NoticeService);
  private auth = inject(AuthService);

  private readonly cards = viewChildren(PendingCardComponent, { read: ElementRef });
  private readonly fetched = signal(false);
  /** `GET /inbox`, used until the live store has its first snapshot. */
  private readonly fetchedItems = signal<PendingRef[]>([]);
  protected readonly isDesktop = this.auth.isDesktop;

  protected readonly ready = computed(() => this.realtime.sessionsLoaded() || this.fetched());
  private readonly lingering = lingerList<PendingRef>(() =>
    this.realtime.sessionsLoaded() ? this.realtime.pendingItems() : this.fetchedItems(),
  );
  private readonly entries = computed<Entry[]>(() => {
    const byId = new Map(this.realtime.sessions().map((s) => [s.id, s]));
    return this.lingering()
      .map((l: Lingering<PendingRef>) => ({
        ref: l.item,
        gone: l.gone,
        session: byId.get(l.item.sessionId),
      }))
      .sort((a, b) => a.ref.createdAt - b.ref.createdAt);
  });
  protected readonly needs = computed(() => this.entries().filter((e) => e.ref.answerable));
  protected readonly desk = computed(() => this.entries().filter((e) => !e.ref.answerable));
  /** A live foreign session nobody can answer from here: suggest the plugin. */
  protected readonly foreignHint = computed(() =>
    this.realtime
      .sessions()
      .some((s) => s.live && !s.owned && s.control?.channel === 'none' && !s.control.hooksActive),
  );

  constructor() {
    const release = this.realtime.subscribeInbox();
    const sub = this.realtime.refetch$.subscribe(() => void this.load());
    inject(DestroyRef).onDestroy(() => {
      release();
      sub.unsubscribe();
    });
    void this.load();
  }

  protected focusCard(i: number): void {
    const list = this.cards();
    const target = list[Math.max(0, Math.min(list.length - 1, i))];
    queueMicrotask(() => (target?.nativeElement as HTMLElement | undefined)?.focus());
  }

  private async load(): Promise<void> {
    try {
      this.fetchedItems.set(await this.pendings.loadInbox());
    } catch (e) {
      if (!this.realtime.sessionsLoaded()) this.notices.error(e, () => void this.load(), 'Inbox');
    } finally {
      this.fetched.set(true);
    }
  }
}
