import { Injectable, NgZone, computed, inject, signal } from '@angular/core';
import { Router } from '@angular/router';

/**
 * List and detail on wide screens (docs/UI-REDESIGN.md §9, stage 9): from
 * {@link WIDE_QUERY} up, a session opened from Sessions (or its Agents tab)
 * or from the Inbox shows beside the list instead of replacing it. The URL
 * stays the session's own (`#/session/<id>`, `…/on/<machine>`), so a
 * bookmark, the browser's Back and a notification's `?pending=` open it as
 * before; narrower screens and phones show it on its own page, as before.
 *
 * The layout (`ListDetailComponent`) tells this service what shows; the
 * shell, the lists and a session's header read it.
 */

/** The lists of the work a session can open beside. */
export type WorkList = 'sessions' | 'inbox' | 'agents' | 'projects';

/** Where each list lives. */
export const LIST_PATHS: Readonly<Record<WorkList, string>> = {
  sessions: '/',
  inbox: '/inbox',
  agents: '/agents',
  projects: '/projects',
};

/** From this width a session opens beside its list. */
export const WIDE_QUERY = '(min-width: 1280px)';

/** What the detail shows: a session (or one of its agents), on this
 * computer (`machine` null) or another. */
export interface DetailRef {
  id: string;
  machine: string | null;
  agent: string | null;
}

export function isWorkList(v: unknown): v is WorkList {
  return typeof v === 'string' && Object.hasOwn(LIST_PATHS, v);
}

/** The list a session shows beside: Projects' rows open no session, so the
 * Sessions list stands in for it. */
export function besideList(list: WorkList): WorkList {
  return list === 'projects' ? 'sessions' : list;
}

/** A list's section (core/nav.ts): what the sidebar marks while it shows. */
export function listSection(list: WorkList): string {
  return LIST_PATHS[list].replace(/^\//, '');
}

function media(query: string): MediaQueryList | null {
  try {
    return typeof matchMedia === 'function' ? matchMedia(query) : null;
  } catch {
    return null;
  }
}

@Injectable({ providedIn: 'root' })
export class ListDetailService {
  private readonly router = inject(Router);

  /** The screen is wide enough for a list and a session side by side. */
  readonly wide = signal(media(WIDE_QUERY)?.matches ?? false);
  /** The list shown last (the one a session opens beside). */
  readonly list = signal<WorkList>('sessions');
  /** That list's URL, its query and fragment kept (`/?machine=…`). */
  readonly listUrl = signal(LIST_PATHS.sessions);
  /** What the detail shows (null: a list on its own, or another page). */
  readonly detail = signal<DetailRef | null>(null);

  /** The list beside a session. */
  readonly beside = computed(() => besideList(this.list()));
  /** A session shows beside its list now. */
  readonly split = computed(() => this.wide() && !!this.detail());
  /** Where closing the detail goes: the list beside it. */
  readonly closeUrl = computed(() =>
    this.beside() === this.list() ? this.listUrl() : LIST_PATHS[this.beside()],
  );

  /** A list was shown since the app started; until then a link opened a
   * session first (a notification, a bookmark). */
  private listSeen = false;

  constructor() {
    const zone = inject(NgZone);
    media(WIDE_QUERY)?.addEventListener?.('change', (e) =>
      zone.run(() => this.wide.set(e.matches)),
    );
  }

  /** A list's route shows (`url`: its address). */
  showList(list: WorkList, url: string): void {
    this.listSeen = true;
    this.list.set(list);
    this.listUrl.set(url);
    this.detail.set(null);
  }

  /** A session's route shows. A notification's link (`pending`) opened
   * before any list: the Inbox, where requests are, goes beside it. */
  showDetail(ref: DetailRef, pending: boolean): void {
    if (!this.listSeen && pending) {
      this.list.set('inbox');
      this.listUrl.set(LIST_PATHS.inbox);
    }
    const d = this.detail();
    if (d?.id !== ref.id || d.machine !== ref.machine || d.agent !== ref.agent) {
      this.detail.set(ref);
    }
  }

  /** Another page shows: no list, no detail. */
  leave(): void {
    this.detail.set(null);
  }

  /** This session (`machine`: its other computer, none for this one) shows
   * beside the list. */
  isOpen(id: string, machine?: string | null): boolean {
    const d = this.detail();
    return this.split() && !!d && d.id === id && d.machine === (machine ?? null);
  }

  /** Close the detail: the list on its own again. */
  close(): Promise<boolean> {
    return this.router.navigateByUrl(this.closeUrl());
  }
}
