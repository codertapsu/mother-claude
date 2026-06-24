import { Injectable, computed, inject, signal } from '@angular/core';
import type { DownloadEvent, Update } from '@tauri-apps/plugin-updater';

import { ConfigService } from './config.service';

export type UpdaterStatus =
  | 'idle'
  | 'unsupported'
  | 'checking'
  | 'available'
  | 'uptodate'
  | 'downloading'
  | 'installing'
  | 'ready'
  | 'error';

/**
 * Desktop-only in-app updater. Talks to the Tauri updater/process plugins
 * directly — a sanctioned use of the plugin bridge for an OS-level concern (it
 * is not dashboard data and has no meaning in a phone browser, where every
 * method no-ops and the status is `unsupported`). See docs/AUTOUPDATE.md.
 */
@Injectable({ providedIn: 'root' })
export class UpdaterService {
  private config = inject(ConfigService);

  readonly status = signal<UpdaterStatus>('idle');
  readonly currentVersion = signal('');
  readonly newVersion = signal('');
  readonly notes = signal('');
  readonly error = signal('');
  /** Download progress 0..1 (only meaningful while `downloading` + `determinate`). */
  readonly progress = signal(0);
  /** False when the download has no known total — show an indeterminate bar. */
  readonly determinate = signal(true);

  readonly desktop = computed(() => !!this.config.config()?.desktop);
  /** An update is found and being acted on (drives the Settings install button). */
  readonly available = computed(() =>
    ['available', 'downloading', 'installing', 'ready'].includes(this.status()),
  );
  /** An update needs the user's action (drives the header "⬆ Update" badge). */
  readonly attentionNeeded = computed(() => this.status() === 'available');
  readonly busy = computed(() => ['checking', 'downloading', 'installing'].includes(this.status()));

  private update: Update | null = null;
  private downloaded = 0;
  private total = 0;

  /** Run once on startup (desktop): read the running version + silently check. */
  async init(): Promise<void> {
    if (!this.desktop()) {
      this.status.set('unsupported');
      return;
    }
    try {
      const { getVersion } = await import('@tauri-apps/api/app');
      this.currentVersion.set(await getVersion());
    } catch {
      /* not in the desktop webview */
    }
    await this.check();
  }

  /** Check the release endpoint for a newer version. */
  async check(): Promise<void> {
    if (!this.desktop()) {
      this.status.set('unsupported');
      return;
    }
    this.error.set('');
    this.status.set('checking');
    try {
      const { check } = await import('@tauri-apps/plugin-updater');
      const update = await check();
      await this.dispose(); // free any resource held from a previous check
      if (update?.available) {
        this.update = update;
        this.newVersion.set(update.version);
        this.notes.set(update.body ?? '');
        if (update.currentVersion) this.currentVersion.set(update.currentVersion);
        this.status.set('available');
      } else {
        if (update) await this.close(update);
        this.status.set('uptodate');
      }
    } catch (e) {
      this.status.set('error');
      this.error.set(this.message(e));
    }
  }

  /** Download + install the pending update, then relaunch into it. */
  async install(): Promise<void> {
    if (!this.update) return;
    this.error.set('');
    this.status.set('downloading');
    this.progress.set(0);
    this.determinate.set(true);
    this.downloaded = 0;
    this.total = 0;
    try {
      await this.update.downloadAndInstall((event: DownloadEvent) => {
        if (event.event === 'Started') {
          this.total = event.data.contentLength ?? 0;
          // No total (common for the macOS .app.tar.gz) -> indeterminate bar.
          this.determinate.set(this.total > 0);
        } else if (event.event === 'Progress') {
          this.downloaded += event.data.chunkLength;
          if (this.total > 0) this.progress.set(this.downloaded / this.total);
        } else if (event.event === 'Finished') {
          this.progress.set(1);
          this.determinate.set(true);
          this.status.set('installing');
        }
      });
      // On Windows the app exits during install, so this may not run — that's ok.
      this.status.set('ready');
      const { relaunch } = await import('@tauri-apps/plugin-process');
      await relaunch();
    } catch (e) {
      this.status.set('error');
      this.error.set(this.message(e));
    }
  }

  /** Free any pending update resource (the Update holds a Rust-side handle). */
  private async dispose(): Promise<void> {
    const u = this.update;
    this.update = null;
    if (u) await this.close(u);
  }

  private async close(u: Update): Promise<void> {
    try {
      await u.close();
    } catch {
      /* resource already freed */
    }
  }

  private message(e: unknown): string {
    return e instanceof Error ? e.message : String(e);
  }
}
