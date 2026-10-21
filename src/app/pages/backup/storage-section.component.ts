import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  input,
  output,
  signal,
} from '@angular/core';

import { DEFAULT_ACCOUNT } from '../../core/accounts';
import { AccountsService } from '../../core/accounts.service';
import { plural } from '../../core/data-jobs';
import { BackupSuggestion } from '../../core/models';
import {
  StorageSet,
  StorageView,
  accountLocation,
  moveErrorText,
  namesBackupFolder,
  setLocation,
} from '../../core/move-data';
import { StorageService } from '../../core/storage.service';
import { IconComponent } from '../../shared/icon.component';
import { MoveDataSheetComponent, MoveStart } from '../../shared/move-data-sheet.component';
import { shortPath } from '../../shared/util';
import { StorageSheetComponent } from './storage-sheet.component';

interface SetRow {
  set: StorageSet;
  /** "143 conversations · 2 in storage only". */
  facts: string;
  /** The set can be read: Bring back… */
  readable: boolean;
}

interface StoreRow {
  store: StorageView;
  sets: SetRow[];
}

/**
 * Backup & sync › Storage (docs/ACCOUNTS.md §13.6): the backup folder
 * first, then every folder added beside it, each with its sets — *Bring
 * back…* (out of a set into an account) and *Move here…* (from an account
 * into the set; *New set…* into a new one), the conversations archived into a set
 * counted as *in storage only*. *Add storage…*, renaming and removing one
 * (which forgets it and never deletes its files). The desktop and paired
 * phones alike.
 */
