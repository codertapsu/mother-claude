import { Injectable, signal } from '@angular/core';

import { ApiError } from './api.service';
import { PairMachine } from './models';

/**
 * A `409 folder_elsewhere` from continuing a conversation (docs/BACKUP.md
 * "Continuing a conversation from another computer"): the folder it was
 * recorded in — another computer's, which no folder pair maps, or one gone
 * from this machine — folders here that look like it, and the machines of
 * the backups that write paths as that folder does (to pair the folders
 * with, so the next ones need no question).
 */
export interface FolderQuestion {
  /** The desktop's sentence. */
  message: string;
  recorded: string;
  /** The folder is another computer's (not one gone from here). */
  otherMachine: boolean;
  /** Folders here that may be it, the likeliest first. */
  suggestions: string[];
  machines: PairMachine[];
}

/** The question in a refused continue, or null for any other refusal. */
export function folderQuestionOf(e: unknown): FolderQuestion | null {
  if (!(e instanceof ApiError) || e.code !== 'folder_elsewhere') return null;
  const x = e.extra;
  const text = (v: unknown) => (typeof v === 'string' ? v : '');
  const list = (v: unknown): unknown[] => (Array.isArray(v) ? v : []);
  const machines: PairMachine[] = [];
  for (const raw of list(x['machines'])) {
    const m = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {};
    const id = text(m['id']);
    if (!id) continue;
    const os = text(m['os']);
    machines.push({ id, name: text(m['name']), ...(os ? { os } : {}) });
  }
  return {
    message: e.message,
    recorded: text(x['recorded']),
    otherMachine: x['otherMachine'] === true,
    suggestions: list(x['suggestions']).map(text).filter(Boolean),
    machines,
  };
}

/**
 * Asks where a conversation works on this machine, from wherever it was
 * continued; the sheet that asks is one for the whole app (`mc-folder-choice`
 * in the shell), so every continue gets the same question.
 */
@Injectable({ providedIn: 'root' })
export class FolderChoiceService {
  /** The question on screen, one at a time. */
  readonly question = signal<FolderQuestion | null>(null);
  private settle: ((folder: string | null) => void) | null = null;

  /** The folder the user chose, or null when they closed the question. A
   * newer question replaces an open one, which answers null. */
  ask(q: FolderQuestion): Promise<string | null> {
    this.answer(null);
    this.question.set(q);
    return new Promise((resolve) => (this.settle = resolve));
  }

  /** The sheet's answer (null: closed). */
  answer(folder: string | null): void {
    const settle = this.settle;
    this.settle = null;
    this.question.set(null);
    settle?.(folder);
  }
}
