import { Component, OnInit, computed, inject } from '@angular/core';
import { Router, RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';

import { ConfigService } from './core/config.service';
import { PermissionsService } from './core/permissions.service';
import { RealtimeService } from './core/realtime.service';

const ONBOARDING_SKIP_KEY = 'mc_onboarding_skipped';

@Component({
  selector: 'mc-root',
  imports: [RouterOutlet, RouterLink, RouterLinkActive],
  templateUrl: './app.component.html',
  styleUrl: './app.component.scss',
})
export class AppComponent implements OnInit {
  private realtime = inject(RealtimeService);
  private config = inject(ConfigService);
  private permissions = inject(PermissionsService);
  private router = inject(Router);

  protected readonly connection = this.realtime.connection;
  protected readonly needsToken = computed(() => !this.config.config()?.token);
  protected readonly needsPermission = computed(() => this.permissions.missingRequired());

  async ngOnInit(): Promise<void> {
    await this.config.ensure();
    await this.realtime.start();
    await this.permissions.refresh();

    // First-run: if a required OS permission is missing on desktop and the user
    // hasn't skipped this session, guide them through it.
    const skipped = sessionStorage.getItem(ONBOARDING_SKIP_KEY) === '1';
    if (this.permissions.desktop() && this.permissions.missingRequired() && !skipped) {
      this.router.navigate(['/onboarding']);
    }
  }
}