@Component({
  selector: 'mc-storage-section',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, MoveDataSheetComponent, StorageSheetComponent],
  template: `
    <div class="shead">
      <h2 id="storage-head">Storage</h2>
      <button class="btn small" type="button" (click)="adding.set(true)">
        <mc-icon name="plus" [size]="14" /> Add storage…
      </button>
    </div>
    @if (error()) {
      <p class="notice error small" role="alert">{{ error() }}</p>
    }
    @if (notice()) {
      <p class="notice ok small" role="status">{{ notice() }}</p>
    }
    <section class="card list stores" aria-labelledby="storage-head">
      @for (r of rows(); track r.store.id) {
        <div class="store">
          <div class="sthead">
            <mc-icon
              class="sticon"
              [name]="r.store.kind === 'folder' ? 'folder' : 'cloud'"
              [size]="16"
            />
            <div class="stmain">
              <span class="stline">
                <strong class="stname">{{ r.store.label }}</strong>
                @if (r.store.primary && !namesBackupFolder(r.store.label)) {
                  <span class="badge">Backup folder</span>
                }
                @if (!r.store.available) {
                  <span class="badge warn">Not available</span>
                }
              </span>
              <span class="mono muted small break">{{ short(r.store.path) }}</span>
            </div>
            <div class="stacts">
              @if (r.store.available) {
                <button
                  class="btn small"
                  type="button"
                  [attr.aria-label]="'Move into a new set in ' + r.store.label"
                  (click)="moveInto(r.store)"
                >
                  <mc-icon name="plus" [size]="14" /> New set…
                </button>
              }
              @if (!r.store.primary) {
                <button
                  class="btn small ghost"
                  type="button"
                  [attr.aria-label]="'Rename ' + r.store.label"
                  (click)="renaming.set(r.store)"
                >
                  <mc-icon name="edit" [size]="14" /> Rename…
                </button>
                <button
                  class="btn small ghost"
                  type="button"
                  [class.danger]="confirming() === r.store.id"
                  [disabled]="busy() === r.store.id"
                  [attr.aria-label]="
                    (confirming() === r.store.id ? 'Tap again to remove ' : 'Remove ') +
                    r.store.label
                  "
                  (click)="remove(r.store)"
                >
                  <mc-icon name="trash" [size]="14" />
                  {{
                    busy() === r.store.id
                      ? 'Removing…'
                      : confirming() === r.store.id
                        ? 'Tap again to remove'
                        : 'Remove'
                  }}
                </button>
              }
            </div>
          </div>
          @if (confirming() === r.store.id) {
            <p class="muted small forget">
              Mother Claude forgets it; its folder and every set in it stay where they are.
            </p>
          }
          <ul class="stsets" [attr.aria-label]="'Sets in ' + r.store.label">
            @for (s of r.sets; track s.set.name) {
              <li class="stset">
                <div class="stsmain">
                  <span class="stsname">{{ s.set.name }}</span>
                  @if (s.facts) {
                    <span class="muted small">{{ s.facts }}</span>
                  }
                </div>
                @if (r.store.available && !s.set.keyMissing) {
                  <div class="stsacts">
                    @if (s.readable) {
                      <button
                        class="btn small"
                        type="button"
                        [attr.aria-label]="'Bring back from ' + r.store.label + ' · ' + s.set.name"
                        (click)="bringBack(r.store, s.set)"
                      >
                        <mc-icon name="download" [size]="14" /> Bring back…
                      </button>
                    }
                    <button
                      class="btn small"
                      type="button"
                      [attr.aria-label]="'Move here: ' + r.store.label + ' · ' + s.set.name"
                      (click)="moveHere(r.store, s.set)"
                    >
                      <mc-icon name="upload" [size]="14" /> Move here…
                    </button>
                  </div>
                }
              </li>
            } @empty {
              <li class="muted small none">
                @if (r.store.available) {
                  No sets in it yet.
                } @else {
                  Its folder is not there now — an unplugged disk, or a cloud folder that is signed
                  out? Its sets show again when it is back.
                }
              </li>
            }
          </ul>
        </div>
      } @empty {
        <p class="muted small none">
          No storage yet. The backup folder (above) is the first one; add others — another cloud's
          folder, a NAS, an external disk — to move conversations there.
        </p>
      }
    </section>
    <p class="muted small foot">
      Every set in every storage opens with the backup passphrase. A set named after an account is
      that account's backup; any other only Move data reads and writes.
    </p>

    <mc-storage-sheet
      [open]="adding() || !!renaming()"
      [renaming]="renaming()"
      [suggestions]="freeSuggestions()"
      (closed)="closeSheet()"
      (saved)="saved($event)"
    />
    <mc-move-data-sheet [open]="!!moveStart()" [start]="moveStart()" (closed)="moveClosed()" />
  `,
  styles: [
    `
      :host {
        display: block;
      }
      .shead {
        display: flex;
        align-items: flex-end;
        justify-content: space-between;
        flex-wrap: wrap;
        gap: 0.4rem;
        margin: 1.4rem 0 0.55rem;
      }
      .shead h2 {
        margin: 0;
      }
      .notice {
        margin: 0 0 0.5rem;
      }
      .stores {
        padding-top: 0;
        padding-bottom: 0;
      }
      .store {
        padding: 0.75rem 0 0.5rem;
      }
      .store + .store {
        border-top: 1px solid var(--border);
      }
      .sthead {
        display: flex;
        align-items: flex-start;
        flex-wrap: wrap;
        gap: 0.4rem 0.6rem;
      }
      .sticon {
        margin-top: 0.15rem;
        color: var(--accent-text);
      }
      .stmain {
        flex: 1 1 14rem;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.1rem;
      }
      .stline {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.35rem;
      }
      .stname {
        overflow-wrap: anywhere;
      }
      .badge.warn {
        color: var(--amber);
      }
      .stacts,
      .stsacts {
        display: flex;
        flex-wrap: wrap;
        gap: 0.3rem;
        margin-left: auto;
      }
      .forget {
        margin: 0.4rem 0 0 1.6rem;
      }
      .stsets {
        margin: 0.45rem 0 0 1.6rem;
        padding: 0;
        list-style: none;
      }
      .stset {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0.35rem 0.6rem;
        padding: 0.45rem 0;
        border-top: 1px solid var(--border);
      }
      .stsmain {
        flex: 1 1 10rem;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.05rem;
      }
      .stsname {
        font-weight: 500;
        overflow-wrap: anywhere;
      }
      .none {
        margin: 0.6rem 0;
      }
      .foot {
        margin: 0.5rem 0 0;
        line-height: 1.5;
      }
      @media (max-width: 767px) {
        .stacts {
          flex: 1 1 100%;
          margin-left: 1.6rem;
        }
        .stsets {
          margin-left: 0;
        }
        .stsacts {
          flex: 1 1 100%;
          margin-left: 0;
        }
      }
    `,
  ],
})
export class StorageSectionComponent {
  private storage = inject(StorageService);
  private accounts = inject(AccountsService);

