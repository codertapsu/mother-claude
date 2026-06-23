import { Component, OnDestroy, OnInit, inject } from '@angular/core';
import { Router } from '@angular/router';

import { PermissionsService } from '../../core/permissions.service';
import { PermissionsListComponent } from '../../shared/permissions-list.component';

const SKIP_KEY = 'mc_onboarding_skipped';

/** First-run guide that walks the user through granting required permissions. */
@Component({
  selector: 'mc-onboarding',
  imports: [PermissionsListComponent],
  template: `
    <div class="wrap">
      <h1>Welcome to Mother&nbsp;Claude</h1>
      <p class="lead">
        One quick step: macOS needs your OK before the app can read your Claude Code sessions and be
        reachable from your phone. Grant the permission below — it takes about 20 seconds.
      </p>

      <mc-permissions-list />

      <div class="row footer">
        <span class="spacer"></span>
        @if (perms.missingRequired()) {
          <button class="btn" (click)="skip()">Skip for now</button>
          <button class="btn" disabled title="Grant the required permission first">
            Continue
          </button>
        } @else {
          <button class="btn primary" (click)="continue()">Continue to dashboard →</button>
        }
      </div>
      <p class="muted small">
        You can revisit this anytime under <strong>Settings → Permissions</strong>.
      </p>
    </div>
  `,
  styles: [
    `
      .wrap {
        max-width: 44rem;
        margin: 1rem auto;
      }
      .lead {
        color: var(--muted);
        line-height: 1.5;
        margin-bottom: 1rem;
      }
      .footer {
        margin-top: 1rem;
      }
      .small {
        font-size: 0.8rem;
        margin-top: 0.5rem;
      }
    `,
  ],
})
export class OnboardingComponent implements OnInit, OnDestroy {
  protected readonly perms = inject(PermissionsService);
  private router = inject(Router);
  private poll?: ReturnType<typeof setInterval>;

  async ngOnInit(): Promise<void> {
    await this.perms.refresh();
    // While the user is in System Settings, keep the status fresh so it flips to
    // "granted" the moment they return.
    this.poll = setInterval(() => void this.perms.refresh(), 2500);
  }

  ngOnDestroy(): void {
    if (this.poll) clearInterval(this.poll);
  }

  skip(): void {
    sessionStorage.setItem(SKIP_KEY, '1');
    this.router.navigate(['/']);
  }

  continue(): void {
    sessionStorage.removeItem(SKIP_KEY);
    this.router.navigate(['/']);
  }
}
