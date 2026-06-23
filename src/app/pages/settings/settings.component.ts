import { Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DomSanitizer, SafeHtml } from '@angular/platform-browser';

import { ApiService } from '../../core/api.service';
import { ConfigService } from '../../core/config.service';
import { Pairing } from '../../core/models';

const EXPERIMENTAL_KEY = 'mc_experimental';

@Component({
  selector: 'mc-settings',
  imports: [FormsModule],
  templateUrl: './settings.component.html',
  styleUrl: './settings.component.scss',
})
export class SettingsComponent {
  private api = inject(ApiService);
  private config = inject(ConfigService);
  private sanitizer = inject(DomSanitizer);

  protected readonly cfg = this.config.config;
  protected readonly pairing = signal<Pairing | null>(null);
  protected readonly qr = signal<SafeHtml | null>(null);
  protected readonly tokenInput = signal('');
  protected readonly error = signal('');
  protected readonly experimental = signal(localStorage.getItem(EXPERIMENTAL_KEY) === '1');
  protected readonly fda = signal<boolean | null>(null);

  constructor() {
    this.loadPairing();
    this.checkFda();
  }

  private async loadPairing(): Promise<void> {
    try {
      const p = await this.api.getPairing();
      this.pairing.set(p);
      this.qr.set(this.sanitizer.bypassSecurityTrustHtml(p.svg));
    } catch (e) {
      this.error.set(`Pairing unavailable (need a valid token first): ${(e as Error).message}`);
    }
  }

  applyToken(): void {
    const t = this.tokenInput().trim();
    if (!t) return;
    this.config.setToken(t);
    this.tokenInput.set('');
    // Reload so the new token takes effect everywhere (WS reconnects).
    location.reload();
  }

  toggleExperimental(value: boolean): void {
    this.experimental.set(value);
    localStorage.setItem(EXPERIMENTAL_KEY, value ? '1' : '0');
  }

  private async checkFda(): Promise<void> {
    if (!this.config.config()?.desktop) return;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      this.fda.set(await invoke<boolean>('check_full_disk_access'));
    } catch {
      this.fda.set(null);
    }
  }

  async openPrivacy(): Promise<void> {
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('open_privacy_settings');
    } catch {
      /* not in desktop */
    }
  }
}
