import { Injectable, inject, signal } from '@angular/core';
import { Router } from '@angular/router';

import { ApiService } from './api.service';
import { ContinueBody, SessionSummary } from './models';
import { NoticeService } from './notice.service';

/** The answer to "continue on another account": the new session, or the
 * refusal for the caller to explain (the sheet shows it in place). */
export type ContinueOnResult = { ok: true; id: string } | { ok: false; error: unknown };

/**
 * Actions offered from more than one place (a limitation's button in the
 * channel sheet on the list, the session page, the Info tab): take a session
 * over as an owned one, fork it, continue it on another Claude account, open
 * its terminal view.
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
    this.setBusy(s.id, true);
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
      await this.openSession(s.id, id);
      return id;
    } catch (e) {
      this.notices.error(e, () => void this.continueHere(s, body), 'Could not continue');
      return null;
    } finally {
      this.setBusy(s.id, false);
    }
  }

  /**
   * "Continue on another account" (docs/ACCOUNTS.md §5.4): the conversation
   * continues as a copy (a fork) under `account`, the original stays as it
   * was — also one Mother Claude runs right now (it keeps running). For an
   * account with its own directory the server copies the conversation
   * there first, once the desktop allowed copies between the two accounts
   * (409 `copy_consent_required` until then). Navigates to the copy; a
   * refusal is returned, not shown.
   */
  async continueOn(
    s: Pick<SessionSummary, 'id' | 'live'>,
    account: { id: string; name: string },
  ): Promise<ContinueOnResult> {
    if (this.busy().has(s.id)) return { ok: false, error: new Error('Already on its way.') };
    this.setBusy(s.id, true);
    try {
      const res = await this.api.continueSession(s.id, {
        account: account.id,
        ...(s.live ? { fork: true } : {}),
      });
      const id = res?.id || s.id;
      this.notices.success(
        id !== s.id
          ? `Continuing on ${account.name}: you are driving the copy from here.`
          : `Continuing on ${account.name}.`,
      );
      await this.openSession(s.id, id);
      return { ok: true, id };
    } catch (e) {
      return { ok: false, error: e };
    } finally {
      this.setBusy(s.id, false);
    }
  }

  /** Open the session page with its terminal view showing. */
  openTerminal(id: string): void {
    void this.router.navigate(['/session', id], { queryParams: { view: 'terminal' } });
  }

  private async openSession(from: string, id: string): Promise<void> {
    if (id !== from || !this.router.url.startsWith(`/session/${from}`)) {
      await this.router.navigate(['/session', id]);
    }
  }

  private setBusy(id: string, on: boolean): void {
    this.busy.update((prev) => {
      const next = new Set(prev);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });
  }
}
