import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core';

import { ApiService, describeError } from '../../core/api.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { AutostartView } from '../../core/machines';
import { NoticeService } from '../../core/notice.service';

/**
 * *Start Mother Claude when you sign in, in the tray* (docs/PLAN-MACHINES.md
 * §5.13): a desktop setting, off by default. On, the system starts Mother
 * Claude at sign-in with no window — a tray icon (the menu bar on a Mac) —
 * so your phone and your other computers reach this one after a restart
 * without anyone opening it; *Open Mother Claude* there shows the window.
 * The switch writes the system's own login item (`PUT /api/app/autostart`,
 * the desktop only).
 */
@Component({
  selector: 'mc-startup-setting',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [TPipe],
  template: `
    @if (view(); as v) {
      @if (v.supported) {
        <label class="switch list-row">
          <input
            type="checkbox"
            [checked]="v.enabled && !v.overridden"
            [disabled]="busy()"
            (change)="set($any($event.target).checked)"
          />
          <span class="switch-text">
            <span>{{ 'Start Mother Claude when you sign in, in the tray' | t }}</span>
            <span class="muted small">{{
              'It runs with no window, so your phone and your other computers reach this {computer} after a restart. Open it from the tray (the menu bar on a Mac).'
                | t: { computer: computer() }
            }}</span>
            @if (v.overridden) {
              <span class="small warn-text">{{
                'Switched off in the system’s startup apps: switch it on here to start again.' | t
              }}</span>
            }
            @if (v.startedInTray) {
              <span class="muted small">{{
                'This time it started at sign-in: closing the window keeps it running in the tray.'
                  | t
              }}</span>
            }
          </span>
        </label>
      }
    }
    @if (error()) {
      <p class="error-text small" role="alert">{{ error() }}</p>
    }
  `,
  styles: [
    `
      .warn-text {
        color: var(--amber);
      }
    `,
  ],
})
export class StartupSettingComponent implements OnInit {
  private readonly api = inject(ApiService);
  private readonly notices = inject(NoticeService);

  protected readonly computer = computerNoun;
  protected readonly view = signal<AutostartView | null>(null);
  protected readonly busy = signal(false);
  protected readonly error = signal('');

  ngOnInit(): void {
    void this.load();
  }

  private async load(): Promise<void> {
    try {
      this.view.set(await this.api.getAutostart());
    } catch (e) {
      this.error.set(describeError(e).text);
    }
  }

  protected async set(on: boolean): Promise<void> {
    this.busy.set(true);
    this.error.set('');
    try {
      const v = await this.api.setAutostart(on);
      this.view.set(v);
      this.notices.show(
        'success',
        v.enabled
          ? tr('Mother Claude starts when you sign in, in the tray.')
          : tr('Mother Claude no longer starts when you sign in.'),
      );
    } catch (e) {
      this.error.set(describeError(e).text);
      await this.load();
    } finally {
      this.busy.set(false);
    }
  }
}