  /** The backup folder's suggestions, offered when adding a storage too. */
  readonly suggestions = input<BackupSuggestion[]>([]);
  /** A storage was added, renamed or removed, or Move data closed. */
  readonly changed = output<void>();

  protected readonly adding = signal(false);
  protected readonly renaming = signal<StorageView | null>(null);
  protected readonly confirming = signal<string | null>(null);
  protected readonly busy = signal<string | null>(null);
  protected readonly error = signal('');
  protected readonly notice = signal('');
  protected readonly moveStart = signal<MoveStart | null>(null);
  private confirmTimer?: ReturnType<typeof setTimeout>;

  /** The suggested folders that are no storage yet. */
  protected readonly freeSuggestions = computed(() => {
    const taken = new Set((this.storage.list() ?? []).map((s) => s.path.replace(/\/+$/, '')));
    return this.suggestions().filter((s) => !taken.has(s.path.replace(/\/+$/, '')));
  });
  protected readonly rows = computed<StoreRow[]>(() =>
    (this.storage.list() ?? []).map((store) => ({
      store,
      sets: store.sets.map((set) => ({
        set,
        facts: this.facts(set),
        readable: set.exists && !set.keyMissing,
      })),
    })),
  );

  protected short(p: string): string {
    return shortPath(p);
  }

  protected namesBackupFolder(label: string): boolean {
    return namesBackupFolder(label);
  }

  protected closeSheet(): void {
    this.adding.set(false);
    this.renaming.set(null);
  }

  protected saved(message: string): void {
    this.closeSheet();
    this.error.set('');
    this.notice.set(message);
    this.changed.emit();
  }

  /** Out of a set: into the account it is named after (else the Default). */
  protected bringBack(store: StorageView, set: StorageSet): void {
    const into = set.account && this.accounts.byId(set.account) ? set.account : DEFAULT_ACCOUNT;
    this.moveStart.set({
      from: setLocation(store.id, set.name),
      to: accountLocation(into),
      what: 'all',
    });
  }

  /** Into a set: from the account it is named after (else the Default). */
  protected moveHere(store: StorageView, set: StorageSet): void {
    const from = set.account && this.accounts.byId(set.account) ? set.account : DEFAULT_ACCOUNT;
    this.moveStart.set({
      from: accountLocation(from),
      to: setLocation(store.id, set.name),
      what: 'sessions',
    });
  }

  /** Into a new set of a storage. */
  protected moveInto(store: StorageView): void {
    this.moveStart.set({
      from: accountLocation(DEFAULT_ACCOUNT),
      to: setLocation(store.id, ''),
      what: 'sessions',
    });
  }

  protected moveClosed(): void {
    this.moveStart.set(null);
    this.changed.emit();
  }

  /** Forget a storage, confirmed with a second tap. */
  protected async remove(store: StorageView): Promise<void> {
    if (this.busy()) return;
    if (this.confirming() !== store.id) {
      this.confirming.set(store.id);
      clearTimeout(this.confirmTimer);
      this.confirmTimer = setTimeout(() => this.confirming.set(null), 6000);
      return;
    }
    clearTimeout(this.confirmTimer);
    this.busy.set(store.id);
    this.error.set('');
    this.notice.set('');
    try {
      await this.storage.remove(store.id);
      this.notice.set(`${store.label} is forgotten; its files are where they were.`);
      this.changed.emit();
    } catch (e) {
      this.error.set(`${store.label}: ${moveErrorText(e)}`);
    } finally {
      this.busy.set(null);
      this.confirming.set(null);
    }
  }

  private facts(set: StorageSet): string {
    const parts: string[] = [];
    if (set.keyMissing) parts.push('its key is not in the folder here');
    else if (!set.exists) parts.push('empty');
    else if (set.sessions !== undefined) parts.push(plural(set.sessions, 'conversation'));
    if (set.archived) parts.push(`${set.archived} in storage only`);
    if (set.account) {
      const name = this.accounts.byId(set.account)?.name;
      parts.push(name ? `${name}'s backup` : 'an account’s backup');
    } else {
      parts.push('only Move data uses it');
    }
    return parts.join(' · ');
  }
}
