import { Component, computed, effect, inject, signal, untracked } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { NavigationEnd, Router, RouterLink, RouterOutlet } from '@angular/router';
import { filter, map } from 'rxjs';

import { ApiService } from './core/api.service';
import { PendingPair } from './core/models';
import { NoticeService } from './core/notice.service';
import { AttentionService } from './core/attention.service';
import { AuthService } from './core/auth.service';
import { ConfigService } from './core/config.service';
import { PermissionsService } from './core/permissions.service';
import { sessionStorageGet, storageGet, storageSet } from './core/platform';
import { PairRequestsService, isStillPending } from './core/pair-requests.service';
import { PushService } from './core/push.service';
import { RealtimeService, connectionDetail, connectionLabel } from './core/realtime.service';
import { I18nService } from './core/i18n/i18n.service';
import { JumpService, applePlatform, isJumpKey, jumpShortcut } from './core/jump';
import { ListDetailService, listSection } from './core/list-detail';
import { ProjectSettingsService } from './core/project-settings';
import {
  HELP_ENTRY,
  MORE_ENTRY,
  NavEntry,
  barPlaces,
  moreActive,
  sectionOf,
  sidebarGroups,
  titleOf,
} from './core/nav';
import { NavBadges } from './core/nav-badges';
import { tr } from './core/i18n/i18n';
import { TPipe } from './core/i18n/t.pipe';
import { ThemeService } from './core/theme.service';
import { UpdaterService } from './core/updater.service';
import { ViewportService } from './core/viewport.service';
import { IconComponent, IconName } from './shared/icon.component';
import { LimitationsComponent } from './shared/limitations.component';
import { FolderChoiceSheetComponent } from './shared/folder-choice-sheet.component';
import { PairApprovalDialogComponent } from './shared/pair-approval-dialog.component';
import { JumpToComponent } from './pages/jump/jump-to.component';
import { ProjectSettingsSheetComponent } from './pages/project-settings/project-settings-sheet.component';
import { SheetComponent } from './shared/sheet.component';
import { TimeAgoComponent } from './shared/time-ago.component';
import { ToastComponent } from './shared/toast.component';

const ONBOARDING_SKIP_KEY = 'mc_onboarding_skipped';
const WELCOME_SEEN_KEY = 'mc_welcome_seen';

/**
 * The app shell: a phone layout (< 768 px: top bar with the connection dot +
 * the bottom bar's tabs and More) and a desktop layout (left nav), both
 * drawn from the map (core/nav.ts), with *Jump to…* (⌘K or Ctrl+K, the
 * sidebar's button, the phone's search button). Also owns startup: config → pairing
 * state → realtime → push, the redirect to /pair when this device is not
 * (or no longer) paired, and the desktop-only onboarding/updater bits.
 */
@Component({
  selector: 'mc-root',
  imports: [
    RouterOutlet,
    RouterLink,
    IconComponent,
    JumpToComponent,
    LimitationsComponent,
    FolderChoiceSheetComponent,
    PairApprovalDialogComponent,
    ProjectSettingsSheetComponent,
    SheetComponent,
    TimeAgoComponent,
    ToastComponent,
    TPipe,
  ],
  templateUrl: './app.component.html',
  styleUrl: './app.component.scss',
  host: { '(document:keydown)': 'onKey($event)' },
})
export class AppComponent {
  private realtime = inject(RealtimeService);
  private config = inject(ConfigService);
  private auth = inject(AuthService);
  private api = inject(ApiService);
  private permissions = inject(PermissionsService);
  private updater = inject(UpdaterService);
  private push = inject(PushService);
  private router = inject(Router);
  private notices = inject(NoticeService);
  /** Keeps --vvh/--vv-top in sync from startup. */
  private viewport = inject(ViewportService);
  protected readonly theme = inject(ThemeService);
  /** Started here: the language follows the desktop's system name too. */
  private readonly i18n = inject(I18nService);
  protected readonly attention = inject(AttentionService);
  /** Devices waiting to pair (desktop): the dialog's queue follows it. */
  private readonly pairs = inject(PairRequestsService);
  /** What the map's badges show. */
  protected readonly badges = inject(NavBadges);
  /** A session beside its list (wide screens). */
  private readonly listDetail = inject(ListDetailService);
  /** A project's settings, opened from anywhere: the sheet loads (its own
   * chunk) the first time. */
  protected readonly projectSettings = inject(ProjectSettingsService);
  /** *Jump to…*: loaded (its own chunk) the first time it opens. */
  protected readonly jump = inject(JumpService);
  private readonly apple = applePlatform();
  /** The key that opens it, as the sidebar's button shows it. */
  protected readonly jumpKey = jumpShortcut(this.apple);

