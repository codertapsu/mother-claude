// Leases: one Claude Code session's right to act in the browser, shown as one
// tab group (colored, titled after the session). A session acts only in its
// group's tabs; a tab the user drags into the group joins the lease, one
// dragged out leaves it.
//
// Chrome has no empty tab groups, so a new lease's group holds a small
// placeholder page until the session opens its first tab, which reuses it.

import type { LeaseInfo, LeaseOpenFrame, TabGroupColor } from '../shared/protocol.ts';
import { TAB_GROUP_COLORS } from '../shared/protocol.ts';
import type { LeaseRecord, Store } from './state.ts';

export const STOP_HOLD_MS = 60_000;
const TITLE_MAX = 40;

export function placeholderUrl(lease: string): string {
  return chrome.runtime.getURL(`lease.html#${encodeURIComponent(lease)}`);
}

export function isPlaceholderUrl(url: string | undefined): boolean {
  return !!url && url.startsWith(chrome.runtime.getURL('lease.html'));
}

function validColor(c: string | undefined): TabGroupColor | null {
  return c && (TAB_GROUP_COLORS as readonly string[]).includes(c) ? (c as TabGroupColor) : null;
}

/** A stable color for a lease the desktop gave none (never grey: that reads as "off"). */
export function colorFor(lease: string): TabGroupColor {
  let h = 0;
  for (const ch of lease) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  const palette = TAB_GROUP_COLORS.filter((c) => c !== 'grey');
  return palette[h % palette.length] ?? 'blue';
}

export function groupTitle(rec: LeaseRecord): string {
  const t = rec.session.title.trim() || 'Claude Code';
  return t.length > TITLE_MAX ? `${t.slice(0, TITLE_MAX - 1)}…` : t;
}

async function groupExists(groupId: number): Promise<boolean> {
  try {
    await chrome.tabGroups.get(groupId);
    return true;
  } catch {
    return false;
  }
}

export interface LeaseHooks {
  /** The user stopped a lease (Stop, or closed its group). */
  revoked(rec: LeaseRecord): void;
  changed(): void;
}

export class Leases {
  private readonly store: Store;
  private readonly hooks: LeaseHooks;
  /** Groups we remove ourselves: not a user's revoke. */
  private readonly expectRemoval = new Set<number>();
  /** Which lease each known tab belongs to (onRemoved no longer says). */
  private readonly tabLease = new Map<number, string>();
  private readonly opening = new Map<string, Promise<number>>();
  private readonly recentlyLeft = new Map<number, { lease: string; at: number }>();

  constructor(store: Store, hooks: LeaseHooks) {
    this.store = store;
    this.hooks = hooks;
  }

  get(lease: string): LeaseRecord | undefined {
    return this.store.session.leases[lease];
  }

  all(): LeaseRecord[] {
    return Object.values(this.store.session.leases);
  }

  byGroup(groupId: number): LeaseRecord | undefined {
    if (groupId < 0) return undefined;
    return this.all().find((l) => l.groupId === groupId);
  }

  /** The lease of a tab as last seen (works after the tab is gone). */
  leaseOfTab(tabId: number): LeaseRecord | undefined {
    const lease = this.tabLease.get(tabId);
    return lease ? this.get(lease) : undefined;
  }

  noteTab(tabId: number, groupId: number | undefined): void {
    const rec = groupId === undefined ? undefined : this.byGroup(groupId);
    if (rec) {
      this.tabLease.set(tabId, rec.lease);
      this.recentlyLeft.delete(tabId);
      return;
    }
    const before = this.tabLease.get(tabId);
    this.tabLease.delete(tabId);
    if (before) this.recentlyLeft.set(tabId, { lease: before, at: Date.now() });
  }

  /**
   * The lease of a tab that just closed. Chrome reports a closing grouped tab
   * as leaving its group first, so a tab that left a moment ago still counts.
   */
  leaseOfClosedTab(tabId: number): LeaseRecord | undefined {
    const now = this.leaseOfTab(tabId);
    if (now) return now;
    const left = this.recentlyLeft.get(tabId);
    this.recentlyLeft.delete(tabId);
    return left && Date.now() - left.at < 2000 ? this.get(left.lease) : undefined;
  }

  forgetTab(tabId: number): void {
    this.tabLease.delete(tabId);
    for (const rec of this.all()) {
      if (rec.placeholderTabId === tabId) rec.placeholderTabId = null;
      if (rec.activeTabId === tabId) rec.activeTabId = null;
    }
    this.store.persist();
  }

