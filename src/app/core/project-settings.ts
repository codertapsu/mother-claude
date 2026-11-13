import { Injectable, signal } from '@angular/core';

import { RootEntry, RootMachine, RootsView } from './machines';
import { joinPath, lastSegment, pathDepth, pathSegments, pathWithin } from './paths';

/**
 * A project's settings in one sheet (docs/UI-REDESIGN.md §9, stage 10; F10
 * of §2.5): its account, its folder on each computer, code sync, how it
 * moves and the copies of its sessions, and its teams' limits — each edited
 * by what already edits it on its own page. Opened from *Sessions ›
 * Projects*, a session's ⋮ menu and *Jump to…*; the shell hosts the sheet.
 */

/** A project as its settings name it: its folder on this computer, and
 * the name it goes by. */
export interface ProjectRef {
  cwd: string;
  name: string;
}

/** A project of a folder: its last segment names it. */
export function projectRef(cwd: string, name?: string | null): ProjectRef {
  return { cwd, name: name || lastSegment(cwd) || cwd };
}

/** A project's folder on one computer (null: the shared folder that holds
 * it here has none there yet). */
export interface ProjectFolder {
  machine: RootMachine;
  path: string | null;
}

/** Where a project is on each computer: through the deepest shared folder
 * (root) that holds it here — this computer first, then the paired ones. */
export interface ProjectPlaces {
  root: RootEntry;
  folders: ProjectFolder[];
}

/** A project's place on every computer, or null when no shared folder
 * holds it here (its sessions then can't move to another computer). */
export function projectPlaces(view: RootsView | null, cwd: string): ProjectPlaces | null {
  const here = view?.machines.find((m) => m.here);
  if (!view || !here) return null;
  let best: { root: RootEntry; depth: number } | null = null;
  for (const root of view.roots) {
    const at = root.paths[here.id];
    if (!at || !pathWithin(cwd, at)) continue;
    const depth = pathDepth(at);
    if (!best || depth > best.depth) best = { root, depth };
  }
  if (!best) return null;
  const { root, depth } = best;
  const rest = pathSegments(cwd).slice(depth);
  const others = view.machines.filter((m) => !m.here && m.paired);
  return {
    root,
    folders: [
      { machine: here, path: cwd },
      ...others.map((machine) => {
        const base = root.paths[machine.id];
        return { machine, path: base ? rest.reduce(joinPath, base) : null };
      }),
    ],
  };
}

@Injectable({ providedIn: 'root' })
export class ProjectSettingsService {
  /** The project whose settings show (null: the sheet is closed). */
  readonly project = signal<ProjectRef | null>(null);
  /** The sheet was asked for once: the shell loads it (its own chunk). */
  readonly wanted = signal(false);

  open(project: ProjectRef): void {
    this.wanted.set(true);
    this.project.set({ ...project });
  }

  close(): void {
    this.project.set(null);
  }
}
