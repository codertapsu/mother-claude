import { Injectable, inject, signal } from '@angular/core';
import { Router } from '@angular/router';

import { ApiService } from './api.service';
import { FolderChoiceService, folderQuestionOf } from './folder-choice.service';
import { tr } from './i18n/i18n';
import { ContinueBody, SessionSummary } from './models';
import { NoticeService } from './notice.service';
import { SESSION_MACHINE, sessionLink } from './session-links';

/** The answer to "continue on another account": the new session, or the
 * refusal for the caller to explain (the sheet shows it in place) — with
 * the folder the user chose here meanwhile, to send again with a retry —
 * or `cancelled` when the user closed the question of its folder. */
export type ContinueOnResult =
  { ok: true; id: string } | { ok: false; error: unknown; folder?: string; cancelled?: boolean };

type Attempt = { ok: true; id: string } | { ok: false; error: unknown };

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
  private folders = inject(FolderChoiceService);
  /** Another computer's sessions (its scope's copy of this service): the
   * pages opened are that computer's. */
  private machine = inject(SESSION_MACHINE);

  /** Ids with a continue/fork request in flight. */
  readonly busy = signal<ReadonlySet<string>>(new Set());

  /**
   * Continue as an owned session. A live foreign session is always forked
   * into a new id (never two writers on one transcript); an ended one
   * resumes in place. One another computer started (or whose folder is
   * gone) asks for its folder here first. Navigates to the resulting
   * session.
   */
  async continueHere(
    s: Pick<SessionSummary, 'id' | 'live'>,
    body: ContinueBody = {},
  ): Promise<string | null> {
    if (this.busy().has(s.id)) return null;
    const res = await this.attempt(s, body);
    if (res.ok) {
      this.notices.success(
        res.id !== s.id
          ? tr('Continued here as a new session: you are driving the copy from here.')
          : tr('You can drive this session from here now.'),
      );
      await this.openSession(s.id, res.id);
      return res.id;
    }
    const question = body.folder ? null : folderQuestionOf(res.error);
    if (question) {
      const folder = await this.folders.ask(question);
      return folder ? this.continueHere(s, { ...body, folder }) : null;
    }
    this.notices.error(res.error, () => void this.continueHere(s, body), tr('Could not continue'));
    return null;
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
    folder?: string,
  ): Promise<ContinueOnResult> {
    if (this.busy().has(s.id)) return { ok: false, error: new Error(tr('Already on its way.')) };
    const res = await this.attempt(s, { account: account.id, ...(folder ? { folder } : {}) });
    if (res.ok) {
      this.notices.success(
        res.id !== s.id
          ? tr('Continuing on {account}: you are driving the copy from here.', {
              account: account.name,
            })
          : tr('Continuing on {account}.', { account: account.name }),
      );
      await this.openSession(s.id, res.id);
      return res;
    }
    const question = folder ? null : folderQuestionOf(res.error);
    if (question) {
      const chosen = await this.folders.ask(question);
      if (!chosen) return { ok: false, error: res.error, cancelled: true };
      return this.continueOn(s, account, chosen);
    }
    return { ok: false, error: res.error, ...(folder ? { folder } : {}) };
  }

  /** `POST /sessions/{id}/continue`, the session busy meanwhile. */
  private async attempt(
    s: Pick<SessionSummary, 'id' | 'live'>,
    body: ContinueBody,
  ): Promise<Attempt> {
    this.setBusy(s.id, true);
    try {
      const res = await this.api.continueSession(s.id, {
        ...(s.live ? { fork: true } : {}),
        ...body,
      });
      return { ok: true, id: res?.id || s.id };
    } catch (e) {
      return { ok: false, error: e };
    } finally {
      this.setBusy(s.id, false);
    }
  }

  /** Open the session page with its terminal view showing. */
  openTerminal(id: string): void {
    void this.router.navigate(sessionLink(id, this.machine), { queryParams: { view: 'terminal' } });
  }

  private async openSession(from: string, id: string): Promise<void> {
    const page = this.machine ? `/session/${from}/on/${this.machine}` : `/session/${from}`;
    if (id !== from || !this.router.url.startsWith(page)) {
      await this.router.navigate(sessionLink(id, this.machine));
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
