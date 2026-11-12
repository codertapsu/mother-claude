import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  input,
  output,
  signal,
} from '@angular/core';

import { ApiService } from '../../core/api.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import {
  MachinesSettings,
  MachinesView,
  SettingsOffer,
  sameCode,
  sameRelay,
} from '../../core/machines';
import { NoticeService } from '../../core/notice.service';
import { AttentionCardComponent } from '../../shared/attention-card.component';
import { DesktopOnlyComponent } from '../../shared/desktop-only.component';
import { codeText } from '../machines/machines-text';
import { relayText } from '../machines/options-form.component';

/**
 * A setting another computer uses, suggested here (docs/PLAN-MACHINES.md
 * §5.3), in the Inbox's *Needs you*: its relay — two computers on different
 * relays may never meet — or how it moves code, so a move works the same
 * from either computer. *Use it here too* takes it, *Keep mine* says no. A
 * phone sees it, the desktop answers.
 */
@Component({
  selector: 'mc-inbox-offer',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AttentionCardComponent, DesktopOnlyComponent, TPipe],
  template: `
    <mc-attention-card
      kind="offer"
      tone="info"
      [heading]="'{name} suggests its settings' | t: { name: offer().fromName }"
      [why]="why()"
      [when]="offer().at"
    >
      <mc-desktop-only [line]="'Answer on the desktop app.' | t">
        <button class="btn small primary" type="button" [disabled]="busy()" (click)="answer(true)">
          {{ 'Use it here too' | t }}
        </button>
        <button class="btn small ghost" type="button" [disabled]="busy()" (click)="answer(false)">
          {{ 'Keep mine' | t }}
        </button>
      </mc-desktop-only>
    </mc-attention-card>
  `,
})
export class OfferItemComponent {
  private readonly api = inject(ApiService);
  private readonly notices = inject(NoticeService);

  readonly offer = input.required<SettingsOffer>();
  /** This computer's settings, to say what differs. */
  readonly settings = input.required<MachinesSettings>();
  /** The view after an answer. */
  readonly answered = output<MachinesView>();

  protected readonly busy = signal(false);

  /** What differs: the relay, how code moves, or both. */
  protected readonly why = computed(() => {
    const o = this.offer();
    const s = this.settings();
    const parts: string[] = [];
    if (!sameRelay(o.relay, s.relay)) {
      parts.push(
        tr(
          '{name} uses another relay setting: {relay}. Two computers on different relays may never meet.',
          { name: o.fromName, relay: relayText(o.relay) },
        ),
      );
    }
    if (o.code && !sameCode(o.code, s.code)) {
      parts.push(
        tr(
          '{name} moves code another way: {code}. Use it here too, so a move works the same from either computer?',
          { name: o.fromName, code: codeText(o.code) },
        ),
      );
    }
    return parts.join(' ');
  });

  protected async answer(accept: boolean): Promise<void> {
    this.busy.set(true);
    try {
      this.answered.emit(await this.api.answerSettingsOffer(accept));
    } catch (e) {
      this.notices.error(e);
    } finally {
      this.busy.set(false);
    }
  }
}
