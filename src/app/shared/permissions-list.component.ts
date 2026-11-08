import { Component, inject } from '@angular/core';

import { TPipe } from '../core/i18n/t.pipe';
import { PermissionsService } from '../core/permissions.service';

/** Reusable, self-contained list of OS permissions with guidance + actions.
 * The permissions are macOS's (System Settings, Finder): the desktop
 * reports none on other systems. Their names and steps are the server's. */
@Component({
  selector: 'mc-permissions-list',
  imports: [TPipe],
  template: `
    @if (!perms.desktop()) {
      <p class="muted">{{ 'Permissions are managed by the desktop app.' | t }}</p>
    } @else {
      @for (p of perms.permissions(); track p.id) {
        <div class="card perm">
          <div class="row head">
            <strong>{{ p.label }}</strong>
            @if (p.required) {
              <span class="badge req">{{ 'required' | t }}</span>
            } @else {
              <span class="badge">{{ 'optional' | t }}</span>
            }
            <span class="spacer"></span>
            @if (p.granted === true) {
              <span class="badge working">{{ 'granted' | t }}</span>
            } @else if (p.granted === false) {
              <span class="badge failed">{{ 'not granted' | t }}</span>
            } @else {
              <span class="badge">{{ 'ask when prompted' | t }}</span>
            }
          </div>
          <p class="desc">{{ p.description }}</p>

          @if (p.granted !== true) {
            <ol class="steps">
              @for (step of p.steps; track step) {
                <li>{{ step }}</li>
              }
            </ol>
            @if (p.id === 'full-disk-access' && perms.appLocation()) {
              <p class="muted small mono">
                {{ 'Add this app: {path}' | t: { path: perms.appLocation() } }}
              </p>
            }
            <div class="row">
              @if (p.settingsUrl) {
                <button class="btn primary" (click)="perms.openSettings(p.id)">
                  {{ 'Open System Settings' | t }}
                </button>
              }
              @if (p.id === 'full-disk-access') {
                <button class="btn" (click)="perms.revealApp()">
                  {{ 'Reveal app in Finder' | t }}
                </button>
              }
            </div>
          }
        </div>
      }
      <div class="row recheck">
        <button class="btn" (click)="perms.refresh()">{{ 'Re-check' | t }}</button>
        @if (perms.missingRequired()) {
          <span class="muted small">{{ 'Grant the required permission, then re-check.' | t }}</span>
        } @else {
          <span class="muted small">{{ 'All required permissions granted.' | t }}</span>
        }
      </div>
    }
  `,
  styles: [
    `
      .perm {
        margin-bottom: 0.6rem;
      }
      .head {
        align-items: center;
      }
      .desc {
        margin: 0.4rem 0;
      }
      .steps {
        margin: 0.4rem 0 0.6rem;
        padding-left: 1.2rem;
        line-height: 1.5;
      }
      .small {
        font-size: 0.8rem;
      }
      .mono {
        word-break: break-all;
      }
      .badge.req {
        color: var(--accent);
        border-color: var(--accent);
      }
      .recheck {
        margin-top: 0.5rem;
        align-items: center;
      }
    `,
  ],
})
export class PermissionsListComponent {
  protected readonly perms = inject(PermissionsService);
}
