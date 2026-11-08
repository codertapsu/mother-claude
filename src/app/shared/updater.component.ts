import { Component, inject } from '@angular/core';

import { TPipe } from '../core/i18n/t.pipe';
import { UpdaterService } from '../core/updater.service';

/** Desktop-only "in-app updates" card: shows the running version, checks the
 * release endpoint, and downloads/installs/relaunches into a new version. */
@Component({
  selector: 'mc-updater',
  imports: [TPipe],
  template: `
    <div class="card updater">
      @if (!u.desktop()) {
        <p class="muted">
          {{
            'In-app updates run in the desktop app. In a browser, download the latest installer from the releases page.'
              | t
          }}
        </p>
      } @else {
        <div class="row top">
          <div class="info">
            <div>
              {{ 'Current version:' | t }} <strong>{{ u.currentVersion() || '…' }}</strong>
            </div>
            @switch (u.status()) {
              @case ('checking') {
                <div class="muted">{{ 'Checking for updates…' | t }}</div>
              }
              @case ('uptodate') {
                <div class="muted">{{ 'You’re on the latest version.' | t }}</div>
              }
              @case ('available') {
                <div class="ok">
                  {{ 'Update available:' | t }} <strong>{{ u.newVersion() }}</strong>
                </div>
              }
              @case ('downloading') {
                <div class="muted">
                  {{
                    u.determinate()
                      ? ('Downloading… {pct}%' | t: { pct: pct() })
                      : ('Downloading…' | t)
                  }}
                </div>
              }
              @case ('installing') {
                <div class="muted">{{ 'Installing…' | t }}</div>
              }
              @case ('ready') {
                <div class="muted">{{ 'Update installed — restarting…' | t }}</div>
              }
              @case ('error') {
                <div class="err">{{ 'Update failed: {error}' | t: { error: u.error() } }}</div>
              }
            }
          </div>
          <span class="spacer"></span>
          @if (u.available()) {
            <button
              class="btn primary"
              [disabled]="u.status() !== 'available'"
              (click)="u.install()"
            >
              {{ u.status() === 'available' ? ('Download & install' | t) : ('Working…' | t) }}
            </button>
          } @else {
            <button class="btn" [disabled]="u.busy()" (click)="u.check()">
              {{ u.status() === 'checking' ? ('Checking…' | t) : ('Check for updates' | t) }}
            </button>
          }
        </div>
        @if (u.status() === 'downloading') {
          <div class="bar" [class.indeterminate]="!u.determinate()">
            <div class="fill" [style.width.%]="u.determinate() ? pct() : 40"></div>
          </div>
        }
        @if (u.notes() && u.status() === 'available') {
          <p class="muted small notes">{{ u.notes() }}</p>
        }
      }
    </div>
  `,
  styles: [
    `
      .top {
        align-items: center;
        gap: 0.75rem;
      }
      .info {
        min-width: 0;
      }
      .ok {
        color: var(--accent);
      }
      .err {
        color: var(--red);
      }
      .notes {
        margin: 0.5rem 0 0;
        white-space: pre-wrap;
      }
      .bar {
        margin-top: 0.6rem;
        height: 6px;
        border-radius: 3px;
        background: var(--border, #2a2a2a);
        overflow: hidden;
      }
      .fill {
        height: 100%;
        background: var(--accent);
        transition: width 0.15s ease;
      }
      .bar.indeterminate .fill {
        width: 40% !important;
        animation: mc-indeterminate 1.1s ease-in-out infinite;
      }
      @keyframes mc-indeterminate {
        0% {
          margin-left: -40%;
        }
        100% {
          margin-left: 100%;
        }
      }
    `,
  ],
})
export class UpdaterComponent {
  protected readonly u = inject(UpdaterService);

  protected pct(): number {
    return Math.round(this.u.progress() * 100);
  }
}
