import { Component, inject } from '@angular/core';

import { ConfigService } from '../../core/config.service';
import { PermissionsService } from '../../core/permissions.service';
import { PermissionsListComponent } from '../../shared/permissions-list.component';
import { LimitationsComponent } from '../../shared/limitations.component';
import { PairingComponent } from '../../shared/pairing.component';
import { UpdaterComponent } from '../../shared/updater.component';

@Component({
  selector: 'mc-settings',
  imports: [PermissionsListComponent, LimitationsComponent, PairingComponent, UpdaterComponent],
  templateUrl: './settings.component.html',
  styleUrl: './settings.component.scss',
})
export class SettingsComponent {
  private config = inject(ConfigService);
  private permissions = inject(PermissionsService);

  protected readonly cfg = this.config.config;
  protected readonly desktop = this.permissions.desktop;

  constructor() {
    void this.permissions.refresh();
  }
}
