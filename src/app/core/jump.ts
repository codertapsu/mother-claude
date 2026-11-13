import { Injectable, inject, signal } from '@angular/core';
import { NavigationEnd, Router } from '@angular/router';
import { filter } from 'rxjs';

import { fold } from '../pages/help/help-text';
import type { IconName } from '../shared/icon.component';
import { storageGet, storageSet } from './platform';
import type { ProjectRef } from './project-settings';

/**
 * *Jump to…* (docs/UI-REDESIGN.md §9, stage 11, from direction B): one
 * search over the sessions of every computer, the projects, the computers,
 * every page of the map (core/nav.ts), the Settings sections and the Help
 * topics — fuzzy, recent items first. ⌘K (Ctrl+K off Apple systems) opens
 * it anywhere, and so do *Jump to…* in the sidebar and the search button
 * of the phone's top bar. The palette itself (`pages/jump`) loads the
 * first time it opens; this file holds what it ranks by, and the service
 * the shell drives.
 */

export type JumpKind = 'session' | 'project' | 'computer' | 'page' | 'setting' | 'help';

/** Where an item goes: a route, or a project's settings sheet. */
export type JumpTarget =
  | { kind: 'link'; commands: string[]; queryParams?: Record<string, string>; fragment?: string }
  | { kind: 'project'; project: ProjectRef };

export interface JumpItem {
  /** Stable across visits: the recent list keeps it. */
  key: string;
  kind: JumpKind;
  /** In the language shown. */
  title: string;
  /** A second line (its project, its place…), in the language shown. */
  detail?: string;
  icon: IconName;
  /** More words it is found by, never shown (paths, English names, old
   * names). */
  words?: string;
  target: JumpTarget;
}

/** The kinds, in the order a tie puts them. */
const KIND_ORDER: readonly JumpKind[] = [
  'session',
  'page',
  'project',
  'computer',
  'setting',
  'help',
];

/** A recent item ranks above an equal match: this much for the newest. */
const RECENT_BOOST = 12;

/** The key of a session's item (`machine`: its other computer). */
export function sessionKey(id: string, machine?: string | null): string {
  return `session:${machine ?? ''}:${id}`;
}

