import { Component, computed, effect, inject, signal } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { NavigationEnd, Router, RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';
import { filter, map } from 'rxjs';

import { ApiService } from './core/api.service';
import { AttentionService } from './core/attention.service';
import { AuthService } from './core/auth.service';
import { ConfigService } from './core/config.service';
import { PermissionsService } from './core/permissions.service';
import { sessionStorageGet, storageGet, storageSet } from './core/platform';
import { PushService } from './core/push.service';
import { RealtimeService, connectionLabel } from './core/realtime.service';
import { ThemeService } from './core/theme.service';
import { UpdaterService } from './core/updater.service';
import { IconComponent, IconName } from './shared/icon.component';
import { LimitationsComponent } from './shared/limitations.component';
import { ModalComponent } from './shared/modal.component';

const ONBOARDING_SKIP_KEY = 'mc_onboarding_skipped';
const WELCOME_SEEN_KEY = 'mc_welcome_seen';

/** Sections reached through the phone's "More" tab. */
const MORE_SECTIONS = [
  'more',
  'new',
  'remote',
  'api',
  'settings',
  'services',
  'good-to-know',
  'onboarding',
];

const TITLES: Record<string, string> = {
  '': 'Sessions',
  inbox: 'Inbox',
  agents: 'Agents',
  more: 'More',
  new: 'New session',
  remote: 'Remote',
  api: 'API',
  settings: 'Settings',
  services: 'Services',
  'good-to-know': 'Good to know',
  onboarding: 'Welcome',
  pair: 'Pair this device',
  session: 'Session',
};

interface NavItem {
  path: string;
  label: string;
  icon: IconName;
  exact?: boolean;
  badge?: 'pending';
  desktopOnly?: boolean;
}

/**
 * The app shell: a phone layout (< 768 px: top bar with the connection dot +
 * bottom tabs Sessions · Inbox · Agents · More) and a desktop layout (left
 * nav). Also owns startup: config → pairing state → realtime → push, the
 * redirect to /pair when this device is not (or no longer) paired, and the
 * desktop-only onboarding/updater bits.
 */
@Component({
  selector: 'mc-root',
  imports: [
    RouterOutlet,
    RouterLink,
    RouterLinkActive,
    IconComponent,
    LimitationsComponent,
    ModalComponent,
  ],
  templateUrl: './app.component.html',
  styleUrl: './app.component.scss',
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
  protected readonly theme = inject(ThemeService);
  protected readonly attention = inject(AttentionService);

  protected readonly ready = signal(false);
  protected readonly connection = this.realtime.connection;
  protected readonly pendingCount = this.attention.pendingCount;
  protected readonly interstitial = this.api.interstitial;
  protected readonly desktopRejected = this.auth.desktopRejected;
  protected readonly needsPermission = computed(() => this.permissions.missingRequired());
  protected readonly updateAvailable = this.updater.attentionNeeded;
  protected readonly isDesktop = this.auth.isDesktop;
  protected readonly mock = this.config.mock;
  protected readonly showWelcome = signal(false);

  /** Hide the navigation until this device is paired. */
  protected readonly chrome = computed(() => this.ready() && this.auth.state() !== 'unpaired');

  private readonly url = toSignal(
    this.router.events.pipe(
      filter((e): e is NavigationEnd => e instanceof NavigationEnd),
      map((e) => e.urlAfterRedirects),
    ),
    { initialValue: this.router.url },
  );
  private readonly section = computed(() => {
    const path = (this.url() ?? '/').split(/[?#]/)[0].replace(/^\//, '');
    return path.split('/')[0] ?? '';
  });
  protected readonly pageTitle = computed(() => TITLES[this.section()] ?? 'Mother Claude');
  protected readonly moreActive = computed(() => MORE_SECTIONS.includes(this.section()));

  protected readonly nav: NavItem[] = [
    { path: '/', label: 'Sessions', icon: 'sessions', exact: true },
    { path: '/inbox', label: 'Inbox', icon: 'inbox', badge: 'pending' },
    { path: '/agents', label: 'Agents', icon: 'agents' },
    { path: '/new', label: 'New session', icon: 'plus' },
    { path: '/remote', label: 'Remote', icon: 'phone', desktopOnly: true },
    { path: '/api', label: 'API', icon: 'code' },
    { path: '/settings', label: 'Settings', icon: 'settings' },
  ];
  protected readonly desktopNav = computed(() =>
    this.nav.filter((n) => !n.desktopOnly || this.isDesktop()),
  );

  protected readonly connLabel = computed(() => connectionLabel(this.connection()));

  private lastState: string | null = null;

  constructor() {
    void this.boot();

    // Pairing state drives the socket and the /pair redirect.
    effect(() => {
      if (!this.ready()) return;
      const state = this.auth.state();
      const prev = this.lastState;
      this.lastState = state;
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
    const order = ['system', 'dark', 'light'] as const;
    const next = order[(order.indexOf(this.theme.pref()) + 1) % order.length];
    this.theme.set(next);
  }

  protected themeIcon(): IconName {
    const p = this.theme.pref();
    return p === 'system' ? 'monitor' : p === 'dark' ? 'moon' : 'sun';
  }

  protected reload(): void {
    location.reload();
  }
}
