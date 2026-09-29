import { Component, computed, effect, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiService, errorMessage } from '../../core/api.service';
import { AttentionService } from '../../core/attention.service';
import { AuthService } from '../../core/auth.service';
import { ConfigService } from '../../core/config.service';
import { DeviceService } from '../../core/device.service';
import { PairCode, PushPrefs } from '../../core/models';
import { PermissionsService } from '../../core/permissions.service';
import { PushService } from '../../core/push.service';
import { RealtimeService, connectionLabel } from '../../core/realtime.service';
import { ThemePref, ThemeService } from '../../core/theme.service';
import { UpdaterService } from '../../core/updater.service';
import { IconComponent } from '../../shared/icon.component';
import { PermissionsListComponent } from '../../shared/permissions-list.component';
import { UpdaterComponent } from '../../shared/updater.component';

type PrefKey =
  'permission' | 'question' | 'plan' | 'finished' | 'failed' | 'claude' | 'idle' | 'showDetails';

const PREF_ROWS: { key: PrefKey; label: string; hint: string }[] = [
  { key: 'permission', label: 'Permission requests', hint: 'A tool call waits for approval' },
  { key: 'question', label: 'Questions', hint: 'Claude asks you something' },
  { key: 'plan', label: 'Plans', hint: 'A plan is ready for review' },
  { key: 'finished', label: 'Finished', hint: 'A longer turn (30 s+) completes' },
  { key: 'failed', label: 'Failures', hint: 'A session fails or hits an API error' },
  {
    key: 'claude',
    label: 'When Claude asks',
    hint: 'Claude sends a notification itself (e.g. “tell me when the tests finish”)',
  },
  {
    key: 'idle',
    label: 'Waiting for you',
    hint: 'A session sits idle, waiting for your next prompt',
  },
  {
    key: 'showDetails',
    label: 'Show details',
    hint: 'Include the command or question in the notification (off: only the session and what happened)',
  },
];

const INGRESS: Record<string, string> = {
  loopback: 'This computer',
  lan: 'Local network',
  tunnel: 'Internet (tunnel)',
  'bridge-loopback': 'This computer (API port)',
};

/** Notifications, theme, connection details and about. */
@Component({
  selector: 'mc-settings',
  imports: [RouterLink, IconComponent, PermissionsListComponent, UpdaterComponent],
  templateUrl: './settings.component.html',
  styleUrl: './settings.component.scss',
})
export class SettingsComponent {
  private api = inject(ApiService);
  private auth = inject(AuthService);
  private config = inject(ConfigService);
  private device = inject(DeviceService);
  private permissions = inject(PermissionsService);
  private realtime = inject(RealtimeService);
  protected readonly push = inject(PushService);
  protected readonly theme = inject(ThemeService);
  protected readonly attention = inject(AttentionService);
  protected readonly updater = inject(UpdaterService);

  protected readonly cfg = this.config.config;
  protected readonly me = this.auth.me;
  protected readonly isDesktop = this.auth.isDesktop;
  protected readonly desktopApp = this.permissions.desktop;
  protected readonly connection = computed(() => connectionLabel(this.realtime.connection()));
  protected readonly prefRows = PREF_ROWS;
  protected readonly prefs = signal<PushPrefs | null>(null);
  protected readonly prefsError = signal('');
  protected readonly testSent = signal('');
  protected readonly confirmForget = signal(false);
  protected readonly forgetting = signal(false);

  // Home Screen app (a device pairs its second app with a `self` code).
  protected readonly selfCode = signal<PairCode | null>(null);
  protected readonly selfCodeError = signal('');
  protected readonly selfCodeBusy = signal(false);
  protected readonly isDevice = computed(() => this.me()?.principal === 'device');
  protected readonly dangerousLabel = computed(() => {
    const m = this.me();
    if (!m) return this.cfg()?.desktop ? 'Allowed (this computer)' : '—';
    if (m.principal === 'desktop') return 'Allowed (this computer)';
    if (m.allowDangerous) return 'Allowed for this device on this network';
    return 'Only from the Mac';
  });

  protected readonly ingress = computed(() => {
    // A tunnel pointed at the main port arrives on the loopback listener:
    // `effectiveIngress` is the network the request really came from.
    const m = this.me();
    const i = m?.effectiveIngress ?? m?.ingress;
    return i ? (INGRESS[i] ?? i) : '—';
  });
  protected readonly secure = window.isSecureContext;

  constructor() {
    void this.permissions.refresh();
    void this.push.refresh();
    if (!this.isDesktop()) void this.push.loadHint();
    // Per-event preferences exist once this device has push on.
    effect(() => {
      if (this.push.state() === 'on' && !this.prefs()) void this.loadPrefs();
    });
  }

  protected setTheme(p: ThemePref): void {
    this.theme.set(p);
  }

  protected async togglePref(key: PrefKey, on: boolean): Promise<void> {
    const cur = this.prefs();
    if (!cur) return;
    this.prefs.set({ ...cur, [key]: on });
    this.prefsError.set('');
    try {
      // Only the changed field: the server merges it into the stored prefs.
      const saved = await this.api.setPushPrefs({ [key]: on });
      if (saved) this.prefs.set(saved);
    } catch (e) {
      this.prefs.set(cur);
      this.prefsError.set(errorMessage(e));
    }
  }

  /** A one-time code for this device's iOS Home Screen app. */
  protected async homeScreenCode(): Promise<void> {
    this.selfCodeBusy.set(true);
    this.selfCodeError.set('');
    try {
      this.selfCode.set(await this.api.createPairCode({ target: 'self' }));
    } catch (e) {
      this.selfCode.set(null);
      this.selfCodeError.set(errorMessage(e));
    } finally {
      this.selfCodeBusy.set(false);
    }
  }

  protected async test(): Promise<void> {
    this.testSent.set('');
    try {
      await this.api.pushTest();
      this.testSent.set('Test notification sent.');
    } catch (e) {
      this.testSent.set(`Could not send: ${errorMessage(e)}`);
    }
  }

  protected reconnect(): void {
    this.realtime.restart();
  }

  protected async forget(): Promise<void> {
    if (!this.confirmForget()) {
      this.confirmForget.set(true);
      setTimeout(() => this.confirmForget.set(false), 4000);
      return;
    }
    this.forgetting.set(true);
    // Unpair on the server too, so the token stops working everywhere; forget
    // it locally even if the Mac can't be reached right now.
    await this.device.forgetThisDevice();
    this.forgetting.set(false);
  }

  private async loadPrefs(): Promise<void> {
    try {
      this.prefs.set(await this.api.getPushPrefs());
    } catch (e) {
      this.prefsError.set(errorMessage(e));
    }
  }
}
