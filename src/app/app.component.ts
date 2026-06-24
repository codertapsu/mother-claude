import { Component, OnInit, computed, inject, signal } from '@angular/core';
import { Router, RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';

import { ConfigService } from './core/config.service';
import { PermissionsService } from './core/permissions.service';
import { RealtimeService } from './core/realtime.service';
import { UpdaterService } from './core/updater.service';
import { LimitationsComponent } from './shared/limitations.component';
import { ModalComponent } from './shared/modal.component';

const ONBOARDING_SKIP_KEY = 'mc_onboarding_skipped';
const WELCOME_SEEN_KEY = 'mc_welcome_seen';

@Component({
  selector: 'mc-root',
  imports: [RouterOutlet, RouterLink, RouterLinkActive, LimitationsComponent, ModalComponent],
  templateUrl: './app.component.html',
  styleUrl: './app.component.scss',
})
export class AppComponent implements OnInit {
  private realtime = inject(RealtimeService);
  private config = inject(ConfigService);
  private permissions = inject(PermissionsService);
  private updater = inject(UpdaterService);
  private router = inject(Router);

  protected readonly connection = this.realtime.connection;
  protected readonly needsToken = computed(() => !this.config.config()?.token);
  protected readonly needsPermission = computed(() => this.permissions.missingRequired());
  protected readonly updateAvailable = this.updater.attentionNeeded;
  protected readonly showWelcome = signal(false);
  protected readonly menuOpen = signal(false);

  async ngOnInit(): Promise<void> {
    await this.config.ensure();
    await this.realtime.start();
    await this.permissions.refresh();

    // Desktop only: silently check for an app update in the background.
    void this.updater.init();

    // First-run: if a required OS permission is missing on desktop and the user
    // hasn't skipped this session, guide them through it.
    const skipped = sessionStorage.getItem(ONBOARDING_SKIP_KEY) === '1';
    const needsOnboarding =
      this.permissions.desktop() && this.permissions.missingRequired() && !skipped;
    if (needsOnboarding) {
      this.router.navigate(['/onboarding']);
    }

    // Otherwise, show the one-time "good to know" welcome on first launch.
    if (!needsOnboarding && localStorage.getItem(WELCOME_SEEN_KEY) !== '1') {
      this.showWelcome.set(true);
    }
  }

  dismissWelcome(): void {
    localStorage.setItem(WELCOME_SEEN_KEY, '1');
    this.showWelcome.set(false);
  }
}
