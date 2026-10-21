import { Injectable, inject, signal } from '@angular/core';

import { ApiService } from './api.service';
import { BACKUP_STORAGE, StorageView, moveErrorText, normalizeStorages } from './move-data';
import { RealtimeService } from './realtime.service';

/**
 * The storages (docs/ACCOUNTS.md §13): the backup folder and every folder
 * added beside it, each with its sets — for the Move data sheet, the job
 * rows' titles and Backup & sync. Read from `GET /backup` when a view needs
 * them (the Backup page hands its own reads over), and changed from the
 * desktop or a paired phone alike.
 */
@Injectable({ providedIn: 'root' })
export class StorageService {
  private api = inject(ApiService);
  private realtime = inject(RealtimeService);

  /** The storages, the backup folder first (null until read). */
  readonly list = signal<StorageView[] | null>(null);
  /** The last failed read, one sentence ('' when fine). */
  readonly loadError = signal('');

  private inflight?: Promise<void>;
  private again = false;

  constructor() {
    this.realtime.refetch$.subscribe(() => {
      if (this.list()) void this.refresh();
    });
  }

  /** A `GET /backup` answer read elsewhere. */
  accept(raw: unknown): void {
    this.list.set(normalizeStorages(raw));
    this.loadError.set('');
  }

  ensureLoaded(): Promise<void> {
    return this.list() ? Promise.resolve() : this.refresh();
  }

  /** Read them again; a call while a read is in flight reads once more
   * after it (its answer may predate a change). */
  refresh(): Promise<void> {
    if (this.inflight) {
      this.again = true;
      return this.inflight;
    }
    const run = async () => {
      do {
        this.again = false;
        try {
          this.accept(await this.api.getBackup());
        } catch (e) {
          this.loadError.set(moveErrorText(e));
        }
      } while (this.again);
      this.inflight = undefined;
    };
    this.inflight = run();
    return this.inflight;
  }

  byId(id: string): StorageView | undefined {
    return this.list()?.find((s) => s.id === id);
  }

  /** A storage's label, for a job's title (undefined when unknown). */
  labelOf(id: string): string | undefined {
    return this.byId(id)?.label ?? (id === BACKUP_STORAGE ? 'the backup folder' : undefined);
  }

  // Each change throws the server's refusal for the caller to show, and
  // reads the list again after it.

  async add(path: string, label?: string): Promise<void> {
    await this.api.addStorage({ path, ...(label ? { label } : {}) });
    await this.refresh();
  }

  async rename(id: string, label: string): Promise<void> {
    await this.api.renameStorage(id, label);
    await this.refresh();
  }

  /** Forget it: its files stay where they are. */
  async remove(id: string): Promise<void> {
    await this.api.removeStorage(id);
    await this.refresh();
  }
}