  isStopped(rec: LeaseRecord): boolean {
    return rec.stoppedUntil > Date.now();
  }

  async tabsOf(rec: LeaseRecord): Promise<chrome.tabs.Tab[]> {
    if (rec.groupId === null) return [];
    try {
      const tabs = await chrome.tabs.query({ groupId: rec.groupId });
      for (const t of tabs) if (t.id !== undefined) this.tabLease.set(t.id, rec.lease);
      return tabs.sort((a, b) => a.windowId - b.windowId || a.index - b.index);
    } catch {
      return [];
    }
  }

  /** The lease's tabs with a page (not the placeholder). */
  async pageTabs(rec: LeaseRecord): Promise<chrome.tabs.Tab[]> {
    const tabs = await this.tabsOf(rec);
    return tabs.filter(
      (t) => !(t.id === rec.placeholderTabId && isPlaceholderUrl(t.url ?? t.pendingUrl)),
    );
  }

  async contains(rec: LeaseRecord, tabId: number): Promise<boolean> {
    try {
      const tab = await chrome.tabs.get(tabId);
      return rec.groupId !== null && tab.groupId === rec.groupId;
    } catch {
      return false;
    }
  }

  /** Where commands without a tabId run. */
  async activeTab(rec: LeaseRecord): Promise<number | null> {
    const tabs = await this.pageTabs(rec);
    if (rec.activeTabId !== null && tabs.some((t) => t.id === rec.activeTabId))
      return rec.activeTabId;
    const pick =
      tabs.find((t) => t.active) ??
      [...tabs].sort((a, b) => (b.lastAccessed ?? 0) - (a.lastAccessed ?? 0))[0];
    rec.activeTabId = pick?.id ?? null;
    this.store.persist();
    return rec.activeTabId;
  }

  /** A command for a lease whose group is still being made waits for it. */
  async whenReady(lease: string): Promise<void> {
    await this.opening.get(lease)?.catch(() => undefined);
  }

  async open(frame: LeaseOpenFrame): Promise<number> {
    const work = this.doOpen(frame);
    this.opening.set(frame.lease, work);
    try {
      return await work;
    } finally {
      if (this.opening.get(frame.lease) === work) this.opening.delete(frame.lease);
    }
  }

  private async doOpen(frame: LeaseOpenFrame): Promise<number> {
    const existing = this.get(frame.lease);
    if (existing) {
      existing.session = frame.session;
      const color = validColor(frame.color);
      if (color) existing.color = color;
      existing.stoppedUntil = 0;
      if (existing.groupId !== null && (await groupExists(existing.groupId))) {
        await chrome.tabGroups
          .update(existing.groupId, { title: groupTitle(existing), color: existing.color })
          .catch(() => undefined);
        this.store.persist();
        this.hooks.changed();
        return existing.groupId;
      }
      existing.groupId = null;
      const { groupId } = await this.createGroup(existing);
      this.hooks.changed();
      return groupId;
    }
    const rec: LeaseRecord = {
      lease: frame.lease,
      session: frame.session,
      color: validColor(frame.color) ?? colorFor(frame.lease),
      groupId: null,
      windowId: null,
      placeholderTabId: null,
      activeTabId: null,
      stoppedUntil: 0,
      openedAt: Date.now(),
    };
    this.store.session.leases[rec.lease] = rec;
    const { groupId } = await this.createGroup(rec);
    await this.store.flush();
    this.hooks.changed();
    return groupId;
  }

  private async pickWindow(rec: LeaseRecord): Promise<number> {
    if (rec.windowId !== null) {
      try {
        const w = await chrome.windows.get(rec.windowId);
        if (w.id !== undefined && w.type === 'normal') return w.id;
      } catch {
        // closed
      }
    }
    try {
      const w = await chrome.windows.getLastFocused({ windowTypes: ['normal'] });
      if (w.id !== undefined && !w.incognito) return w.id;
    } catch {
      // no focused window
    }
    const all = await chrome.windows.getAll({ windowTypes: ['normal'] });
    const pick = all.find((w) => !w.incognito) ?? all[0];
    if (pick?.id !== undefined) return pick.id;
    const created = await chrome.windows.create({ focused: false, type: 'normal' });
    if (created?.id === undefined) throw new Error('cannot open a window');
    return created.id;
  }

