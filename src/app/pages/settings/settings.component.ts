import { Component, computed, effect, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

import { ApiService, errorMessage } from '../../core/api.service';
import { AttentionService } from '../../core/attention.service';
import { AuthService } from '../../core/auth.service';
import { ConfigService } from '../../core/config.service';
import { PushPrefs } from '../../core/models';
import { PermissionsService } from '../../core/permissions.service';
import { PushService } from '../../core/push.service';
import { RealtimeService, connectionLabel } from '../../core/realtime.service';
import { ThemePref, ThemeService } from '../../core/theme.service';
import { UpdaterService } from '../../core/updater.service';
import { IconComponent } from '../../shared/icon.component';
import { PermissionsListComponent } from '../../shared/permissions-list.component';
import { UpdaterComponent } from '../../shared/updater.component';

type PrefKey = 'permission' | 'question' | 'plan' | 'finished' | 'failed' | 'claude';

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

  protected readonly ingress = computed(() => {
    const i = this.me()?.ingress;
    return i ? (INGRESS[i] ?? i) : '—';
  });
  protected readonly secure = window.isSecureContext;

  constructor() {
    void this.permissions.refresh();
    void this.push.refresh();
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
    const next = { ...cur, [key]: on };
    this.prefs.set(next);
    this.prefsError.set('');
    try {
      const saved = await this.api.setPushPrefs(next);
      if (saved) this.prefs.set(saved);
    } catch (e) {
      this.prefs.set(cur);
      this.prefsError.set(errorMessage(e));
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

  protected forget(): void {
    if (!this.confirmForget()) {
      this.confirmForget.set(true);
      setTimeout(() => this.confirmForget.set(false), 4000);
      return;
    }
    this.auth.forget();
  }

  private async loadPrefs(): Promise<void> {
    try {
      this.prefs.set(await this.api.getPushPrefs());
    } catch (e) {
      this.prefsError.set(errorMessage(e));
    }
  }
}
