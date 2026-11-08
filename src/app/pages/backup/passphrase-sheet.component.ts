import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  linkedSignal,
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
import { AuthService } from '../../core/auth.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { PassphraseChange } from '../../core/models';
import { machineNoun } from '../../core/paths';
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
  imports: [FormsModule, CopyButtonComponent, IconComponent, SheetComponent, TPipe],
  template: `
    <mc-sheet [open]="open()" [title]="title()" (closed)="tryClose()">
      @if (open()) {
        @switch (step()) {
          @case ('choose') {
            @if (mode().unfinished) {
              <p class="notice warn small" role="status">
                {{
                  'The last passphrase change did not finish, so some backups may open only with the new passphrase. Set it again — the same one or a new one — to finish: every backup then opens with it.'
                    | t
                }}
              </p>
            }
            <p class="small lede">
              {{
                "Without the passphrase no backup can be read — by you or anyone, and Mother Claude can't recover it."
                  | t
              }}
              {{ keptText() }}
              @if (wasSet()) {
                {{
                  'Changing it keeps every backup readable: they are re-keyed to the new one.' | t
                }}
              }
            </p>
            <div class="stack">
              <button class="btn primary" type="button" [disabled]="busy()" (click)="generate()">
                <mc-icon name="key" [size]="15" />
                {{ busy() ? ('Generating…' | t) : ('Generate a passphrase' | t) }}
              </button>
              <button class="btn" type="button" [disabled]="busy()" (click)="step.set('own')">
                {{ 'Use my own…' | t }}
              </button>
            </div>
          }
          @case ('own') {
            <form class="stack" (submit)="saveOwn($event)">
              <label class="field">
                <span class="field-label">{{ 'Passphrase' | t }}</span>
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
                <span class="field-label">{{ 'Type it again' | t }}</span>
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
                {{ 'Show what I type' | t }}
              </label>
              @if (ownProblem()) {
                <p class="muted small">{{ ownProblem() }}</p>
              }
              <div class="row">
                <button class="btn primary" type="submit" [disabled]="!!ownProblem() || busy()">
                  {{ busy() ? ('Saving…' | t) : saveLabel() }}
                </button>
                <button
                  class="btn ghost"
                  type="button"
                  [disabled]="busy()"
                  (click)="step.set('choose')"
                >
                  {{ 'Back' | t }}
                </button>
              </div>
            </form>
          }
          @case ('show') {
            @if (stopped()) {
              <p class="notice warn small" role="alert">
                {{ 'The change did not finish: {error}' | t: { error: stopped() } }}
                {{
                  'Some backups already open with this new passphrase, so write it down as well. Setting the passphrase again finishes the change.'
                    | t
                }}
              </p>
            }
            <p class="small lede">
              {{
                'Write it down and keep it somewhere safe. It is shown only now; without it no backup can be read.'
                  | t
              }}
            </p>
            <p class="secret mono">{{ secret() }}</p>
            <div class="row">
              <mc-copy-button
                [text]="secret()"
                [label]="'Copy' | t"
                [ariaLabel]="'Copy the passphrase' | t"
              />
            </div>
            <label class="check" [class.nudge]="nudge()">
              <input
                type="checkbox"
                [checked]="written()"
                (change)="written.set($any($event.target).checked); nudge.set(false)"
              />
              {{ 'I have written it down' | t }}
            </label>
            @if (nudge()) {
              <p class="notice warn small" role="alert">
                {{ 'Tick “I have written it down” first: it is shown only now.' | t }}
              </p>
            }
            @for (line of changeLines(); track line) {
              <p class="muted small done">{{ line }}</p>
            }
            <div class="row actions">
              <button class="btn primary" type="button" [disabled]="!written()" (click)="finish()">
                {{ 'Done' | t }}
              </button>
            </div>
          }
          @case ('saved') {
            <p class="notice ok small" role="status">
              {{ savedText() }} {{ 'Backups are encrypted with it from now on.' | t }}
            </p>
            @for (line of changeLines(); track line) {
              <p class="muted small done">{{ line }}</p>
            }
            <div class="row actions">
              <button class="btn primary" type="button" (click)="finish()">{{ 'Done' | t }}</button>
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
  private readonly hostOs = inject(AuthService).hostOs;
  /** The desktop's computer in a sentence: "Mac", "PC" (Vietnamese "máy
   * Mac", "máy PC"). */
  private readonly computer = computed(() => tr(machineNoun(this.hostOs())));
  /** Where the passphrase is kept: this Mac's Keychain, elsewhere on the
   * computer itself (docs/ACCOUNTS.md D3). */
  protected keptText(): string {
    return this.hostOs() === 'macos'
      ? tr("It is kept in this Mac's Keychain.")
      : tr('It is kept on this {computer}.', { computer: this.computer() });
  }
  protected savedText(): string {
    return this.hostOs() === 'macos'
      ? tr("Saved in this Mac's Keychain.")
      : tr('Saved on this {computer}.', { computer: this.computer() });
  }
  protected saveLabel(): string {
    return this.hostOs() === 'macos'
      ? tr('Save in the Keychain')
      : tr('Save on this {computer}', { computer: this.computer() });
  }
  /** What a change that went through did to the sets. */
  protected readonly change = signal<PassphraseChange | null>(null);
  /** Why a change that some set already has stopped ('' when none did). */
  protected readonly stopped = signal('');

  /** What the dialog does, as it was when it opened: saving a first
   * passphrase sets one (`changing` turns true under it) and finishing a
   * change ends `unfinished`, and neither may retitle the open dialog. */
  protected readonly mode = linkedSignal({
    source: this.open,
    computation: () =>
      untracked(() => ({ changing: this.changing(), unfinished: this.unfinished() })),
  });
  protected readonly wasSet = computed(() => this.mode().changing);

  protected readonly title = computed(() =>
    this.mode().unfinished
      ? tr('Finish the passphrase change')
      : this.mode().changing
        ? tr('Change the passphrase')
        : tr('Backup passphrase'),
  );
  protected readonly ownProblem = computed(() => {
    const p = this.own();
    if ([...p].length < PASSPHRASE_MIN)
      return tr('At least {n} characters.', { n: PASSPHRASE_MIN });
    if (p !== this.again()) return tr('Type the same passphrase twice.');
    return '';
  });
  /** Which sets it re-keyed, and which it left as they were. */
  protected readonly changeLines = computed(() => {
    const c = this.change();
    if (!c) return [];
    const lines: string[] = [];
    if (c.rekeyed.length === 1) {
      lines.push(tr("{name}'s backup opens with it now.", { name: c.rekeyed[0] }));
    } else if (c.rekeyed.length) {
      lines.push(tr('The backups of {names} open with it now.', { names: andList(c.rekeyed) }));
    }
    if (c.left.length) {
      const names = andList(c.left);
      const computer = this.computer();
      lines.push(
        c.left.length === 1
          ? tr(
              'Left as it was: {names} — a backup in the folder of no account on this {computer} that opens with neither passphrase.',
              { names, computer },
            )
          : tr(
              'Left as they were: {names} — backups in the folder of no account on this {computer} that open with neither passphrase.',
              { names, computer },
            ),
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
      if (!change.passphrase) {
        throw new Error(
          tr('The {computer} sent no passphrase. Try again.', { computer: this.computer() }),
        );
      }
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
        this.error.set(this.unfinishedText(e, false));
        this.saved.emit();
      } else {
        this.error.set(tr('Nothing changed: {error}', { error: backupErrorText(e) }));
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
        this.error.set(this.unfinishedText(err, true));
        this.saved.emit();
      } else {
        this.error.set(tr('Nothing changed: {error}', { error: backupErrorText(err) }));
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

  /** A change some backups have already: `typed`, the passphrase in the
   * form; else the one generated. */
  private unfinishedText(e: unknown, typed: boolean): string {
    const stopped = tr('The change did not finish: {error}', { error: backupErrorText(e) });
    const keep = typed
      ? tr(
          'Some backups already open with the passphrase you typed, so keep it: setting it again finishes the change.',
        )
      : tr(
          'Some backups already open with the new passphrase, so keep it: setting it again finishes the change.',
        );
    return `${stopped} ${keep}`;
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
