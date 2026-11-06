import { ChangeDetectionStrategy, Component, computed, inject, input, output } from '@angular/core';

import { AuthService } from '../../core/auth.service';
import { DANGEROUS_REASON } from '../../core/move-data';
import { BackupSet } from '../../core/models';
import { machineNoun } from '../../core/paths';
import { SheetComponent } from '../../shared/sheet.component';

/**
 * Force backup's confirmation (docs/BACKUP.md "Force backup, force pull"):
 * what gets replaced in the backup (the backup's copy of every session this
 * computer has, wherever the two differ — also another machine's newer or
 * changed one), what stays (sessions only in the backup; running ones are skipped),
 * where the replaced copies go (the folder keeps them seven days) and what
 * the other machine sees next. It needs the dangerous right, as deleting does;
 * without it the dialog says why and the button stays off. The page starts
 * the backup.
 */
@Component({
  selector: 'mc-force-backup-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [SheetComponent],
  template: `
    <mc-sheet [open]="!!set()" [title]="title()" (closed)="closed.emit()">
      @if (set(); as s) {
        <div class="notice warn small what" role="alert">
          <p>
            This {{ mac() }}'s copy of every session it has replaces the backup's copy of
            <strong>{{ s.name }}</strong> wherever the two differ — also where another machine
            backed up a newer copy, or one that changed there too.
          </p>
          <p>
            Sessions only in the backup stay as they are, and sessions running here are skipped.
            Each copy it replaces stays in the backup folder for seven days.
          </p>
          <p>
            The other machine keeps its own copy: its next pull puts this {{ mac() }}'s beside it,
            or replaces it when its copy is where this one came from. A force pull there makes its
            copy this {{ mac() }}'s.
          </p>
        </div>
        @if (!canDangerous()) {
          <p class="small blocked">{{ dangerousReason }}</p>
        }
        <div class="row actions">
          <button
            class="btn danger"
            type="button"
            [disabled]="busy() || !canDangerous()"
            (click)="confirmed.emit(s.account)"
          >
            {{ busy() ? 'Starting…' : 'Force backup' }}
          </button>
          <button class="btn ghost" type="button" [disabled]="busy()" (click)="closed.emit()">
            Cancel
          </button>
        </div>
      }
    </mc-sheet>
  `,
  styles: [
    `
      .what p {
        margin: 0 0 0.4rem;
        line-height: 1.5;
      }
      .what p:last-child {
        margin-bottom: 0;
      }
      .blocked {
        color: var(--amber);
        margin: 0.6rem 0 0;
      }
      .actions {
        margin-top: 0.9rem;
      }
    `,
  ],
})
export class ForceBackupSheetComponent {
  /** A force backup needs the dangerous right (docs/ACCOUNTS.md §13.9). */
  protected readonly canDangerous = inject(AuthService).canDangerous;
  private readonly hostOs = inject(AuthService).hostOs;
  /** The desktop's computer: "this Mac", "this PC". */
  protected readonly mac = computed(() => machineNoun(this.hostOs()));
  protected readonly dangerousReason = DANGEROUS_REASON;

  /** The set to force-back up (null: closed). */
  readonly set = input<BackupSet | null>(null);
  /** The page is starting it. */
  readonly busy = input(false);
  readonly closed = output<void>();
  /** "Force backup": the account whose set it is. */
  readonly confirmed = output<string>();

  protected readonly title = computed(() => {
    const s = this.set();
    return s ? `Force backup of ${s.name}` : 'Force backup';
  });
}
