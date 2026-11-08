import { Component, DestroyRef, computed, effect, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';
import { Subscription, debounceTime } from 'rxjs';

import { ApiService, errorMessage } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import {
  BrowserActivity,
  BrowserOverview,
  BrowserPairCode,
  GRANT_ACTIONS,
  GrantAction,
  MODE_HINTS,
  MODE_LABELS,
  SITE_MODES,
  SiteEntry,
  SiteMode,
  actNeedsDesktop,
  activityText,
  grantLabel,
  hostOf,
  normalizeSite,
  outcomeBadge,
  sensitiveCategory,
} from '../../core/browser';
import { PendingRef } from '../../core/models';
import { NoticeService } from '../../core/notice.service';
import { NowService } from '../../core/now.service';
import { RealtimeService } from '../../core/realtime.service';
import { IconComponent } from '../../shared/icon.component';
import { PendingCardComponent } from '../../shared/pending-card.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';
import { sessionName } from '../../shared/util';

/**
 * The Browser page: the Mother Claude extension (docs/BROWSER.md). Pair a
 * browser, see which are connected and which sessions act in them (Stop,
 * Resume), set each site's mode, see the sensitive list, the standing
 * grants, the settings and the activity — and answer waiting browser
 * approvals. One code path for the desktop and the phone: a phone reads,
 * approves (with the dangerous grant) and sets ordinary sites' modes; what
 * needs the desktop says so.
 */
@Component({
  selector: 'mc-browser',
  imports: [FormsModule, RouterLink, IconComponent, PendingCardComponent, TimeAgoComponent, TPipe],
  templateUrl: './browser.component.html',
  styleUrl: './browser.component.scss',
})
export class BrowserComponent {
  private api = inject(ApiService);
  private auth = inject(AuthService);
  private realtime = inject(RealtimeService);
  private notices = inject(NoticeService);
  private clock = inject(NowService);

  protected readonly modes = SITE_MODES;
  protected readonly modeLabels = MODE_LABELS;
  protected readonly modeHints = MODE_HINTS;
  protected readonly grantActions = GRANT_ACTIONS;
  protected readonly grantLabel = grantLabel;
  protected readonly activityText = activityText;
  protected readonly outcomeBadge = outcomeBadge;
  protected readonly hostOf = hostOf;

  protected readonly data = signal<BrowserOverview | null>(null);
  protected readonly activity = signal<BrowserActivity[]>([]);
  protected readonly error = signal('');
  protected readonly loading = signal(false);
  /** Keys of the actions in flight (`site:https://x`, `revoke:brw_…`). */
  protected readonly busy = signal<ReadonlySet<string>>(new Set());
  protected readonly code = signal<BrowserPairCode | null>(null);
  protected readonly confirmRevoke = signal<string | null>(null);
  protected readonly showSensitive = signal(false);
  protected readonly activitySession = signal('');

  // Forms.
  protected readonly newSite = signal('');
  protected readonly newSiteMode = signal<SiteMode>('read');
  protected readonly newGrantAction = signal<GrantAction>('eval');
  protected readonly newGrantOrigin = signal('');
  protected readonly newFolder = signal('');

  protected readonly canDangerous = this.auth.canDangerous;
  protected readonly isDesktop = this.auth.isDesktop;

  protected readonly browsers = computed(() =>
    (this.data()?.browsers ?? []).filter((b) => !b.revoked),
  );
  protected readonly connected = computed(() => this.browsers().filter((b) => b.connected));
  protected readonly defaultMode = computed<SiteMode>(
    () => this.data()?.settings.defaultMode ?? 'off',
  );
  /** Every site with its own mode (not `*`), sorted by host. */
  protected readonly sites = computed<SiteEntry[]>(() =>
    (this.data()?.sites ?? [])
      .filter((s) => s.origin !== '*')
      .sort((a, b) => hostOf(a.origin).localeCompare(hostOf(b.origin))),
  );
  protected readonly sensitiveGroups = computed(() => {
    const groups = new Map<string, string[]>();
    for (const s of this.data()?.sensitive ?? []) {
      groups.set(s.category, [...(groups.get(s.category) ?? []), s.domain]);
    }
    return [...groups.entries()].map(([category, domains]) => ({ category, domains }));
  });
  /** Waiting browser approvals, as the pending card takes them. */
  protected readonly approvals = computed<PendingRef[]>(() =>
    (this.data()?.approvals ?? []).map((a) => ({
      id: a.id,
      sessionId: a.session,
      kind: 'permission',
      source: 'browser',
      title: a.summary,
      detail: a.detail,
      toolName: undefined,
      dangerous: true,
      answerable: true,
      createdAt: a.createdAt,
    })),
  );
  protected readonly stops = computed(() =>
    (this.data()?.stops ?? []).filter((s) => s.until > this.clock.now()),
  );
  /** Sessions the activity names, for the filter. */
  protected readonly activitySessions = computed(() => {
    const ids = new Set<string>();
    for (const l of this.data()?.leases ?? []) ids.add(l.session);
    for (const e of this.activity()) ids.add(e.session);
    return [...ids].map((id) => ({ id, label: this.sessionLabel(id) }));
  });
  protected readonly codeLeft = computed(() => {
    const c = this.code();
    if (!c) return 0;
    return Math.max(0, Math.round((c.expiresAt - this.clock.now()) / 1000));
  });
  protected readonly newSiteKey = computed(() => normalizeSite(this.newSite()));
  protected readonly newSiteBlocked = computed(() => {
    const key = this.newSiteKey();
    const d = this.data();
    return (
      !!key &&
      !!d &&
      this.newSiteMode() === 'act' &&
      actNeedsDesktop(key, d.sensitive) &&
      !this.canDangerous()
    );
  });

  private sub?: Subscription;

  constructor() {
    void this.load();
    this.sub = this.realtime.browser$.pipe(debounceTime(150)).subscribe(() => void this.load());
    this.sub.add(this.realtime.refetch$.subscribe(() => void this.load()));
    // An approval answered anywhere leaves the list.
    this.sub.add(
      this.realtime.pendingResolved$.subscribe((r) => {
        if (this.data()?.approvals.some((a) => a.id === r.id)) void this.load();
      }),
    );
    inject(DestroyRef).onDestroy(() => this.sub?.unsubscribe());
    // A code is useless once expired.
    effect(() => {
      if (this.code() && this.codeLeft() === 0) this.code.set(null);
    });
  }

  protected sessionLabel(id: string): string {
    const s = this.realtime.sessions().find((x) => x.id === id);
    if (s) return sessionName(s);
    const lease = this.data()?.leases.find((l) => l.session === id);
    return lease?.title || id.slice(0, 8);
  }

  protected secondsLeft(until: number): number {
    return Math.max(0, Math.round((until - this.clock.now()) / 1000));
  }

  protected browserName(id: string): string {
    return this.data()?.browsers.find((b) => b.id === id)?.name ?? tr('a browser');
  }

  protected isBusy(key: string): boolean {
    return this.busy().has(key);
  }

  protected sensitiveOf(origin: string): string | undefined {
    const d = this.data();
    return d ? sensitiveCategory(origin, d.sensitive) : undefined;
  }

  /** `act` for this site needs the desktop (or a granted device). */
  protected actLocked(origin: string): boolean {
    const d = this.data();
    return !!d && !this.canDangerous() && actNeedsDesktop(origin, d.sensitive);
  }

  protected async load(): Promise<void> {
    this.loading.set(true);
    try {
      const [overview, activity] = await Promise.all([
        this.api.getBrowser(),
        this.api.getBrowserActivity(this.activitySession() || undefined),
      ]);
      this.data.set(overview);
      this.activity.set(activity.entries);
      this.error.set('');
    } catch (e) {
      this.error.set(
        tr('Could not load the browser settings: {error}', { error: errorMessage(e) }),
      );
    } finally {
      this.loading.set(false);
    }
  }

  protected async filterActivity(session: string): Promise<void> {
    this.activitySession.set(session);
    try {
      this.activity.set((await this.api.getBrowserActivity(session || undefined)).entries);
    } catch (e) {
      this.notices.error(errorMessage(e));
    }
  }

  /** Run `work` with `key` marked busy; its overview replaces ours. */
  private async run(
    key: string,
    work: () => Promise<BrowserOverview | unknown>,
    done?: string,
  ): Promise<boolean> {
    if (this.isBusy(key)) return false;
    this.busy.update((s) => new Set([...s, key]));
    try {
      const out = await work();
      if (out && typeof out === 'object' && 'browsers' in out) {
        this.data.set(out as BrowserOverview);
      } else {
        await this.load();
      }
      if (done) this.notices.success(done);
      return true;
    } catch (e) {
      this.notices.error(errorMessage(e));
      return false;
    } finally {
      this.busy.update((s) => {
        const next = new Set(s);
        next.delete(key);
        return next;
      });
    }
  }

  // ── pairing

  protected async pair(): Promise<void> {
    if (this.isBusy('pair')) return;
    this.busy.update((s) => new Set([...s, 'pair']));
    try {
      this.code.set(await this.api.createBrowserPairCode());
    } catch (e) {
      this.notices.error(errorMessage(e));
    } finally {
      this.busy.update((s) => {
        const next = new Set(s);
        next.delete('pair');
        return next;
      });
    }
  }

  protected async revoke(id: string): Promise<void> {
    this.confirmRevoke.set(null);
    await this.run(`revoke:${id}`, () => this.api.revokeBrowser(id), tr('Browser removed'));
  }

  protected pin(id: string, pinned: boolean): Promise<boolean> {
    return this.run(`pin:${id}`, () =>
      this.api.setBrowserSettings({ pinnedBrowser: pinned ? id : '' }),
    );
  }

  // ── sessions

  protected stop(session: string): Promise<boolean> {
    return this.run(
      `stop:${session}`,
      () => this.api.stopBrowserSession(session),
      tr('Stopped: its browser actions are refused for a minute.'),
    );
  }

  protected resume(session: string): Promise<boolean> {
    return this.run(
      `resume:${session}`,
      () => this.api.resumeBrowserSession(session),
      tr('Resumed'),
    );
  }

  // ── site modes

  protected setMode(origin: string, mode: SiteMode | null): Promise<boolean> {
    return this.run(`site:${origin}`, () => this.api.setBrowserSite(origin, mode));
  }

  protected async addSite(): Promise<void> {
    const key = this.newSiteKey();
    if (!key || this.newSiteBlocked()) return;
    if (await this.setMode(key, this.newSiteMode())) this.newSite.set('');
  }

  // ── grants

  protected removeGrant(id: string): Promise<boolean> {
    return this.run(`grant:${id}`, () => this.api.removeBrowserGrant(id), tr('Grant removed'));
  }

  protected async addGrant(): Promise<void> {
    const origin = normalizeSite(this.newGrantOrigin());
    if (!origin || origin === '*' || origin.includes('://*.')) {
      this.notices.error(tr('Give one site, like https://localhost:3000.'));
      return;
    }
    const ok = await this.run('grant:new', () =>
      this.api.addBrowserGrant(this.newGrantAction(), origin),
    );
    if (ok) this.newGrantOrigin.set('');
  }

  // ── settings

  protected setScreenshots(on: boolean): Promise<boolean> {
    return this.run('settings', () => this.api.setBrowserSettings({ screenshots: on }));
  }

  protected setLimit(value: number | string): Promise<boolean> | undefined {
    const n = Math.round(Number(value));
    if (!Number.isFinite(n) || n < 1 || n > 600) {
      this.notices.error(tr('The limit is between 1 and 600 actions a minute.'));
      return undefined;
    }
    return this.run('settings', () => this.api.setBrowserSettings({ actionsPerMinute: n }));
  }

  protected async addFolder(): Promise<void> {
    const folder = this.newFolder().trim();
    const current = this.data()?.settings.uploadFolders ?? [];
    if (!folder || current.includes(folder)) return;
    const ok = await this.run('settings', () =>
      this.api.setBrowserSettings({ uploadFolders: [...current, folder] }),
    );
    if (ok) this.newFolder.set('');
  }

  protected removeFolder(folder: string): Promise<boolean> {
    const current = this.data()?.settings.uploadFolders ?? [];
    return this.run('settings', () =>
      this.api.setBrowserSettings({ uploadFolders: current.filter((f) => f !== folder) }),
    );
  }

  protected copy(text: string): void {
    void navigator.clipboard?.writeText(text).then(
      () => this.notices.success(tr('Copied')),
      () => undefined,
    );
  }
}