  /** A new group for the lease, holding the placeholder or a first page. */
  private async createGroup(
    rec: LeaseRecord,
    first?: { url: string; active: boolean },
  ): Promise<{ groupId: number; tabId: number }> {
    const windowId = await this.pickWindow(rec);
    const tab = await chrome.tabs.create({
      windowId,
      url: first?.url ?? placeholderUrl(rec.lease),
      active: first?.active ?? false,
    });
    if (tab.id === undefined) throw new Error('the tab has no id');
    const groupId = await chrome.tabs.group({ tabIds: [tab.id], createProperties: { windowId } });
    await chrome.tabGroups.update(groupId, {
      title: groupTitle(rec),
      color: rec.color,
      collapsed: false,
    });
    rec.groupId = groupId;
    rec.windowId = windowId;
    if (first) {
      rec.placeholderTabId = null;
      rec.activeTabId = tab.id;
    } else {
      rec.placeholderTabId = tab.id;
    }
    this.tabLease.set(tab.id, rec.lease);
    this.store.persist();
    return { groupId, tabId: tab.id };
  }

  /** Open a page in the lease's group (reusing the placeholder). */
  async openTab(rec: LeaseRecord, url: string, active: boolean): Promise<chrome.tabs.Tab> {
    if (rec.placeholderTabId !== null) {
      const id = rec.placeholderTabId;
      rec.placeholderTabId = null;
      try {
        const ph = await chrome.tabs.get(id);
        if (ph.groupId === rec.groupId) {
          const tab = await chrome.tabs.update(id, { url, active });
          rec.activeTabId = id;
          this.store.persist();
          if (tab) return tab;
        }
      } catch {
        // the user closed it
      }
    }
    if (rec.groupId !== null && (await groupExists(rec.groupId))) {
      const tabs = await this.tabsOf(rec);
      const last = tabs[tabs.length - 1];
      const tab = await chrome.tabs.create({
        url,
        active,
        ...(last ? { windowId: last.windowId, index: last.index + 1 } : {}),
      });
      if (tab.id === undefined) throw new Error('the tab has no id');
      await chrome.tabs.group({ groupId: rec.groupId, tabIds: [tab.id] });
      this.tabLease.set(tab.id, rec.lease);
      rec.activeTabId = tab.id;
      this.store.persist();
      return tab;
    }
    const { tabId } = await this.createGroup(rec, { url, active });
    this.hooks.changed();
    return chrome.tabs.get(tabId);
  }

  /**
   * The user's own tab joins the lease (the side panel's Connect): into its
   * group, or a new group made around it, and it becomes the lease's active
   * tab. The placeholder page goes if nothing used it yet.
   */
  async adopt(rec: LeaseRecord, tabId: number): Promise<void> {
    const other = this.leaseOfTab(tabId);
    if (other && other.lease !== rec.lease) {
      throw new Error('this tab already works for another session (Stop it there first)');
    }
    const tab = await chrome.tabs.get(tabId);
    if (rec.groupId !== null && (await groupExists(rec.groupId))) {
      if (tab.groupId !== rec.groupId) {
        await chrome.tabs.group({ groupId: rec.groupId, tabIds: [tabId] });
      }
    } else {
      const groupId = await chrome.tabs.group({
        tabIds: [tabId],
        createProperties: { windowId: tab.windowId },
      });
      await chrome.tabGroups.update(groupId, {
        title: groupTitle(rec),
        color: rec.color,
        collapsed: false,
      });
      rec.groupId = groupId;
      rec.windowId = tab.windowId;
    }
    this.tabLease.set(tabId, rec.lease);
    rec.activeTabId = tabId;
    const placeholder = rec.placeholderTabId;
    rec.placeholderTabId = null;
    if (placeholder !== null && placeholder !== tabId) {
      try {
        const ph = await chrome.tabs.get(placeholder);
        if (isPlaceholderUrl(ph.url ?? ph.pendingUrl)) await chrome.tabs.remove(placeholder);
      } catch {
        // already gone
      }
    }
    this.store.persist();
    this.hooks.changed();
  }

  /** The command closes a tab; when it was the last one the group goes too, quietly. */
  async closeTab(rec: LeaseRecord, tabId: number): Promise<void> {
    const tabs = await this.tabsOf(rec);
    if (tabs.length <= 1 && rec.groupId !== null) this.expectRemoval.add(rec.groupId);
    await chrome.tabs.remove(tabId);
  }

