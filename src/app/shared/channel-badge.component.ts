import { ChangeDetectionStrategy, Component, computed, input, signal } from '@angular/core';

import { tr } from '../core/i18n/i18n';
import { TPipe } from '../core/i18n/t.pipe';
import { ControlCaps, SessionSummary, Surface } from '../core/models';
import { CapLimitsComponent } from './cap-limits.component';
import {
  capAttach,
  capKeys,
  capLimits,
  capRename,
  capTerminal,
  capThinking,
  capUltracode,
  capWake,
} from './caps';
import { IconComponent } from './icon.component';
import { SheetComponent } from './sheet.component';
import { channelHint, channelLabel, channelTone, surfaceLabel } from './util';

interface CapRow {
  label: string;
  value: string;
  ok: boolean;
}

/**
 * "VS Code · Via hooks" — where the session runs and how much this dashboard
 * can do with it ("Full control", "Via hooks", "Via tmux", "Experimental",
 * "Watch only"). Tapping opens a sheet with the full capability matrix and
 * the session's limits, each with the action that fixes it.
 */
@Component({
  selector: 'mc-channel-badge',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CapLimitsComponent, IconComponent, SheetComponent, TPipe],
  template: `
    <button
      type="button"
      class="badge-btn"
      [attr.data-tone]="tone()"
      [attr.aria-label]="
        '{surface}, {channel}. Show what works from here'
          | t: { surface: surfaceText(), channel: channelText() }
      "
      (click)="open.set(true); $event.stopPropagation()"
    >
      @if (!compact() && surfaceText() !== channelText()) {
        <span class="surface">{{ surfaceText() }}</span>
        <span class="sep" aria-hidden="true">·</span>
      }
      <span class="channel">{{ channelText() }}</span>
      <mc-icon name="info" [size]="12" />
    </button>

    <mc-sheet [open]="open()" [title]="sheetTitle()" (closed)="open.set(false)">
      <p class="hint">{{ hint() }}</p>
      <ul class="caps">
        @for (row of rows(); track row.label) {
          <li [class.no]="!row.ok">
            <mc-icon [name]="row.ok ? 'check' : 'x'" [size]="15" />
            <span class="cap">{{ row.label }}</span>
            <span class="val">{{ row.value }}</span>
          </li>
        }
      </ul>
      <h2>{{ 'Limits' | t }}</h2>
      <mc-cap-limits [session]="session()" [limits]="limits()" (done)="open.set(false)" />
    </mc-sheet>
  `,
  styles: [
    `
      :host {
        display: inline-flex;
        min-width: 0;
      }
      .badge-btn {
        appearance: none;
        display: inline-flex;
        align-items: center;
        gap: 0.3rem;
        font: inherit;
        font-size: 0.72rem;
        line-height: 1.4;
        color: var(--tone);
        background: var(--tone-soft);
        border: 1px solid var(--tone-border);
        border-radius: 0.4rem;
        padding: 0.08rem 0.45rem;
        cursor: pointer;
        white-space: nowrap;
        position: relative;
        z-index: 1;
      }
      .surface {
        color: var(--text);
      }
      .sep {
        color: var(--faint);
      }
      .hint {
        margin: 0 0 0.75rem;
        color: var(--muted);
      }
      .caps {
        list-style: none;
        margin: 0;
        padding: 0;
      }
      .caps li {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        padding: 0.4rem 0;
        border-top: 1px solid var(--border);
        font-size: 0.9rem;
      }
      .caps li mc-icon {
        color: var(--green);
      }
      .caps li.no mc-icon,
      .caps li.no .val {
        color: var(--faint);
      }
      .cap {
        flex: 1;
      }
      .val {
        color: var(--muted);
        font-size: 0.8rem;
        text-align: right;
      }
    `,
  ],
})
export class ChannelBadgeComponent {
  readonly surface = input.required<Surface | string>();
  readonly control = input.required<ControlCaps>();
  /** The session (enables the limit actions: continue, fork, terminal). */
  readonly session = input<SessionSummary | undefined>(undefined);
  /** Show only the channel ("Via hooks"), not the surface. */
  readonly compact = input(false);

