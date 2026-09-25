import { Injectable, inject, signal } from '@angular/core';
import { Router } from '@angular/router';

import { ApiService } from './api.service';
import { ContinueBody, SessionSummary } from './models';
import { NoticeService } from './notice.service';

/**
 * Actions offered from more than one place (a limitation's button in the
 * channel sheet on the list, the session page, the Info tab): take a session
 * over as an owned one, fork it, open its terminal view.
 */
@Injectable({ providedIn: 'root' })
export class SessionActionsService {
  private api = inject(ApiService);
  private router = inject(Router);
  private notices = inject(NoticeService);

  /** Ids with a continue/fork request in flight. */
  readonly busy = signal<ReadonlySet<string>>(new Set());

  /**
   * Continue as an owned session. A live foreign session is always forked
   * into a new id (never two writers on one transcript); an ended one
   * resumes in place. Navigates to the resulting session.
   */
  async continueHere(
    s: Pick<SessionSummary, 'id' | 'live'>,
    body: ContinueBody = {},
  ): Promise<string | null> {
    if (this.busy().has(s.id)) return null;
    this.busy.update((prev) => new Set(prev).add(s.id));
    try {
      const res = await this.api.continueSession(s.id, {
        ...(s.live ? { fork: true } : {}),
        ...body,
      });
      const id = res?.id || s.id;
      this.notices.success(
        id !== s.id
          ? 'Forked: you are driving the copy from here.'
          : 'You can drive this session from here now.',
      );
      if (id !== s.id || !this.router.url.startsWith(`/session/${s.id}`)) {
        await this.router.navigate(['/session', id]);
      }
      return id;
    } catch (e) {
      this.notices.error(e, () => void this.continueHere(s, body), 'Could not continue');
      return null;
    } finally {
      this.busy.update((prev) => {
        const next = new Set(prev);
        next.delete(s.id);
        return next;
      });
    }
  }

  /** Open the session page with its terminal view showing. */
  openTerminal(id: string): void {
    void this.router.navigate(['/session', id], { queryParams: { view: 'terminal' } });
  }
}
