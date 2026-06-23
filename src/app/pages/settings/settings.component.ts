import { Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DomSanitizer, SafeHtml } from '@angular/platform-browser';

import { ApiService } from '../../core/api.service';
import { ConfigService } from '../../core/config.service';
import { PermissionsService } from '../../core/permissions.service';
import { Pairing } from '../../core/models';
import { PermissionsListComponent } from '../../shared/permissions-list.component';

@Component({
  selector: 'mc-settings',
  imports: [FormsModule, PermissionsListComponent],
  templateUrl: './settings.component.html',
  styleUrl: './settings.component.scss',
})
export class SettingsComponent {
  private api = inject(ApiService);
  private config = inject(ConfigService);
  private permissions = inject(PermissionsService);
  private sanitizer = inject(DomSanitizer);

  protected readonly cfg = this.config.config;
  protected readonly desktop = this.permissions.desktop;
  protected readonly pairing = signal<Pairing | null>(null);
  protected readonly qr = signal<SafeHtml | null>(null);
  protected readonly tokenInput = signal('');
  protected readonly error = signal('');

  constructor() {
    this.loadPairing();
    void this.permissions.refresh();
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
}
