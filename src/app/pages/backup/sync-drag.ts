import { Injectable, signal } from '@angular/core';

/**
 * What is being dragged on the Backup & sync page:
 * - an account on this computer (onto the backups: back it up);
 * - an account's backup (onto its account: pull; onto another: restore into it);
 * - conversations of a backup, from its browser (onto their own account:
 *   bring them here; onto another account: restore them into it);
 * - a set of a storage (onto an account: bring its conversations back there).
 * An account dropped on a storage's set moves conversations into it.
 */
export type SyncDragItem =
  | { kind: 'account'; account: string; name: string }
  | { kind: 'set'; account: string; name: string }
  | {
      kind: 'sessions';
      /** The account whose backup they are in. */
      set: string;
      setName: string;
      ids: string[];
      /** Some of them Claude Code would delete again (a sheet warns first). */
      prune: boolean;
    }
  | { kind: 'stored'; store: string; storeLabel: string; set: string };

/** Where an item can be dropped. */
export type SyncDropTarget =
  | { kind: 'backups' }
  | { kind: 'account'; account: string }
  | { kind: 'storage'; store: string; set: string };

/** What a drop does, said on the target while the item is over it ('' when
 * it can't be dropped there). */
export function dropText(
  item: SyncDragItem | null,
  target: SyncDropTarget,
  nameOf: (account: string) => string,
): string {
  if (!item) return '';
  if (target.kind === 'backups') return item.kind === 'account' ? `Back up ${item.name}` : '';
  if (target.kind === 'storage') return item.kind === 'account' ? `Move from ${item.name}…` : '';
  const to = nameOf(target.account) || 'this account';
  switch (item.kind) {
    case 'account':
      return '';
    case 'stored':
      return `Bring back into ${to}…`;
    case 'set':
      return item.account === target.account ? `Pull ${item.name}` : `Restore into ${to}…`;
    case 'sessions': {
      const what = item.ids.length === 1 ? 'it' : `these ${item.ids.length}`;
      return item.set === target.account ? `Bring ${what} here` : `Restore into ${to}…`;
    }
  }
}

/** The page's drag state, shared with the backup browser inside it. */
@Injectable()
export class SyncDrag {
  readonly item = signal<SyncDragItem | null>(null);
  /** The target the item is over (its key), for its highlight. */
  readonly over = signal('');

  start(e: DragEvent, item: SyncDragItem, label: string): void {
    this.item.set(item);
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'copyMove';
      // Some browsers start no drag without data.
      e.dataTransfer.setData('text/plain', label);
    }
  }

  end(): void {
    this.item.set(null);
    this.over.set('');
  }
}

/** A drop target's key, for `SyncDrag.over`. */
export function targetKey(t: SyncDropTarget): string {
  switch (t.kind) {
    case 'backups':
      return 'backups';
    case 'account':
      return `account:${t.account}`;
    case 'storage':
      return `storage:${t.store}:${t.set}`;
  }
}
