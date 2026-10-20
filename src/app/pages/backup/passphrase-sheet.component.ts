import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
} from '@angular/core';
import { FormsModule } from '@angular/forms';

import { ApiService } from '../../core/api.service';
import { PASSPHRASE_MIN, backupErrorText } from '../../core/backup';
import { CopyButtonComponent } from '../../shared/copy-button.component';
import { IconComponent } from '../../shared/icon.component';
import { SheetComponent } from '../../shared/sheet.component';

/** Where the passphrase dialog is. */
export type PassphraseStep = 'choose' | 'own' | 'show' | 'saved';

/**
 * Desktop: the backup passphrase (docs/BACKUP.md). *Generate* asks the Mac
 * for one, shows it this once with a copy button, and closes only after
 * "I have written it down"; *Use my own* takes one of at least twelve
 * characters, typed twice. Either way it goes to the Mac's Keychain and
 * nowhere else: this dialog keeps it only while it is open, and forgets it
 * when it closes. Existing backups stay readable (they are re-keyed).
 */
@Component({
  selector: 'mc-passphrase-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, CopyButtonComponent, IconComponent, SheetComponent],
  template: `
    <mc-sheet [open]="open()" [title]="title()" (closed)="tryClose()">
      @if (open()) {
        @switch (step()) {
          @case ('choose') {
            <p class="small lede">
              Without the passphrase no backup can be read — by you or anyone, and Mother Claude
              can't recover it. It is kept in this Mac's Keychain.
              @if (changing()) {
                Changing it keeps every backup readable: they are re-keyed to the new one.
              }
            </p>
            <div class="stack">
              <button class="btn primary" type="button" [disabled]="busy()" (click)="generate()">
                <mc-icon name="key" [size]="15" />
                {{ busy() ? 'Generating…' : 'Generate a passphrase' }}
              </button>
              <button class="btn" type="button" [disabled]="busy()" (click)="step.set('own')">
                Use my own…
              </button>
            </div>
          }
          @case ('own') {
            <form class="stack" (submit)="saveOwn($event)">
              <label class="field">
                <span class="field-label">Passphrase</span>
                <input
                  name="pp"
                  autocomplete="new-password"
                  spellcheck="false"
                  autocapitalize="off"
                  [type]="reveal() ? 'text' : 'password'"
                  [ngModel]="own()"
                  (ngModelChange)="own.set($event); error.set('')"
                />
              </label>
              <label class="field">
                <span class="field-label">Type it again</span>
                <input
                  name="pp2"
                  autocomplete="new-password"
                  spellcheck="false"
                  autocapitalize="off"
                  [type]="reveal() ? 'text' : 'password'"
                  [ngModel]="again()"
                  (ngModelChange)="again.set($event); error.set('')"
                />
              </label>
              <label class="check small">
                <input
                  type="checkbox"
                  [checked]="reveal()"
                  (change)="reveal.set($any($event.target).checked)"
                />
                Show what I type
              </label>
              @if (ownProblem()) {
                <p class="muted small">{{ ownProblem() }}</p>
              }
              <div class="row">
                <button class="btn primary" type="submit" [disabled]="!!ownProblem() || busy()">
                  {{ busy() ? 'Saving…' : 'Save in the Keychain' }}
                </button>
                <button
                  class="btn ghost"
                  type="button"
                  [disabled]="busy()"
                  (click)="step.set('choose')"
                >
                  Back
                </button>
              </div>
            </form>
          }
          @case ('show') {
            <p class="small lede">
              Write it down and keep it somewhere safe. It is shown only now; without it no backup
              can be read.
            </p>
            <p class="secret mono">{{ secret() }}</p>
            <div class="row">
              <mc-copy-button [text]="secret()" ariaLabel="Copy the passphrase" />
            </div>
            <label class="check" [class.nudge]="nudge()">
              <input
                type="checkbox"
                [checked]="written()"
                (change)="written.set($any($event.target).checked); nudge.set(false)"
              />
              I have written it down
            </label>
            @if (nudge()) {
              <p class="notice warn small" role="alert">
                Tick “I have written it down” first: it is shown only now.
              </p>
            }
            <div class="row actions">
              <button class="btn primary" type="button" [disabled]="!written()" (click)="finish()">
                Done
              </button>
            </div>
          }
          @case ('saved') {
            <p class="notice ok small" role="status">
              Saved in this Mac's Keychain. Backups are encrypted with it from now on.
            </p>
            <div class="row actions">
              <button class="btn primary" type="button" (click)="finish()">Done</button>
            </div>
          }
        }
        @if (error()) {
          <p class="notice error small" role="alert">{{ error() }}</p>
        }
      }
    </mc-sheet>
  `,
  styles: [
    `
      .lede {
        margin: 0 0 0.75rem;
        line-height: 1.55;
        color: var(--muted);
      }
      .secret {
        margin: 0.25rem 0 0.6rem;
        padding: 0.75rem;
        font-size: 1.05rem;
        letter-spacing: 0.04em;
        line-height: 1.6;
        text-align: center;
        overflow-wrap: anywhere;
        user-select: all;
        background: var(--panel-2);
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
      }
      .check {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        margin-top: 0.75rem;
        cursor: pointer;
      }
      .check.nudge {
        color: var(--amber);
      }
      .notice {
        margin: 0.6rem 0 0;
      }
      .actions {
        margin-top: 0.75rem;
      }
    `,
  ],
})
export class PassphraseSheetComponent {
  private api = inject(ApiService);

