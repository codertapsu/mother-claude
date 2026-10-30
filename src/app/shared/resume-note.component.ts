import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';

import { SessionSummary } from '../core/models';
import { NowService } from '../core/now.service';
import { IconComponent, IconName } from './icon.component';
import { resumeNote } from './util';

/**
 * A session between turns that resumes by itself (status `background`), in
 * a sentence on its page: "Claude picks this up when its background work
 * finishes — closing VS Code now stops it." / "Claude resumes this by
 * itself in 12m — …", so it isn't taken for done and closed. Says nothing
 * for any other session.
 */
@Component({
  selector: 'mc-resume-note',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent],
  // Parents space it only while it shows something.
  host: { '[class.has-note]': '!!note()' },
  template: `
    @if (note(); as n) {
      <div class="note">
        <mc-icon [name]="n.icon" [size]="16" />
        <p class="text">{{ n.text }}</p>
      </div>
    }
  `,
  styles: [
    `
      :host {
        display: block;
      }
      .note {
        display: flex;
        align-items: flex-start;
        gap: 0.5rem;
        padding: 0.5rem 0.6rem 0.5rem 0.75rem;
        border: 1px solid var(--accent);
        border-radius: var(--radius-sm);
        background: var(--accent-soft);
        font-size: 0.85rem;
      }
      .note > mc-icon {
        flex: 0 0 auto;
        margin-top: 0.1rem;
        color: var(--accent-text);
      }
      .text {
        flex: 1 1 auto;
        min-width: 0;
        margin: 0;
        line-height: 1.45;
        overflow-wrap: anywhere;
      }
    `,
  ],
})
export class ResumeNoteComponent {
  private clock = inject(NowService);
  readonly session = input<SessionSummary | undefined>(undefined);

  protected readonly note = computed<{ icon: IconName; text: string } | null>(() => {
    const s = this.session();
    if (!s?.live || s.status !== 'background') return null;
    const scheduled = s.activity?.kind === 'scheduled';
    // Only a countdown needs the ticking clock.
    const now = scheduled && s.activity?.resumesAt ? this.clock.now() : Date.now();
    const text = resumeNote(s, now);
    return text ? { icon: scheduled ? 'clock' : 'layers', text } : null;
  });
}
