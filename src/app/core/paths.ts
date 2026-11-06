/**
 * Paths the way the desktop writes them. This app runs in the desktop's
 * webview and on paired phones — an iPhone may drive a Windows PC — so
 * nothing here follows the browser's own system. A folder field takes a full
 * path of either system: `/…` (macOS, Linux), a Windows drive (`D:\…`,
 * `D:/…`) or share (`\\server\share\…`), and where the home folder may stand
 * in, `~`, `~/…` or `~\…`. The desktop, which knows its own system, checks
 * the rest and has the last word (its refusal is shown as it is).
 * Placeholders, examples and hints follow the desktop's system
 * (`me.hostOs`).
 *
 * Folders the desktop recorded (a conversation's folder, a project of a
 * backup made on another machine) may be either system's too: they compare
 * by their own form — a Windows path without regard to case or to which
 * separator, as Windows does.
 */

/** The desktop's system, as `GET /me` names it (`hostOs`). */
export type HostOs = 'macos' | 'windows' | 'linux';

const HOST_OSES: readonly HostOs[] = ['macos', 'windows', 'linux'];

/** A system from the wire; undefined for anything else. */
export function hostOsOf(raw: unknown): HostOs | undefined {
  return HOST_OSES.find((o) => o === raw);
}

/** The system a user agent runs on: the desktop's own webview's, for as long
 * as the server does not say. */
export function hostOsFromUserAgent(userAgent: string): HostOs | undefined {
  if (/Windows/.test(userAgent)) return 'windows';
  if (/Android|iPhone|iPad/.test(userAgent)) return undefined;
  if (/Macintosh|Mac OS X/.test(userAgent)) return 'macos';
  if (/Linux/.test(userAgent)) return 'linux';
  return undefined;
}

/** A Windows full path: a drive (`C:\…`, `C:/…`) or a share
 * (`\\server\share\…`). `C:` alone and `C:dev` are relative to a drive's
 * current folder, so not. */
export function isWindowsAbsolute(p: string): boolean {
  return /^[A-Za-z]:[\\/]/.test(p) || /^\\\\[^\\]/.test(p);
}

/** A full path on macOS or Linux (`/…`) or on Windows. */
export function isAbsoluteAnywhere(p: string): boolean {
  return p.startsWith('/') || isWindowsAbsolute(p);
}

/** The home folder, or a path in it: `~`, `~/…`, `~\…`. */
export function isHomePath(p: string): boolean {
  return p === '~' || p.startsWith('~/') || p.startsWith('~\\');
}

/** Written the Windows way: a drive, a share, or `~\…`. */
function windowsStyle(p: string): boolean {
  return isWindowsAbsolute(p) || p.startsWith('~\\');
}

/** The separators of a path: `\` as well as `/` in a Windows one. */
function separators(p: string): RegExp {
  return windowsStyle(p) ? /[\\/]/ : /\//;
}

/** Without trailing separators; a root keeps its own (`/`, `C:\`). */
export function trimSeparators(p: string): string {
  const t = p.replace(windowsStyle(p) ? /[\\/]+$/ : /\/+$/, '');
  if (/^[A-Za-z]:$/.test(t) && isWindowsAbsolute(p)) return p.slice(0, 3);
  if (!t && p.startsWith('/')) return '/';
  return t;
}

/** The last component: `app` of `/Users/me/app` and of `D:\dev\app`; ''
 * for a root. */
export function lastSegment(p: string): string {
  const parts = trimSeparators(p).split(separators(p));
  const last = parts[parts.length - 1] ?? '';
  return /^[A-Za-z]:$/.test(last) ? '' : last;
}

/** `name` inside `folder`, with the separator the folder uses. */
export function joinPath(folder: string, name: string): string {
  const base = trimSeparators(folder);
  const sep = windowsStyle(folder) && folder.includes('\\') ? '\\' : '/';
  return /[\\/]$/.test(base) ? `${base}${name}` : `${base}${sep}${name}`;
}