  readonly open = input(false);
  /** A passphrase is set already (the dialog changes it). */
  readonly changing = input(false);
  readonly closed = output<void>();
  /** A passphrase was set (the page reloads its view). */
  readonly saved = output<void>();

  protected readonly step = signal<PassphraseStep>('choose');
  protected readonly busy = signal(false);
  protected readonly error = signal('');
  /** The generated passphrase, only while it is shown. */
  protected readonly secret = signal('');
  protected readonly written = signal(false);
  protected readonly nudge = signal(false);
  protected readonly own = signal('');
  protected readonly again = signal('');
  protected readonly reveal = signal(false);

  protected readonly title = computed(() =>
    this.changing() ? 'Change the passphrase' : 'Backup passphrase',
  );
  protected readonly ownProblem = computed(() => {
    const p = this.own();
    if ([...p].length < PASSPHRASE_MIN) return `At least ${PASSPHRASE_MIN} characters.`;
    if (p !== this.again()) return 'Type the same passphrase twice.';
    return '';
  });

  constructor() {
    // Every opening starts over; closing forgets everything typed or shown.
    effect(() => {
      const open = this.open();
      untracked(() => this.forget(open));
    });
  }

  protected async generate(): Promise<void> {
    if (this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      const res = await this.api.generateBackupPassphrase();
      const p = typeof res?.passphrase === 'string' ? res.passphrase : '';
      if (!p) throw new Error('The Mac sent no passphrase. Try again.');
      this.secret.set(p);
      this.written.set(false);
      this.step.set('show');
      this.saved.emit();
    } catch (e) {
      this.error.set(backupErrorText(e));
    } finally {
      this.busy.set(false);
    }
  }

  protected async saveOwn(e: Event): Promise<void> {
    e.preventDefault();
    if (this.ownProblem() || this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      await this.api.setBackupPassphrase(this.own());
      this.own.set('');
      this.again.set('');
      this.step.set('saved');
      this.saved.emit();
    } catch (err) {
      this.error.set(backupErrorText(err));
    } finally {
      this.busy.set(false);
    }
  }

  /** Escape, the backdrop or ✕: not while a passphrase shows unconfirmed. */
  protected tryClose(): void {
    if (this.step() === 'show' && !this.written()) {
      this.nudge.set(true);
      return;
    }
    this.closed.emit();
  }

  protected finish(): void {
    if (this.step() === 'show' && !this.written()) return;
    this.closed.emit();
  }

  private forget(open: boolean): void {
    this.secret.set('');
    this.own.set('');
    this.again.set('');
    this.reveal.set(false);
    this.written.set(false);
    this.nudge.set(false);
    this.error.set('');
    this.busy.set(false);
    if (open) this.step.set('choose');
  }
}
