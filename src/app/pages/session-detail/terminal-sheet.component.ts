import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  ElementRef,
  Injector,
  afterNextRender,
  computed,
  effect,
  inject,
  signal,
  untracked,
  viewChild,
} from '@angular/core';

import { ApiService, describeError } from '../../core/api.service';
import { KeyName } from '../../core/models';
import { capKeys } from '../../shared/caps';
import { IconComponent } from '../../shared/icon.component';
import { SheetComponent } from '../../shared/sheet.component';
import { SessionContext } from './session-context';

const REFRESH_MS = 2000;
const KEYS: { key: KeyName; label: string; aria: string }[] = [
  { key: 'Escape', label: 'Esc', aria: 'Escape' },
  { key: 'Enter', label: '⏎', aria: 'Enter' },
  { key: 'Up', label: '↑', aria: 'Up arrow' },
  { key: 'Down', label: '↓', aria: 'Down arrow' },
  { key: 'Tab', label: 'Tab', aria: 'Tab' },
];
const DIGITS: KeyName[] = ['1', '2', '3', '4', '5', '6', '7', '8', '9'];

/**
 * What the session's terminal shows right now (tmux pane capture, or the
 * rendered log of a background job), refreshed every 2 s while the sheet is
 * open and the page visible. For tmux sessions a key row sends Escape,
 * Enter, arrows, Tab and digits — enough to answer a dialog that is open in
 * the terminal.
 */
@Component({
  selector: 'mc-terminal-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, SheetComponent],
  template: `
    <mc-sheet size="full" title="Terminal" [open]="ctx.terminalOpen()" (closed)="close()">
      <button
        sheetActions
        class="btn ghost small"
        type="button"
        [disabled]="loading()"
        (click)="load()"
      >
        <mc-icon name="refresh" [size]="14" /> Refresh
      </button>
      <div class="wrap">
        @if (error()) {
          <p class="error-text small">{{ error() }}</p>
        }
        <pre #screen class="screen" tabindex="0" aria-label="Terminal contents">{{
          text() || (loading() ? 'Loading…' : 'Nothing to show.')
        }}</pre>
        @if (keys()) {
          <div class="keys" role="group" aria-label="Send a key">
            @for (k of keyList; track k.key) {
              <button
                class="btn small"
                type="button"
                [attr.aria-label]="'Send ' + k.aria"
                [disabled]="!ctx.canAct()"
                (click)="send(k.key)"
              >
                {{ k.label }}
              </button>
            }
            @for (d of digits; track d) {
              <button
                class="btn small digit"
                type="button"
                [attr.aria-label]="'Send ' + d"
                [disabled]="!ctx.canAct()"
                (click)="send(d)"
              >
                {{ d }}
              </button>
            }
          </div>
          @if (!ctx.canDangerous()) {
            <p class="muted small">
              Keys are a dangerous action: they work from the Mac, or on a device allowed to take
              dangerous actions.
            </p>
          }
        }
      </div>
    </mc-sheet>
  `,
  styles: [
    `
      .wrap {
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
        min-height: 100%;
      }
      .screen {
        flex: 1;
        margin: 0;
        min-height: 14rem;
        padding: 0.6rem 0.75rem;
        background: var(--code-bg);
        color: var(--text);
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        font-size: 0.72rem;
        line-height: 1.35;
        white-space: pre;
        overflow: auto;
      }
      .keys {
        display: flex;
        flex-wrap: wrap;
        gap: 0.35rem;
      }
      .keys .btn {
        min-width: 2.75rem;
      }
    `,
  ],
})
export class TerminalSheetComponent {
  protected readonly ctx = inject(SessionContext);
  private api = inject(ApiService);
  private injector = inject(Injector);

  private readonly screen = viewChild<ElementRef<HTMLElement>>('screen');
  protected readonly text = signal('');
  protected readonly loading = signal(false);
  protected readonly error = signal('');
  protected readonly keyList = KEYS;
  protected readonly digits = DIGITS;
  protected readonly keys = computed(() => {
    const s = this.ctx.session();
    return capKeys(s?.control, s) && !!s?.live;
  });
  private timer?: ReturnType<typeof setInterval>;

  constructor() {
    effect(() => {
      const open = this.ctx.terminalOpen();
      untracked(() => {
        clearInterval(this.timer);
        if (!open) return;
        void this.load();
        this.timer = setInterval(() => {
          if (document.visibilityState === 'visible') void this.load();
        }, REFRESH_MS);
      });
    });
    inject(DestroyRef).onDestroy(() => clearInterval(this.timer));
  }

  protected close(): void {
    this.ctx.terminalOpen.set(false);
  }

  async load(): Promise<void> {
    if (this.loading()) return;
    this.loading.set(true);
    try {
      const res = await this.api.getTerminal(this.ctx.id());
      const el = this.screen()?.nativeElement;
      const atEnd = !el || el.scrollHeight - el.scrollTop - el.clientHeight < 40;
      this.text.set(res.text ?? '');
      this.error.set('');
      if (atEnd && el) {
        afterNextRender(() => (el.scrollTop = el.scrollHeight), { injector: this.injector });
      }
    } catch (e) {
      this.error.set(describeError(e).text);
    } finally {
      this.loading.set(false);
    }
  }

  protected async send(key: KeyName): Promise<void> {
    if (await this.ctx.sendKey(key)) setTimeout(() => void this.load(), 350);
  }
}
