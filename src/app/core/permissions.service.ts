import { Injectable, computed, inject, signal } from '@angular/core';

import { ConfigService } from './config.service';
import { PermissionInfo } from './models';

/**
 * macOS permission state for the desktop app. Permissions are a desktop concern
 * (the desktop process reads the protected files); on a phone browser there are
 * none to grant, so everything reports "not applicable". Talks to Tauri `invoke`
 * — the sanctioned use of invoke for OS-level concerns, not dashboard data.
 */
@Injectable({ providedIn: 'root' })
export class PermissionsService {
  private config = inject(ConfigService);

  readonly permissions = signal<PermissionInfo[]>([]);
  readonly appLocation = signal<string>('');
  readonly checked = signal(false);

  readonly desktop = computed(() => !!this.config.config()?.desktop);
  readonly missingRequired = computed(() =>
    this.permissions().some((p) => p.required && p.granted === false),
  );

  /** Re-read the current permission state from the OS. */
  async refresh(): Promise<void> {
    if (!this.config.config()?.desktop) {
      this.permissions.set([]);
      this.checked.set(true);
      return;
    }
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const [perms, loc] = await Promise.all([
        invoke<PermissionInfo[]>('permissions_status'),
        invoke<string>('app_location').catch(() => ''),
      ]);
      this.permissions.set(perms);
      this.appLocation.set(loc);
    } catch {
      this.permissions.set([]);
    }
    this.checked.set(true);
  }

  async openSettings(paneId: string): Promise<void> {
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('open_settings_pane', { paneId });
    } catch {
      /* not in desktop */
    }
  }

  async revealApp(): Promise<void> {
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('reveal_app_in_finder');
    } catch {
      /* not in desktop */
    }
  }
}