  protected readonly ready = signal(false);
  protected readonly connection = this.realtime.connection;
  protected readonly interstitial = this.api.interstitial;
  protected readonly desktopRejected = this.auth.desktopRejected;
  protected readonly needsPermission = computed(() => this.permissions.missingRequired());
  protected readonly updateAvailable = this.updater.attentionNeeded;
  protected readonly isDesktop = this.auth.isDesktop;
  protected readonly mock = this.config.mock;
  protected readonly showWelcome = signal(false);
  protected readonly lastData = this.realtime.lastDataAt;
  /** Desktop: pairing requests waiting for the typed-number approval. */
  private readonly pairQueue = signal<PendingPair[]>([]);
  protected readonly pairRequest = computed(() => this.pairQueue()[0] ?? null);

  /** Hide the navigation until this device is paired. */
  protected readonly chrome = computed(() => this.ready() && this.auth.state() !== 'unpaired');

  private readonly url = toSignal(
    this.router.events.pipe(
      filter((e): e is NavigationEnd => e instanceof NavigationEnd),
      map((e) => e.urlAfterRedirects),
    ),
    { initialValue: this.router.url },
  );
  private readonly section = computed(() => sectionOf(this.url() ?? '/'));
  protected readonly pageTitle = computed(() => tr(titleOf(this.section())));
  /** The session page owns the whole phone screen (its own header). */
  protected readonly immersive = computed(() => this.section() === 'session');
  /** The section the sidebar marks: beside its list, a session belongs to
   * that list's place (the Inbox's, when opened from it). */
  private readonly navSection = computed(() =>
    this.listDetail.split() ? listSection(this.listDetail.beside()) : this.section(),
  );

  // The map (core/nav.ts): the sidebar's groups, the phone's bar and its
  // More tab.
  protected readonly sidebar = computed(() => sidebarGroups(this.isDesktop()));
  protected readonly bar = barPlaces();
  protected readonly helpEntry = HELP_ENTRY;
  protected readonly moreEntry = MORE_ENTRY;
  protected readonly moreActive = computed(() => moreActive(this.section()));

  protected readonly connLabel = computed(() => connectionLabel(this.connection()));
  protected readonly connDetail = computed(() => connectionDetail(this.connection()));

  private lastState: string | null = null;
  private lastToken = '';

