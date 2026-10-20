import { Injectable, inject, signal } from '@angular/core';
import { Router } from '@angular/router';

import { ApiError, ApiService } from './api.service';
import { ContinueBody, SessionSummary } from './models';
import { NoticeService } from './notice.service';

/** The answer to "continue on another account": the new session, or the
 * refusal for the caller to explain (the sheet shows it in place). */
export type ContinueOnResult = { ok: true; id: string } | { ok: false; error: unknown };

/** After a stop, look for the session to be let go this often (it is
 * refused with `already_owned` until its process has exited). */
const RELEASE_WAITS_MS = [300, 500, 800, 1_200, 1_600, 2_400, 3_200];

/** The server refused because Mother Claude drives the session right now. */
export function isAlreadyOwned(e: unknown): boolean {
  return e instanceof ApiError && e.status === 409 && e.code === 'already_owned';
}

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
   * was. Navigates to the copy; a refusal is returned, not shown.
   */
  async continueOn(
    s: Pick<SessionSummary, 'id' | 'live'>,
    account: { id: string; name: string },
  ): Promise<ContinueOnResult> {
    if (this.busy().has(s.id)) return { ok: false, error: new Error('Already on its way.') };
    this.setBusy(s.id, true);
    try {
      return await this.continueOnce(s, account, !!s.live);
    } catch (e) {
      return { ok: false, error: e };
    } finally {
      this.setBusy(s.id, false);
    }
  }

  /**
   * For a session Mother Claude is running right now (the server lets go of
   * it only once its process has exited): stop it, then continue it on
   * `account` as soon as the server allows.
   */
  async stopThenContinueOn(
    s: Pick<SessionSummary, 'id'>,
    account: { id: string; name: string },
  ): Promise<ContinueOnResult> {
    if (this.busy().has(s.id)) return { ok: false, error: new Error('Already on its way.') };
    this.setBusy(s.id, true);
    try {
      await this.api.lifecycle(s.id, 'stop');
      let last: unknown = null;
      for (const wait of RELEASE_WAITS_MS) {
        await new Promise((r) => setTimeout(r, wait));
        try {
          return await this.continueOnce(s, account, true);
        } catch (e) {
          if (!isAlreadyOwned(e)) return { ok: false, error: e };
          last = e;
        }
      }
      return {
        ok: false,
        error: last ?? new Error('The session is still stopping. Try again in a moment.'),
      };
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

  private async continueOnce(
    s: Pick<SessionSummary, 'id'>,
    account: { id: string; name: string },
    fork: boolean,
  ): Promise<ContinueOnResult> {
    const res = await this.api.continueSession(s.id, {
      account: account.id,
      ...(fork ? { fork: true } : {}),
    });
    const id = res?.id || s.id;
    this.notices.success(
      id !== s.id
        ? `Continuing on ${account.name}: you are driving the copy from here.`
        : `Continuing on ${account.name}.`,
    );
    await this.openSession(s.id, id);
    return { ok: true, id };
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