/** The session a URL opens (an agent's page too), as an item's key. */
export function sessionKeyOf(url: string): string | null {
  const m = /^\/session\/([^/?#]+)(?:\/agent\/[^/?#]+)?(?:\/on\/([^/?#]+))?/.exec(url);
  if (!m) return null;
  try {
    return sessionKey(decodeURIComponent(m[1]), m[2] ? decodeURIComponent(m[2]) : null);
  } catch {
    return null;
  }
}

/** Where a word starts. */
function boundary(c: string | undefined): boolean {
  return c === undefined || /[\s\-_/\\.·:›(,]/.test(c);
}

/** A letter's worth: 1, 4 at a word's start. */
const LETTER = 1;
const WORD_START = 3;
/** …and 3 more right after the one before. */
const IN_A_RUN = 3;

/** The letters of `term` in order through `text`, the first at a word's
 * start ("dpc" in "desktop-pc", "nses" in "new session"), placed where
 * they score best — at word starts and in runs; 0 when they are not all
 * there. At most 50 (below any substring). */
function subsequence(term: string, text: string): number {
  const none = -Infinity;
  // best[j]: the best score with the letter so far at text[j].
  let best: number[] = [...text].map((c, j) =>
    c === term[0] && boundary(text[j - 1]) ? LETTER + WORD_START : none,
  );
  for (let i = 1; i < term.length; i++) {
    const next: number[] = new Array(text.length).fill(none);
    let before = none;
    for (let j = 0; j < text.length; j++) {
      if (j >= 2) before = Math.max(before, best[j - 2]);
      if (text[j] !== term[i]) continue;
      const prev = Math.max(before, j >= 1 ? best[j - 1] + IN_A_RUN : none);
      if (prev === none) continue;
      next[j] = prev + LETTER + (boundary(text[j - 1]) ? WORD_START : 0);
    }
    best = next;
  }
  const top = Math.max(none, ...best);
  return top === none ? 0 : Math.min(50, 10 + Math.round((top / term.length) * 5));
}

/** How well one word of a search (folded) matches a text (folded): the
 * whole text, its start, a word's start, anywhere in it, then (`loose`)
 * its letters in order from a word's start; 0 when it doesn't. */
export function termScore(term: string, text: string, loose = true): number {
  if (!term) return 1;
  if (!text) return 0;
  if (text === term) return 120;
  let at = text.indexOf(term);
  if (at === 0) return 100;
  let best = 0;
  while (at > 0) {
    best = Math.max(best, boundary(text[at - 1]) ? 80 : 60);
    if (best === 80) break;
    at = text.indexOf(term, at + 1);
  }
  return best || (loose ? subsequence(term, text) : 0);
}

/** An item's text as the search compares it. */
export interface FoldedItem {
  title: string;
  words: string;
}

export function foldItem(item: Pick<JumpItem, 'title' | 'detail' | 'words'>): FoldedItem {
  return { title: fold(item.title), words: fold(`${item.detail ?? ''} ${item.words ?? ''}`) };
}

/** How well a search matches an item: every word of it must, in its title
 * (its letters in order too) or, less, as written in the rest; 0 when one
 * doesn't. */
export function matchScore(query: string, item: FoldedItem): number {
  const terms = fold(query).split(/\s+/).filter(Boolean);
  let total = 0;
  for (const t of terms) {
    const s = Math.max(termScore(t, item.title), Math.round(termScore(t, item.words, false) * 0.6));
    if (!s) return 0;
    total += s;
  }
  return total;
}

/**
 * The items to show for a search, best first: a better match, then a more
 * recent one (a recent item also ranks a little higher), then sessions
 * before pages, projects, computers, settings and Help, then by name.
 * Without a search: the recent items, then every page.
 */
export function rankJump(
  items: readonly JumpItem[],
  query: string,
  recent: readonly string[],
  limit = 50,
): JumpItem[] {
  const recency = new Map(recent.map((k, i) => [k, i]));
  if (!query.trim()) {
    const byKey = new Map(items.map((i) => [i.key, i]));
    const first = recent.map((k) => byKey.get(k)).filter((i): i is JumpItem => !!i);
    const shown = new Set(first.map((i) => i.key));
    return [...first, ...items.filter((i) => i.kind === 'page' && !shown.has(i.key))].slice(
      0,
      limit,
    );
  }
  const boost = (key: string) => {
    const at = recency.get(key);
    return at === undefined ? 0 : Math.max(0, RECENT_BOOST - at);
  };
  return items
    .map((item) => ({ item, score: matchScore(query, foldItem(item)) }))
    .filter((x) => x.score > 0)
    .map((x) => ({ ...x, score: x.score + boost(x.item.key) }))
    .sort(
      (a, b) =>
        b.score - a.score ||
        (recency.get(a.item.key) ?? Infinity) - (recency.get(b.item.key) ?? Infinity) ||
        KIND_ORDER.indexOf(a.item.kind) - KIND_ORDER.indexOf(b.item.kind) ||
        a.item.title.localeCompare(b.item.title),
    )
    .slice(0, limit)
    .map((x) => x.item);
}

/** Apple's systems take ⌘K; the others Ctrl+K (on a Mac Ctrl+K edits text). */
export function applePlatform(nav: Pick<Navigator, 'userAgent'> = navigator): boolean {
  return /Mac|iPhone|iPad|iPod/.test(nav.userAgent);
}

/** The key that opens *Jump to…*, as the button shows it. */
export function jumpShortcut(apple: boolean): string {
  return apple ? '⌘K' : 'Ctrl+K';
}

/** The key that opens *Jump to…* was pressed. */
export function isJumpKey(e: KeyboardEvent, apple: boolean): boolean {
  const mod = apple ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  return mod && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'k';
}

const RECENT_KEY = 'mc_jump_recent';
const RECENT_MAX = 12;

function readRecent(): string[] {
  try {
    const raw: unknown = JSON.parse(storageGet(RECENT_KEY) ?? '[]');
    return Array.isArray(raw) ? raw.filter((k): k is string => typeof k === 'string') : [];
  } catch {
    return [];
  }
}

@Injectable({ providedIn: 'root' })
export class JumpService {
  /** The palette shows. */
  readonly opened = signal(false);
  /** It was opened once: the shell loads it (its own chunk). */
  readonly wanted = signal(false);
  /** What was opened lately, newest first (kept on this device). */
  readonly recent = signal<string[]>(readRecent());

  constructor() {
    // A session opened any way counts as recent.
    inject(Router)
      .events.pipe(filter((e) => e instanceof NavigationEnd))
      .subscribe((e) => {
        const key = sessionKeyOf(e.urlAfterRedirects);
        if (key) this.remember(key);
      });
  }

  open(): void {
    this.wanted.set(true);
    this.opened.set(true);
  }

  close(): void {
    this.opened.set(false);
  }

  toggle(): void {
    if (this.opened()) this.close();
    else this.open();
  }

  remember(key: string): void {
    const next = [key, ...this.recent().filter((k) => k !== key)].slice(0, RECENT_MAX);
    this.recent.set(next);
    storageSet(RECENT_KEY, JSON.stringify(next));
  }
}
