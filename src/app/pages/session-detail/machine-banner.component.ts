import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';

import { tr } from '../../core/i18n/i18n';
import { MachineContext } from '../../core/machine-context';
import { IconComponent } from '../../shared/icon.component';
import { pastClock } from '../../shared/util';

/**
 * Under another computer's session header: where it runs, and — while that
 * computer is offline — that what shows is from then and sending waits
 * (docs/PLAN-MACHINES.md §6.2).
 */
@Component({
  selector: 'mc-machine-banner',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent],
  host: { '[class.off]': '!m.online()' },
  template: `
    <p class="line" role="status">
      <mc-icon name="monitor" [size]="15" />
      <span>{{ text() }}</span>
    </p>
  `,
  styles: [
    `
      :host {
        display: block;
        padding: 0.4rem max(0.75rem, env(safe-area-inset-right)) 0.4rem
          max(0.75rem, env(safe-area-inset-left));
        background: var(--panel);
        border-bottom: 1px solid var(--border);
        font-size: 0.85rem;
        color: var(--muted);
      }
      :host(.off) {
        background: var(--amber-soft);
        color: var(--amber);
      }
      .line {
        display: flex;
        align-items: center;
        gap: 0.45rem;
        max-width: 960px;
        margin: 0 auto;
      }
      .line mc-icon {
        flex: 0 0 auto;
      }
    `,
  ],
})
export class MachineBannerComponent {
  protected readonly m = inject(MachineContext);

  protected readonly text = computed(() => {
    const name = this.m.name();
    if (!this.m.online()) {
      const since = this.m.since();
      return since
        ? tr(
            "{name} went offline at {time}. What you see is from then; sending waits until it's back.",
            {
              name,
              time: pastClock(since),
            },
          )
        : tr(
            "{name} is offline. What you see is from when it was last heard; sending waits until it's back.",
            { name },
          );
    }
    return this.m.mayDrive()
      ? tr('Runs on {name} — you drive it from here.', { name })
      : tr('Runs on {name} — you can watch it from here.', { name });
  });
}