  /** End a lease: ungroup (or close) its tabs. Detaching the debugger is the caller's. */
  async close(lease: string, closeTabs: boolean): Promise<number[]> {
    const rec = this.get(lease);
    if (!rec) return [];
    const tabs = await this.tabsOf(rec);
    delete this.store.session.leases[lease];
    void this.store.flush();
    if (rec.groupId !== null) this.expectRemoval.add(rec.groupId);
    const ids = tabs.map((t) => t.id).filter((id): id is number => id !== undefined);
    for (const id of ids) this.tabLease.delete(id);
    const placeholder = rec.placeholderTabId;
    try {
      if (closeTabs) {
        if (ids.length) await chrome.tabs.remove(ids);
      } else {
        const [first, ...rest] = ids.filter((id) => id !== placeholder);
        if (first !== undefined) await chrome.tabs.ungroup([first, ...rest]);
        if (placeholder !== null && ids.includes(placeholder))
          await chrome.tabs.remove(placeholder);
      }
    } catch {
      // tabs already gone
    }
    this.hooks.changed();
    return ids;
  }

  /** Stop pressed: refuse the lease's commands for a minute (or until resumed). */
  revoke(lease: string): LeaseRecord | undefined {
    const rec = this.get(lease);
    if (!rec) return undefined;
    rec.stoppedUntil = Date.now() + STOP_HOLD_MS;
    // Written at once, not coalesced: a Stop must survive a worker restart.
    void this.store.flush();
    this.hooks.revoked(rec);
    this.hooks.changed();
    return rec;
  }

  /** Lift a Stop; resolves once that is saved (before the desktop hears of it). */
  async resume(lease: string): Promise<LeaseRecord | undefined> {
    const rec = this.get(lease);
    if (!rec) return undefined;
    rec.stoppedUntil = 0;
    await this.store.flush();
    this.hooks.changed();
    return rec;
  }

  /** The group disappeared: the user closed it, unless we removed it ourselves. */
  onGroupRemoved(groupId: number): void {
    const rec = this.byGroup(groupId);
    if (!rec) {
      this.expectRemoval.delete(groupId);
      return;
    }
    rec.groupId = null;
    rec.placeholderTabId = null;
    rec.activeTabId = null;
    this.store.persist();
    if (this.expectRemoval.delete(groupId)) {
      this.hooks.changed();
      return;
    }
    this.revoke(rec.lease);
  }

  onTabActivated(tabId: number): void {
    const rec = this.leaseOfTab(tabId);
    if (rec && rec.placeholderTabId !== tabId) {
      rec.activeTabId = tabId;
      this.store.persist();
    }
  }

  /** After a restart: check every group still exists, else find it by its title. */
  async verify(): Promise<void> {
    for (const rec of this.all()) {
      if (rec.groupId !== null && (await groupExists(rec.groupId))) {
        await this.tabsOf(rec);
        continue;
      }
      rec.groupId = await this.findGroup(rec);
      if (rec.groupId !== null) await this.tabsOf(rec);
    }
    this.store.persist();
  }

  private async findGroup(rec: LeaseRecord): Promise<number | null> {
    try {
      const groups = await chrome.tabGroups.query({ title: groupTitle(rec), color: rec.color });
      const taken = new Set(this.all().map((l) => l.groupId));
      return groups.find((g) => !taken.has(g.id))?.id ?? null;
    } catch {
      return null;
    }
  }

  /** The desktop's list is the truth: adopt what it has, release what it dropped. */
  async reconcile(list: LeaseInfo[]): Promise<void> {
    const wanted = new Map(list.map((l) => [l.lease, l]));
    for (const rec of this.all()) {
      if (!wanted.has(rec.lease)) await this.close(rec.lease, false);
    }
    for (const info of list) {
      let rec = this.get(info.lease);
      if (!rec) {
        rec = {
          lease: info.lease,
          session: info.session ?? { id: '', title: '' },
          color: validColor(info.color) ?? colorFor(info.lease),
          groupId: null,
          windowId: null,
          placeholderTabId: null,
          activeTabId: null,
          stoppedUntil: 0,
          openedAt: Date.now(),
        };
        this.store.session.leases[rec.lease] = rec;
      } else {
        const color = validColor(info.color);
        const changed =
          (info.session && info.session.title !== rec.session.title) ||
          (color && color !== rec.color);
        if (info.session) rec.session = info.session;
        if (color) rec.color = color;
        if (changed && rec.groupId !== null) {
          await chrome.tabGroups
            .update(rec.groupId, { title: groupTitle(rec), color: rec.color })
            .catch(() => undefined);
        }
      }
      if (rec.groupId === null || !(await groupExists(rec.groupId))) {
        // Group ids are not stable across browser restarts: never adopt the
        // desktop's remembered id, only a group with this lease's title and color.
        rec.groupId = await this.findGroup(rec);
        if (rec.groupId !== null) await this.tabsOf(rec);
      }
    }
    this.store.persist();
    this.hooks.changed();
  }
}
