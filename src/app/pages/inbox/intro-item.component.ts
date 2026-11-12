import { ChangeDetectionStrategy, Component, inject, input, output, signal } from '@angular/core';

import { ApiService } from '../../core/api.service';
import { computerNoun, tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { Introduction, MachinesView } from '../../core/machines';
import { NoticeService } from '../../core/notice.service';
import { AttentionCardComponent } from '../../shared/attention-card.component';
import { DesktopOnlyComponent } from '../../shared/desktop-only.component';
import { osName } from '../machines/machines-text';

/**
 * One introduction in the Inbox's *Needs you* (docs/PLAN-MACHINES.md §5.12):
 * one of your computers says another one exists — *Pair with it?* It pairs
 * nothing on its own: accepting starts the normal pairing with that
 * computer (the same six digits on both screens, what each may do), and it
 * is paired only once both desktops confirmed. *Not now* tells the other
 * computer. While its pairing runs, *Show the pairing* opens it again; one
 * that did not pair may be tried again. A phone sees it, the desktop
 * answers.
 */
@Component({
  selector: 'mc-inbox-intro',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AttentionCardComponent, DesktopOnlyComponent, TPipe],
  template: `
    @let i = intro();
    <mc-attention-card
      kind="intro"
      tone="info"
      [heading]="'{from} introduces {name}' | t: { from: i.fromName, name: i.other.name }"
      [meta]="meta()"
      [why]="why()"
      [when]="i.at"
    >
      <mc-desktop-only [line]="'Answer on the desktop app.' | t">
        @if (i.state === 'offered') {
          <button
            class="btn small primary"
            type="button"
            [disabled]="busy()"
            (click)="answer(true)"
          >
            {{ 'Pair with {name}…' | t: { name: i.other.name } }}
          </button>
          <button class="btn small ghost" type="button" [disabled]="busy()" (click)="answer(false)">
            {{ 'Not now' | t }}
          </button>
        } @else if (i.pairing) {
          <span class="muted small">{{ 'Pairing…' | t }}</span>
          <button class="btn small" type="button" (click)="opened.emit(i)">
            {{ 'Show the pairing' | t }}
          </button>
        } @else {
          <span class="muted small">{{ 'Not paired yet.' | t }}</span>
          <button class="btn small" type="button" [disabled]="busy()" (click)="answer(true)">
            {{ 'Try again' | t }}
          </button>
          <button class="btn small ghost" type="button" [disabled]="busy()" (click)="answer(false)">
            {{ 'Not now' | t }}
          </button>
        }
      </mc-desktop-only>
    </mc-attention-card>
  `,
})
export class IntroItemComponent {
  private readonly api = inject(ApiService);
  private readonly notices = inject(NoticeService);

  readonly intro = input.required<Introduction>();
  /** The view after an answer. */
  readonly answered = output<MachinesView>();
  /** Show the pairing an accepted introduction runs. */
  readonly opened = output<Introduction>();

  protected readonly busy = signal(false);

  /** "Linux · 1A2B 3C4D …": the computer introduced. */
  protected meta(): string {
    const o = this.intro().other;
    return `${osName(o.os)} · ${o.key}`;
  }

  protected why(): string {
    return tr(
      'Pair this {computer} with it? An introduction pairs nothing on its own: both computers show the same six digits, and each chooses what the other may do.',
      { computer: computerNoun() },
    );
  }

  protected async answer(accept: boolean): Promise<void> {
    const i = this.intro();
    this.busy.set(true);
    try {
      this.answered.emit(await this.api.answerIntroduction(i.id, accept));
      if (accept) this.opened.emit(i);
    } catch (e) {
      this.notices.error(e);
    } finally {
      this.busy.set(false);
    }
  }
}
