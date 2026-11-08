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
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { BackupSuggestion } from '../../core/models';
import { samePath } from '../../core/paths';
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
import { SyncDrag, SyncDropTarget, dropText, targetKey } from './sync-drag';

interface SetRow {
  set: StorageSet;
  /** "143 conversations · 2 in storage only". */
  facts: string;
  /** The set can be read: Bring back… */
  readable: boolean;
}

interface StoreRow {
  store: StorageView;
  /** Its name as shown (the backup folder's own, translated). */
  name: string;
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
  imports: [IconComponent, MoveDataSheetComponent, StorageSheetComponent, TPipe],
  template: `
    <div class="shead">
      <h2 id="storage-head">{{ 'Storage' | t }}</h2>
      <button class="btn small" type="button" (click)="adding.set(true)">
        <mc-icon name="plus" [size]="14" /> {{ 'Add storage…' | t }}
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
                <strong class="stname">{{ r.name }}</strong>
                @if (r.store.primary && !namesBackupFolder(r.store.label)) {
                  <span class="badge">{{ 'Backup folder' | t }}</span>
                }
                @if (!r.store.available) {
                  <span class="badge warn">{{ 'Not available' | t }}</span>
                }
              </span>
              <span class="mono muted small break">{{ short(r.store.path) }}</span>
            </div>
            <div class="stacts">
              @if (r.store.available) {
                <button
                  class="btn small"
                  type="button"
                  [attr.aria-label]="'Move into a new set in {name}' | t: { name: r.name }"
                  (click)="moveInto(r.store)"
                >
                  <mc-icon name="plus" [size]="14" /> {{ 'New set…' | t }}
                </button>
              }
              @if (!r.store.primary) {
                <button
                  class="btn small ghost"
                  type="button"
                  [attr.aria-label]="'Rename {name}' | t: { name: r.name }"
                  (click)="renaming.set(r.store)"
                >
                  <mc-icon name="edit" [size]="14" /> {{ 'Rename…' | t }}
                </button>
                <button
                  class="btn small ghost"
                  type="button"
                  [class.danger]="confirming() === r.store.id"
                  [disabled]="busy() === r.store.id"
                  [attr.aria-label]="
                    confirming() === r.store.id
                      ? ('Tap again to remove {name}' | t: { name: r.name })
                      : ('Remove {name}' | t: { name: r.name })
                  "
                  (click)="remove(r.store)"
                >
                  <mc-icon name="trash" [size]="14" />
                  {{
                    busy() === r.store.id
                      ? ('Removing…' | t)
                      : confirming() === r.store.id
                        ? ('Tap again to remove' | t)
                        : ('Remove' | t)
                  }}
                </button>
              }
            </div>
          </div>
          @if (confirming() === r.store.id) {
            <p class="muted small forget">
              {{
                'Mother Claude forgets it; its folder and every set in it stay where they are.' | t
              }}
            </p>
          }
          <ul class="stsets" [attr.aria-label]="'Sets in {name}' | t: { name: r.name }">
            @for (s of r.sets; track s.set.name) {
              <li
                class="stset"
                [class.can]="!!dropLabel(r.store, s.set)"
                [class.over]="drag?.over() === key(r.store, s.set)"
                [attr.draggable]="canDrag(r.store, s)"
                [attr.aria-label]="r.name + ' · ' + s.set.name"
                (dragstart)="dragSet($event, r.store, s)"
                (dragend)="drag?.end()"
                (dragover)="over($event, r.store, s.set)"
                (dragleave)="leave(r.store, s.set)"
                (drop)="drop($event, r.store, s.set)"
              >
                @if (canDrag(r.store, s)) {
                  <span class="grip" aria-hidden="true" [title]="'Drag onto an account' | t"
                    >⠿</span
                  >
                }
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
                        [attr.aria-label]="
                          'Bring back from {store} · {set}' | t: { store: r.name, set: s.set.name }
                        "
                        (click)="bringBack(r.store, s.set)"
                      >
                        <mc-icon name="download" [size]="14" /> {{ 'Bring back…' | t }}
                      </button>
                    }
                    <button
                      class="btn small"
                      type="button"
                      [attr.aria-label]="
                        'Move here: {store} · {set}' | t: { store: r.name, set: s.set.name }
                      "
                      (click)="moveHere(r.store, s.set)"
                    >
                      <mc-icon name="upload" [size]="14" /> {{ 'Move here…' | t }}
                    </button>
                  </div>
                }
                @if (dropLabel(r.store, s.set); as label) {
                  <span class="droptag small">{{ label }}</span>
                }
              </li>
            } @empty {
              <li class="muted small none">
                @if (r.store.available) {
                  {{ 'No sets in it yet.' | t }}
                } @else {
                  {{
                    'Its folder is not there now — an unplugged disk, or a cloud folder that is signed out? Its sets show again when it is back.'
                      | t
                  }}
                }
              </li>
            }
          </ul>
        </div>
      } @empty {
        <p class="muted small none">
          {{
            "No storage yet. The backup folder (above) is the first one; add others — another cloud's folder, a NAS, an external disk — to move conversations there."
              | t
          }}
        </p>
      }
    </section>
    <p class="muted small foot">
      {{
        "Every set in every storage opens with the backup passphrase. A set named after an account is that account's backup; any other only Move data reads and writes."
          | t
      }}
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
      .stset {
        position: relative;
        border-radius: var(--radius-sm);
        transition:
          background 0.15s,
          box-shadow 0.15s;
      }
      .stset[draggable='true'] {
        cursor: grab;
      }
      .stset.can {
        box-shadow: 0 0 0 2px var(--accent);
      }
      .stset.over {
        background: var(--accent-soft);
      }
      .grip {
        color: var(--faint);
        align-self: center;
        user-select: none;
      }
      .droptag {
        position: absolute;
        right: 0.5rem;
        top: -0.6rem;
        padding: 0.1rem 0.5rem;
        border-radius: 999px;
        background: var(--accent);
        color: var(--on-accent);
        font-weight: 600;
        pointer-events: none;
      }
      @media (pointer: coarse) {
        .grip {
          display: none;
        }
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
  /** The Backup & sync page's drag and drop, when it is on that page. */
  protected readonly drag = inject(SyncDrag, { optional: true });

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
    const taken = (this.storage.list() ?? []).map((s) => s.path);
    return this.suggestions().filter((s) => !taken.some((t) => samePath(t, s.path)));
  });
  protected readonly rows = computed<StoreRow[]>(() =>
    (this.storage.list() ?? []).map((store) => ({
      store,
      name: this.nameOf(store),
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

  /** A storage's name: its label, the backup folder's in the language
   * shown (the desktop names it "Backup folder", and it can't be renamed). */
  private nameOf(store: StorageView): string {
    return store.primary && namesBackupFolder(store.label) ? tr('Backup folder') : store.label;
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

  /** Out of a set into `account` (a set dropped on an account's lane). */
  bringBackInto(storeId: string, setName: string, account: string): void {
    const store = (this.storage.list() ?? []).find((s) => s.id === storeId);
    const set = store?.sets.find((x) => x.name === setName);
    if (!store || !set) return;
    this.moveStart.set({
      from: setLocation(store.id, set.name),
      to: accountLocation(account),
      what: 'all',
    });
  }

  // ─────────────────────────────── Drag and drop (on the Backup & sync page)

  protected key(store: StorageView, set: StorageSet): string {
    return targetKey(this.target(store, set));
  }

  /** A set that can be read is dragged onto an account to bring it back. */
  protected canDrag(store: StorageView, row: SetRow): boolean {
    return !!this.drag && store.available && row.readable;
  }

  protected dragSet(e: DragEvent, store: StorageView, row: SetRow): void {
    if (!this.drag || !this.canDrag(store, row)) {
      e.preventDefault();
      return;
    }
    this.drag.start(
      e,
      { kind: 'stored', store: store.id, storeLabel: store.label, set: row.set.name },
      `${this.nameOf(store)} · ${row.set.name}`,
    );
  }

  /** What dropping the dragged item on this set does ('' when it can't). */
  protected dropLabel(store: StorageView, set: StorageSet): string {
    if (!this.drag || !store.available || set.keyMissing) return '';
    return dropText(this.drag.item(), this.target(store, set), () => '');
  }

  protected over(e: DragEvent, store: StorageView, set: StorageSet): void {
    if (!this.dropLabel(store, set)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    this.drag?.over.set(this.key(store, set));
  }

  protected leave(store: StorageView, set: StorageSet): void {
    if (this.drag?.over() === this.key(store, set)) this.drag.over.set('');
  }

  /** An account dropped here: Move data from it into this set, preset. */
  protected drop(e: DragEvent, store: StorageView, set: StorageSet): void {
    const item = this.drag?.item() ?? null;
    const label = this.dropLabel(store, set);
    this.drag?.end();
    if (!label || item?.kind !== 'account') return;
    e.preventDefault();
    this.moveStart.set({
      from: accountLocation(item.account),
      to: setLocation(store.id, set.name),
      what: 'sessions',
    });
  }

  private target(store: StorageView, set: StorageSet): SyncDropTarget {
    return { kind: 'storage', store: store.id, set: set.name };
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
      this.notice.set(
        tr('{name} is forgotten; its files are where they were.', { name: this.nameOf(store) }),
      );
      this.changed.emit();
    } catch (e) {
      this.error.set(`${this.nameOf(store)}: ${moveErrorText(e)}`);
    } finally {
      this.busy.set(null);
      this.confirming.set(null);
    }
  }

  private facts(set: StorageSet): string {
    const parts: string[] = [];
    if (set.keyMissing) parts.push(tr('its key is not in the folder here'));
    else if (!set.exists) parts.push(tr('empty'));
    else if (set.sessions !== undefined) parts.push(plural(set.sessions, 'conversation'));
    if (set.archived) parts.push(tr('{n} in storage only', { n: set.archived }));
    if (set.account) {
      const name = this.accounts.byId(set.account)?.name;
      parts.push(name ? tr("{name}'s backup", { name }) : tr('an account’s backup'));
    } else {
      parts.push(tr('only Move data uses it'));
    }
    return parts.join(' · ');
  }
}
