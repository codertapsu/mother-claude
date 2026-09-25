import { ChangeDetectionStrategy, Component, computed, input, signal } from '@angular/core';

import { ControlCaps, Surface } from '../core/models';
import { IconComponent } from './icon.component';
import { SheetComponent } from './sheet.component';
import { channelHint, channelLabel, channelTone, surfaceLabel } from './util';

interface CapRow {
  label: string;
  value: string;
  ok: boolean;
}

/**
 * "VS Code · Hooks" — where the session runs and how this dashboard can reach
 * it. Tapping opens a sheet with the full capability matrix and the server's
 * notes, so the UI is honest about what each session supports.
 */
@Component({
  selector: 'mc-channel-badge',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, SheetComponent],
  template: `
    <button
      type="button"
      class="badge-btn"
      [attr.data-tone]="tone()"
      [attr.aria-label]="surfaceText() + ', control: ' + channelText() + '. Show what works'"
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
      @if (control().notes.length) {
        <h2>Notes</h2>
        <ul class="notes">
          @for (n of control().notes; track $index) {
            <li>{{ n }}</li>
          }
        </ul>
      }
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
      .caps,
      .notes {
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
      }
      .notes li {
        padding: 0.3rem 0;
        font-size: 0.85rem;
        color: var(--muted);
        line-height: 1.45;
      }
    `,
  ],
})
export class ChannelBadgeComponent {
  readonly surface = input.required<Surface | string>();
  readonly control = input.required<ControlCaps>();
  /** Show only the channel ("Hooks"), not the surface. */
  readonly compact = input(false);

  protected readonly open = signal(false);
  protected readonly surfaceText = computed(() => surfaceLabel(this.surface()));
  protected readonly channelText = computed(() => channelLabel(this.control().channel));
  protected readonly tone = computed(() => channelTone(this.control().channel));
  protected readonly hint = computed(() => channelHint(this.control().channel));
  protected readonly sheetTitle = computed(() =>
    this.surfaceText() === this.channelText()
      ? this.surfaceText()
      : `${this.surfaceText()} · ${this.channelText()}`,
  );

  protected readonly rows = computed<CapRow[]>(() => {
    const c = this.control();
    const yes = (b: boolean) => (b ? 'yes' : 'no');
    return [
      {
        label: 'Send messages',
        ok: c.message !== 'none',
        value:
          c.message === 'direct' ? 'direct' : c.message === 'relay' ? 'relayed' : 'not available',
      },
      { label: 'Approve permissions', ok: c.approve, value: yes(c.approve) },
      { label: 'Answer questions', ok: c.answer, value: yes(c.answer) },
      {
        label: 'Interrupt',
        ok: c.interrupt !== 'none',
        value:
          c.interrupt === 'immediate'
            ? 'immediately'
            : c.interrupt === 'next-tool'
              ? 'at the next tool call'
              : 'no',
      },
      { label: 'Switch model', ok: c.setModel, value: yes(c.setModel) },
      {
        label: 'Switch permission mode',
        ok: c.setMode !== 'none',
        value:
          c.setMode === 'any' ? 'any time' : c.setMode === 'on-prompt' ? 'with a prompt' : 'no',
      },
      { label: 'Change effort', ok: c.setEffort, value: yes(c.setEffort) },
      { label: 'Slash commands', ok: c.slashCommands, value: yes(c.slashCommands) },
      { label: 'Attach images', ok: c.attachImages, value: yes(c.attachImages) },
      { label: 'Stop background tasks', ok: c.stopTask, value: yes(c.stopTask) },
      {
        label: 'Continue here (fork)',
        ok: c.fork || c.continueOwned,
        value: yes(c.fork || c.continueOwned),
      },
      {
        label: 'Stop / respawn / remove',
        ok: c.lifecycle.stop || c.lifecycle.respawn || c.lifecycle.rm,
        value:
          [c.lifecycle.stop && 'stop', c.lifecycle.respawn && 'respawn', c.lifecycle.rm && 'remove']
            .filter(Boolean)
            .join(' · ') || 'no',
      },
      { label: 'Hooks active', ok: c.hooksActive, value: yes(c.hooksActive) },
    ];
  });
}
