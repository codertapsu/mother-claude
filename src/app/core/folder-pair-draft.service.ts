import { Injectable, signal } from '@angular/core';

import { lastSegment, trimSeparators } from './paths';

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

/** The folder a project is in, in its own form: `/Users/m/dev/app` →
 * `/Users/m/dev`, `D:\dev\app` → `D:\dev`, `D:\app` → `D:\`; a root is its
 * own. */
function folderAbove(path: string): string {
  const p = trimSeparators(path.trim());
  const name = lastSegment(p);
  return name ? trimSeparators(p.slice(0, p.length - name.length)) : p;
}
