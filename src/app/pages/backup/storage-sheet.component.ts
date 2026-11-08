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

import { AuthService } from '../../core/auth.service';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { BackupSuggestion } from '../../core/models';
import {
  StorageView,
  moveErrorText,
  storageKindOf,
  storagePathProblem,
} from '../../core/move-data';
import { folderExample, lastSegment } from '../../core/paths';
import { StorageService } from '../../core/storage.service';
import { IconComponent } from '../../shared/icon.component';
import { SheetComponent } from '../../shared/sheet.component';
import { shortPath } from '../../shared/util';

/** Longest label (the server's rule). */
const LABEL_MAX = 40;

/** The clouds a storage's folder is in, by name (names, never translated). */
const KIND_NAMES: Record<string, string> = {
  icloud: 'iCloud Drive',
  gdrive: 'Google Drive',
  onedrive: 'OneDrive',
  dropbox: 'Dropbox',
};

/**
 * Add a storage (a folder that holds sets beside the backup folder: another
 * cloud's folder, a NAS, an external disk) — from the same suggestions as
 * the backup folder, or a typed path — with a label; or rename one
 * (`renaming`). The desktop and paired phones alike.
 */
@Component({
  selector: 'mc-storage-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, IconComponent, SheetComponent, TPipe],
  template: `
    <mc-sheet
      [open]="open()"
      [title]="renaming() ? ('Rename storage' | t) : ('Add storage' | t)"
      (closed)="closed.emit()"
    >
      @if (open()) {
        <form class="form" (submit)="save($event)">
          @if (!renaming()) {
            <p class="muted small lede">
              {{
                "A folder for sets beside the backup folder: another cloud's folder, a NAS, an external disk. Every set in it opens with the backup passphrase."
                  | t
              }}
            </p>
            @if (suggestions().length) {
              <div class="sugg" role="group" [attr.aria-label]="'Suggested folders' | t">
                <span class="muted small">{{ 'Suggested:' | t }}</span>
                @for (s of suggestions(); track s.path) {
                  <button
                    class="btn small"
                    type="button"
                    [disabled]="busy()"
                    [attr.aria-pressed]="s.path === path()"
                    [title]="s.path"
                    (click)="useSuggestion(s)"
                  >
                    <mc-icon name="cloud" [size]="14" /> {{ s.label }}
                  </button>
                }
              </div>
            }
            <label class="field">
              <span class="field-label">{{ 'Folder' | t }}</span>
              <input
                name="path"
                [placeholder]="pathPlaceholder()"
                spellcheck="false"
                autocapitalize="off"
                autocomplete="off"
                [disabled]="busy()"
                [ngModel]="path()"
                (ngModelChange)="path.set($event); error.set('')"
              />
            </label>
            @if (path().trim() && pathProblem()) {
              <p class="error-text small">{{ pathProblem() }}</p>
            }
          }
          <label class="field">
            <span class="field-label">{{ 'Label' | t }}</span>
            <input
              name="label"
              autocomplete="off"
              [attr.maxlength]="labelMax"
              [placeholder]="labelHint()"
              [disabled]="busy()"
              [ngModel]="label()"
              (ngModelChange)="label.set($event); error.set('')"
            />
          </label>
          @if (error()) {
            <p class="notice error small" role="alert">{{ error() }}</p>
          }
          <div class="row">
            <button class="btn primary" type="submit" [disabled]="!canSave() || busy()">
              {{ busy() ? ('Saving…' | t) : renaming() ? ('Save' | t) : ('Add storage' | t) }}
            </button>
            <button class="btn ghost" type="button" [disabled]="busy()" (click)="closed.emit()">
              {{ 'Cancel' | t }}
            </button>
          </div>
        </form>
      }
    </mc-sheet>
  `,
  styles: [
    `
      .form {
        display: flex;
        flex-direction: column;
        gap: 0.7rem;
      }
      .lede {
        margin: 0;
        line-height: 1.5;
      }
      .sugg {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.35rem;
      }
      .error-text,
      .notice {
        margin: -0.3rem 0 0;
      }
    `,
  ],
})
export class StorageSheetComponent {
  private storage = inject(StorageService);
  /** The desktop's system: the folder is typed its way. */
  private readonly hostOs = inject(AuthService).hostOs;

  readonly open = input(false);
  /** The storage to rename (null: add one). */
  readonly renaming = input<StorageView | null>(null);
  /** The backup folder's suggestions (iCloud Drive, Google Drive, …). */
  readonly suggestions = input<BackupSuggestion[]>([]);
  readonly closed = output<void>();
  /** Added or renamed: the new list is read. */
  readonly saved = output<string>();

  protected readonly labelMax = LABEL_MAX;
  protected readonly path = signal('');
  protected readonly label = signal('');
  protected readonly busy = signal(false);
  protected readonly error = signal('');

  protected readonly pathProblem = computed(() => storagePathProblem(this.path(), this.hostOs()));
  protected readonly pathPlaceholder = computed(() => folderExample(this.hostOs(), 'storage'));
  /** What it is called when no label is typed. */
  protected readonly labelHint = computed(() => {
    const p = this.path().trim();
    if (!p) return tr('Its name here');
    const kind = storageKindOf(p);
    return KIND_NAMES[kind] ?? (lastSegment(p) || tr('Folder'));
  });
  protected readonly canSave = computed(() => {
    const label = this.label().trim();
    if ([...label].length > LABEL_MAX) return false;
    const r = this.renaming();
    if (r) return !!label && label !== r.label;
    return !this.pathProblem();
  });

  constructor() {
    // Every opening starts over.
    effect(() => {
      if (!this.open()) return;
      const r = this.renaming();
      untracked(() => {
        this.path.set('');
        this.label.set(r?.label ?? '');
        this.busy.set(false);
        this.error.set('');
      });
    });
  }

  protected useSuggestion(s: BackupSuggestion): void {
    this.path.set(s.path);
    if (!this.label().trim()) this.label.set(s.label.replace(/\s*\(.*\)$/, ''));
    this.error.set('');
  }

  protected async save(e: Event): Promise<void> {
    e.preventDefault();
    if (!this.canSave() || this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    const label = this.label().trim();
    const r = this.renaming();
    try {
      if (r) {
        await this.storage.rename(r.id, label);
        this.saved.emit(tr('It is called {name} now.', { name: label }));
      } else {
        await this.storage.add(this.path().trim(), label || undefined);
        this.saved.emit(
          tr('{name} is a storage now ({path}).', {
            name: label || this.labelHint(),
            path: shortPath(this.path().trim()),
          }),
        );
      }
    } catch (err) {
      this.error.set(moveErrorText(err));
    } finally {
      this.busy.set(false);
    }
  }
}