  protected readonly open = signal(false);
  protected readonly surfaceText = computed(() => surfaceLabel(this.surface()));
  protected readonly channelText = computed(() => channelLabel(this.control().channel));
  protected readonly tone = computed(() => channelTone(this.control().channel));
  protected readonly hint = computed(() => channelHint(this.control().channel));
  protected readonly limits = computed(() => capLimits(this.control()));
  protected readonly sheetTitle = computed(() =>
    this.surfaceText() === this.channelText()
      ? this.surfaceText()
      : `${this.surfaceText()} · ${this.channelText()}`,
  );

  /** What can be done from here, each in words (translated as it is built:
   * the rows follow a language switch). */
  protected readonly rows = computed<CapRow[]>(() => {
    const c = this.control();
    const s = this.session();
    const no = tr('no||capability');
    const yes = (b: boolean) => (b ? tr('yes||capability') : no);
    const attach = capAttach(c);
    const rename = capRename(c, s?.owned);
    return [
      {
        label: tr('Send messages'),
        ok: c.message !== 'none',
        value:
          c.message === 'direct'
            ? tr('straight in')
            : c.message === 'relay'
              ? capWake(c)
                ? tr('relayed')
                : tr('at the next stopping point')
              : tr('not available'),
      },
      {
        label: tr('Approve permissions'),
        ok: c.approve,
        value: c.approve ? (c.approveVia === 'keys' ? tr('by keys') : yes(true)) : no,
      },
      { label: tr('Answer questions'), ok: c.answer, value: yes(c.answer) },
      {
        label: tr('Interrupt'),
        ok: c.interrupt !== 'none',
        value:
          c.interrupt === 'immediate'
            ? tr('immediately')
            : c.interrupt === 'next-tool'
              ? tr('at the next tool call')
              : no,
      },
      { label: tr('Switch model'), ok: c.setModel, value: yes(c.setModel) },
      {
        label: tr('Switch permission mode'),
        ok: c.setMode !== 'none',
        value:
          c.setMode === 'any'
            ? tr('any time')
            : c.setMode === 'on-prompt'
              ? tr('with a prompt')
              : no,
      },
      { label: tr('Change effort'), ok: c.setEffort, value: yes(c.setEffort) },
      { label: tr('Switch ultracode'), ok: capUltracode(c), value: yes(capUltracode(c)) },
      { label: tr('Switch thinking'), ok: capThinking(c), value: yes(capThinking(c)) },
      { label: tr('Slash commands'), ok: c.slashCommands, value: yes(c.slashCommands) },
      {
        label: tr('Attachments'),
        ok: attach !== 'none',
        value: attach === 'inline' ? tr('images') : attach === 'path' ? tr('as file paths') : no,
      },
      {
        label: tr('Rename'),
        ok: rename !== 'none',
        value:
          rename === 'sync'
            ? tr('everywhere')
            : rename === 'alias'
              ? tr('only in Mother Claude')
              : no,
      },
      { label: tr('Terminal view'), ok: capTerminal(c, s), value: yes(capTerminal(c, s)) },
      { label: tr('Send keys'), ok: capKeys(c, s), value: yes(capKeys(c, s)) },
      { label: tr('Stop background tasks'), ok: c.stopTask, value: yes(c.stopTask) },
      {
        label: tr('Continue here'),
        ok: c.fork || c.continueOwned,
        value: c.fork || c.continueOwned ? (s?.live ? tr('as a fork') : yes(true)) : no,
      },
      {
        label: tr('Stop / respawn / remove'),
        ok: c.lifecycle.stop || c.lifecycle.respawn || c.lifecycle.rm,
        value:
          [
            c.lifecycle.stop && tr('stop||lifecycle'),
            c.lifecycle.respawn && tr('respawn||lifecycle'),
            c.lifecycle.rm && tr('remove||lifecycle'),
          ]
            .filter(Boolean)
            .join(' · ') || no,
      },
      { label: tr('Hooks connected'), ok: c.hooksActive, value: yes(c.hooksActive) },
    ];
  });
}
