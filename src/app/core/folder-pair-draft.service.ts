import { Injectable, signal } from '@angular/core';

import { folderAbove } from './paths';

/** A folder pair to offer: another machine and the folder there. */
export interface PairDraft {
  machine: string;
  otherFolder: string;
}

/**
 * A pull or restore that placed projects under another machine's path (no
 * folder pair maps them) offers a pair from its report: the draft fills the
 * Folder pairs form on the backup page (docs/BACKUP.md "Folder pairs").
 */
@Injectable({ providedIn: 'root' })
export class FolderPairDraftService {
  readonly draft = signal<PairDraft | null>(null);

  /** Offer a pair for a project of `machine` at `folder` (the folder above
   * it is what is usually paired). */
  offer(machine: string, folder: string): void {
    this.draft.set({ machine, otherFolder: folderAbove(folder) });
    document.getElementById('folder-pairs')?.scrollIntoView({ behavior: 'smooth' });
  }

  take(): PairDraft | null {
    const d = this.draft();
    this.draft.set(null);
    return d;
  }
}