  constructor() {
    void this.boot();

    // Server notices become toasts.
    this.realtime.notice$.subscribe((n) => {
      const level =
        n.level === 'error'
          ? 'error'
          : n.level === 'warn' || n.level === 'warning'
            ? 'warning'
            : n.level === 'success'
              ? 'success'
              : 'info';
      this.notices.show(level, n.text);
    });
    // Desktop: a device asked to pair (on any page: the Inbox lists it too).
    this.realtime.pairRequest$.subscribe((f) => {
      if (!f.pending?.id || !this.isDesktop()) return;
      if (!isStillPending(f.pending, Date.now())) return;
      this.pairQueue.update((q) => (q.some((p) => p.id === f.pending.id) ? q : [...q, f.pending]));
    });
    // Requests settle elsewhere too (approved or denied in the Inbox or in
    // another window, expired): keep only those still waiting.
    effect(() => {
      const open = new Set(this.pairs.pending().map((p) => p.id));
      untracked(() =>
        this.pairQueue.update((q) => {
          const next = q.filter((p) => open.has(p.id));
          return next.length === q.length ? q : next;
        }),
      );
    });
    // Say connection changes out loud (the dot alone says nothing to a
    // screen reader).
    let lastConn = '';
    effect(() => {
      const c = this.connection();
      if (lastConn && c !== lastConn && c !== 'connecting') {
        this.notices.announce(tr('Connection: {state}', { state: connectionLabel(c) }));
      }
      lastConn = c;
    });

    // What waits for you, for the Inbox's badge and dot (Machines' health
    // for the Computers dot too), once this device is paired.
    effect(() => {
      if (this.ready() && this.auth.state() === 'paired') {
        untracked(() => this.attention.start());
      }
    });

    // Pairing state drives the socket and the /pair redirect.
    effect(() => {
      if (!this.ready()) return;
      const state = this.auth.state();
      const token = this.config.token();
      const prev = this.lastState;
      const prevToken = this.lastToken;
      this.lastState = state;
      this.lastToken = token;
      if (state === 'paired' && prev === 'paired' && prevToken && token !== prevToken) {
        // Paired again while paired: the socket and the push subscription
        // still belong to the old device.
        this.realtime.restart();
        void this.loadMe();
        void this.push.resubscribe();
        return;
      }
      if (state === 'unpaired') {
        this.realtime.stop();
        // Read the address bar, not router.url: during the initial navigation
        // the router still reports '/', and redirecting would drop `?c=`.
        const onPair = location.hash.startsWith('#/pair');
        if (!onPair) {
          const revoked = this.auth.reason() === 'revoked';
          void this.router.navigate(
            ['/pair'],
            revoked ? { queryParams: { reason: 'revoked' } } : {},
          );
        }
      } else if (state === 'paired' && prev === 'unpaired') {
        // Just paired: connect with the new token.
        this.realtime.restart();
        void this.loadMe();
      }
    });
  }

  private async boot(): Promise<void> {
    await this.config.ensure();
    await this.auth.init();
    this.ready.set(true);
    if (this.auth.state() === 'paired') {
      void this.loadMe();
      await this.realtime.start();
    }
    void this.push.register();

    await this.permissions.refresh();
    // Desktop only: silently check for an app update in the background.
    void this.updater.init();

    // First run on desktop: guide through a missing required OS permission.
    const skipped = sessionStorageGet(ONBOARDING_SKIP_KEY) === '1';
    const needsOnboarding =
      this.permissions.desktop() && this.permissions.missingRequired() && !skipped;
    if (needsOnboarding) {
      void this.router.navigate(['/onboarding']);
    } else if (this.chrome() && !this.mock() && storageGet(WELCOME_SEEN_KEY) !== '1') {
      this.showWelcome.set(true);
    }
  }

  private async loadMe(): Promise<void> {
    try {
      this.auth.me.set(await this.api.getMe());
    } catch {
      // The WS hello carries `me` too; a 401 here already flips to unpaired.
    }
  }

  protected dismissWelcome(): void {
    storageSet(WELCOME_SEEN_KEY, '1');
    this.showWelcome.set(false);
  }

  protected cycleTheme(): void {
    const order = ['dark', 'light', 'system'] as const;
    const next = order[(order.indexOf(this.theme.pref()) + 1) % order.length];
    this.theme.set(next);
  }

  /** The theme in words, for the toggle's label. */
  protected themeName(): string {
    const p = this.theme.pref();
    return p === 'dark' ? tr('Dark') : p === 'light' ? tr('Light') : tr('System||theme');
  }

  /** The entry the page shown belongs to. */
  protected isCurrent(entry: NavEntry): boolean {
    return entry.sections.includes(this.navSection());
  }

  protected themeIcon(): IconName {
    const p = this.theme.pref();
    return p === 'system' ? 'monitor' : p === 'dark' ? 'moon' : 'sun';
  }

  protected reload(): void {
    location.reload();
  }

  protected nextPairRequest(): void {
    this.pairQueue.update((q) => q.slice(1));
  }

  protected reconnect(): void {
    this.realtime.reconnectNow();
  }

  /** ⌘K (Ctrl+K off Apple systems) opens *Jump to…* anywhere, and closes
   * it again. */
  protected onKey(e: KeyboardEvent): void {
    if (!this.chrome() || e.defaultPrevented || !isJumpKey(e, this.apple)) return;
    e.preventDefault();
    this.jump.toggle();
  }
}
