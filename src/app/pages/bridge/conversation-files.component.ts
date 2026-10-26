import {
  ChangeDetectionStrategy,
  Component,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
} from '@angular/core';

import { DEFAULT_ACCOUNT } from '../../core/accounts';
import { AccountsService } from '../../core/accounts.service';
import { ApiService } from '../../core/api.service';
import { AuthService } from '../../core/auth.service';
import {
  SessionFiles,
  deleteErrorText,
  desktopOs,
  normalizeSessionFiles,
  revealLabel,
} from '../../core/delete-data';
import { JobView } from '../../core/models';
import { CopyButtonComponent } from '../../shared/copy-button.component';
import {
  DeleteDataSheetComponent,
  DeleteStart,
  deletedIds,
} from '../../shared/delete-data-sheet.component';
import { IconComponent } from '../../shared/icon.component';
import { shortPath } from '../../shared/util';

/**
 * The API screen's picked conversation on disk (docs/ACCOUNTS.md §14.4):
 * where its transcript is, with Copy path and, on the desktop, the reveal;
 * and Delete…, the delete dialog for it. A conversation open in the API's
 * host comes back skipped as running, and the dialog says to close it first.
 */
@Component({
  selector: 'mc-conversation-files',
  imports: [CopyButtonComponent, DeleteDataSheetComponent, IconComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="files">
      @if (files(); as f) {
        <span class="lbl">Transcript</span>
        <span class="mono path" [title]="f.transcript">{{ short(f.transcript) }}</span>
      }
      <span class="acts">
        @if (files(); as f) {
          <mc-copy-button
            [text]="f.transcript"
            label="Copy path"
            ariaLabel="Copy the transcript's path"
          />
          @if (isDesktop()) {
            <button class="btn small" type="button" [disabled]="revealing()" (click)="reveal()">
              <mc-icon name="folder" [size]="14" /> {{ revealText }}
            </button>
          }
        }
        <button class="btn small danger" type="button" (click)="openDelete()">
          <mc-icon name="trash" [size]="14" /> Delete…
        </button>
      </span>
      @if (error()) {
        <span class="error-text small" role="alert">{{ error() }}</span>
      }
    </div>
    <!-- Only while open: the API screen's other dialogs come first. -->
    @if (deleteStart(); as start) {
      <mc-delete-data-sheet [open]="true" [start]="start" (closed)="closed($event)" />
    }
  `,
  styles: [
    `
      .files {
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        gap: 0.35rem;
        min-width: 0;
      }
      .lbl {
        font-size: 0.78rem;
        text-transform: uppercase;
        letter-spacing: 0.04em;
        opacity: 0.6;
      }
      .path {
        font-size: 0.85em;
        overflow-wrap: anywhere;
      }
      .acts {
        display: flex;
        flex-wrap: wrap;
        gap: 0.35rem;
      }
    `,
  ],
})
export class ConversationFilesComponent {
  private api = inject(ApiService);
  private accounts = inject(AccountsService);
  protected readonly isDesktop = inject(AuthService).isDesktop;

  /** The conversation (a session id; a running one's thread id). */
  readonly id = input.required<string>();
  /** What to call it in the dialog. */
  readonly title = input('');
  /** The account it is recorded as (absent: not known). */
  readonly account = input<string | undefined>(undefined);
  /** It was deleted (and not undone). */
  readonly deleted = output<string>();

  /** Where its files are (null: not on this Mac yet, or not known). */
  protected readonly files = signal<SessionFiles | null>(null);
  protected readonly revealText = revealLabel(desktopOs(navigator.userAgent));
  protected readonly revealing = signal(false);
  protected readonly error = signal('');
  protected readonly deleteStart = signal<DeleteStart | null>(null);
  protected readonly short = shortPath;

  constructor() {
    effect(() => {
      const id = this.id();
      untracked(() => void this.load(id));
    });
  }

  /** The detail says where its files are; a conversation that has not
   * written a transcript yet has none. */
  private async load(id: string): Promise<void> {
    this.files.set(null);
    this.error.set('');
    try {
      const detail = await this.api.getSession(id);
      if (this.id() === id) this.files.set(normalizeSessionFiles(detail.files));
    } catch {
      // Not indexed (yet): no path to show.
    }
  }

  protected async reveal(): Promise<void> {
    if (this.revealing()) return;
    this.revealing.set(true);
    this.error.set('');
    try {
      await this.api.revealSession(this.id());
    } catch (e) {
      this.error.set(deleteErrorText(e));
    } finally {
      this.revealing.set(false);
    }
  }

  protected openDelete(): void {
    const id = this.id();
    const from = this.account() ?? (this.accounts.multi() ? undefined : DEFAULT_ACCOUNT);
    this.deleteStart.set({
      what: 'this conversation',
      groups: [{ ...(from ? { from } : {}), sessions: [id] }],
      known: [{ id, title: this.title(), name: '', projectName: '' }],
    });
  }

  protected closed(jobs: JobView[]): void {
    this.deleteStart.set(null);
    const id = this.id();
    if (deletedIds(jobs).has(id)) this.deleted.emit(id);
  }
}