/** What a path compares by: a Windows one lower-cased with `\` only. */
function pathKey(p: string): string {
  const t = trimSeparators(p.trim());
  return isWindowsAbsolute(t) ? t.replace(/\//g, '\\').toLowerCase() : t;
}

/** Two paths name one folder (a trailing separator never counts). */
export function samePath(a: string, b: string): boolean {
  return pathKey(a) === pathKey(b);
}

/** `path` is `dir` or inside it (whole components). */
export function pathWithin(path: string, dir: string): boolean {
  const p = pathKey(path);
  const d = pathKey(dir);
  if (p === d) return true;
  const sep = isWindowsAbsolute(d) ? '\\' : '/';
  return p.startsWith(d.endsWith(sep) ? d : `${d}${sep}`);
}

/** How many components a path has (`/a/b` and `C:\a\b`: 2). */
export function pathDepth(p: string): number {
  return trimSeparators(p)
    .split(separators(p))
    .filter((s) => s && !/^[A-Za-z]:$/.test(s)).length;
}

/** A home folder's path shortened: `/Users/me/dev/app` → `~/dev/app`,
 * `C:\Users\me\dev\app` → `~\dev\app`. */
export function tildePath(path: string | undefined): string {
  return (path ?? '')
    .replace(/^\/(Users|home)\/[^/]+/, '~')
    .replace(/^[A-Za-z]:([\\/])Users\1[^\\/]+/i, '~');
}

/** Options of a folder field: whether `~` may stand for the home folder
 * (default yes), and what the field calls its path. */
export interface FolderField {
  home?: boolean;
  noun?: 'folder' | 'directory';
}

/** "Enter the folder's full path, starting with / (or ~ for your home
 * folder)." — in the desktop's form. */
export function fullPathText(os: HostOs, field: FolderField = {}): string {
  const noun = field.noun ?? 'folder';
  const start = os === 'windows' ? 'a drive like C:\\ or a share like \\\\server\\share' : '/';
  const home = (field.home ?? true) ? ' (or ~ for your home folder)' : '';
  return `Enter the ${noun}'s full path, starting with ${start}${home}.`;
}

/** Why a typed folder can't be sent ('' when it may be): a full path of
 * either system (or the home folder's, where `~` may stand in) without `.`
 * or `..` in it. The desktop checks the rest: that it is a full path of its
 * own system, and what the field needs besides. */
export function folderPathProblem(path: string, os: HostOs, field: FolderField = {}): string {
  const p = path.trim();
  if (!p) return `Enter the ${field.noun ?? 'folder'}'s path.`;
  if (!isAbsoluteAnywhere(p) && !((field.home ?? true) && isHomePath(p))) {
    return fullPathText(os, field);
  }
  if (p.split(/[\\/]/).some((s) => s === '.' || s === '..')) {
    return 'Enter the path without . or .. in it.';
  }
  return '';
}

/** What a placeholder shows (`projects`: a folder that holds projects, as
 * a folder pair names one). */
export type FolderExample = 'backup' | 'storage' | 'project' | 'projects';

const EXAMPLES: Record<HostOs, Record<FolderExample, string>> = {
  macos: {
    backup: '/Users/you/Library/Mobile Documents/com~apple~CloudDocs/Mother Claude',
    storage: '/Volumes/NAS/Mother Claude',
    project: '/Users/you/project',
    projects: '/Users/you/dev',
  },
  windows: {
    backup: 'D:\\Backups\\Mother Claude',
    storage: '\\\\nas\\share\\Mother Claude',
    project: 'C:\\Users\\you\\project',
    projects: 'D:\\dev',
  },
  linux: {
    backup: '/home/you/Mother Claude',
    storage: '/mnt/nas/Mother Claude',
    project: '/home/you/project',
    projects: '/home/you/dev',
  },
};

/** An example folder in the form of `os`: the desktop's, or another
 * machine's of the backups (a folder pair's other side). */
export function folderExample(os: HostOs, kind: FolderExample): string {
  return EXAMPLES[os][kind];
}

/** A folder in the home folder, in the desktop's form: `~/x` or `~\x`. */
export function homeFolder(os: HostOs, name: string): string {
  return os === 'windows' ? `~\\${name}` : `~/${name}`;
}
