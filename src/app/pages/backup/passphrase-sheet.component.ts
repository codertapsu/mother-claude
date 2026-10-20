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
import {
  PASSPHRASE_MIN,
  andList,
  backupErrorText,
  normalizePassphraseChange,
  passphraseFailure,
} from '../../core/backup';
import { PassphraseChange } from '../../core/models';
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
 * when it closes. Every set in the folder is re-keyed to it (and the dialog
 * says which, and which it left alone). A change that fails after some set
 * already has the new passphrase did not finish: a generated one is still
 * shown, behind the same gate, and setting the passphrase again finishes it.
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
            @if (unfinished()) {
              <p class="notice warn small" role="status">
                The last passphrase change did not finish, so some backups may open only with the
                new passphrase. Set it again — the same one or a new one — to finish: every backup
                then opens with it.
              </p>
            }
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
            @if (stopped()) {
              <p class="notice warn small" role="alert">
                The change did not finish: {{ stopped() }} Some backups already open with this new
                passphrase, so write it down as well. Setting the passphrase again finishes the
                change.
              </p>
            }
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
            @for (line of changeLines(); track line) {
              <p class="muted small done">{{ line }}</p>
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
            @for (line of changeLines(); track line) {
              <p class="muted small done">{{ line }}</p>
            }
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
        line-height: 1.5;
      }
      .notice:first-child {
        margin: 0 0 0.75rem;
      }
      .done {
        margin: 0.6rem 0 0;
        line-height: 1.5;
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
  /** The last change did not finish: setting it again finishes it. */
  readonly unfinished = input(false);
  readonly closed = output<void>();
  /** The passphrase changed, or a change went part of the way (the page
   * reads its view again). */
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
  /** What a change that went through did to the sets. */
  protected readonly change = signal<PassphraseChange | null>(null);
  /** Why a change that some set already has stopped ('' when none did). */
  protected readonly stopped = signal('');

  protected readonly title = computed(() =>
    this.unfinished()
      ? 'Finish the passphrase change'
      : this.changing()
        ? 'Change the passphrase'
        : 'Backup passphrase',
  );
  protected readonly ownProblem = computed(() => {
    const p = this.own();
    if ([...p].length < PASSPHRASE_MIN) return `At least ${PASSPHRASE_MIN} characters.`;
    if (p !== this.again()) return 'Type the same passphrase twice.';
    return '';
  });
  /** Which sets it re-keyed, and which it left as they were. */
  protected readonly changeLines = computed(() => {
    const c = this.change();
    if (!c) return [];
    const lines: string[] = [];
    if (c.rekeyed.length === 1) {
      lines.push(`${c.rekeyed[0]}'s backup opens with it now.`);
    } else if (c.rekeyed.length) {
      lines.push(`The backups of ${andList(c.rekeyed)} open with it now.`);
    }
    if (c.left.length) {
      const one = c.left.length === 1;
      lines.push(
        `Left as ${one ? 'it was' : 'they were'}: ${andList(c.left)} — ${one ? 'a backup' : 'backups'} in the folder of no account on this Mac that ${one ? 'opens' : 'open'} with neither passphrase.`,
      );
    }
    return lines;
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
      const change = normalizePassphraseChange(await this.api.generateBackupPassphrase());
      if (!change.passphrase) throw new Error('The Mac sent no passphrase. Try again.');
      this.show(change.passphrase);
      this.change.set(change);
      this.saved.emit();
    } catch (e) {
      const failed = passphraseFailure(e);
      if (failed.applied && failed.passphrase) {
        // Some backups have it already: it must not be lost.
        this.show(failed.passphrase);
        this.stopped.set(backupErrorText(e));
        this.saved.emit();
      } else if (failed.applied) {
        this.error.set(this.unfinishedText(e, 'the new passphrase'));
        this.saved.emit();
      } else {
        this.error.set(`Nothing changed: ${backupErrorText(e)}`);
      }
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
      const change = normalizePassphraseChange(await this.api.setBackupPassphrase(this.own()));
      this.own.set('');
      this.again.set('');
      this.change.set(change);
      this.step.set('saved');
      this.saved.emit();
    } catch (err) {
      if (passphraseFailure(err).applied) {
        // What was typed stays in the form: saving it again finishes it.
        this.error.set(this.unfinishedText(err, 'the passphrase you typed'));
        this.saved.emit();
      } else {
        this.error.set(`Nothing changed: ${backupErrorText(err)}`);
      }
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

  private show(passphrase: string): void {
    this.secret.set(passphrase);
    this.written.set(false);
    this.nudge.set(false);
    this.step.set('show');
  }

  private unfinishedText(e: unknown, which: string): string {
    return `The change did not finish: ${backupErrorText(e)} Some backups already open with ${which}, so keep it: setting it again finishes the change.`;
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
    this.change.set(null);
    this.stopped.set('');
    if (open) this.step.set('choose');
  }
}
