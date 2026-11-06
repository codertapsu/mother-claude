import {
  HostOs,
  hostOsOf,
  isWindowsAbsolute,
  joinPath,
  lastSegment,
  trimSeparators,
} from '../paths';
import { storageGet, urlParam } from '../platform';

/** `?os=windows|linux` (or localStorage `mc_mock_os`) makes the mock's
 * desktop that system — its hints, and the folders it takes; a Mac by
 * default. */
export function mockHostOs(): HostOs {
  return hostOsOf(urlParam('os') ?? storageGet('mc_mock_os')) ?? 'macos';
}

const HOMES: Record<HostOs, string> = {
  macos: '/Users/marcus',
  windows: 'C:\\Users\\marcus',
  linux: '/home/marcus',
};

/** How a full path starts on `os`, as the desktop says it. */
function startText(os: HostOs): string {
  return os === 'windows' ? 'a drive like C:\\ or a share like \\\\server\\share' : '/';
}

/** A folder typed for the mock's desktop, checked by that system's rules as
 * the real one checks them: `~` (`~/`, and `~\` on Windows) expanded, a
 * full path of that system, no `.` or `..`, no trailing separator. */
export function mockFolder(
  raw: string,
  os: HostOs = mockHostOs(),
): { path: string } | { error: string } {
  const t = raw.trim();
  const p = t.replace(os === 'windows' ? /^~(?=[\\/]|$)/ : /^~(?=\/|$)/, () => HOMES[os]);
  const absolute = os === 'windows' ? isWindowsAbsolute(p) : p.startsWith('/');
  if (!absolute) {
    return {
      error: `${t} is not a full path on this computer: start it with ${startText(os)} (or ~ for your home folder)`,
    };
  }
  if (p.split(os === 'windows' ? /[\\/]/ : /\//).some((s) => s === '.' || s === '..')) {
    return { error: 'the directory cannot contain . or ..' };
  }
  return { path: trimSeparators(os === 'windows' ? p.replace(/\//g, '\\') : p) };
}

/** Folders of the mock's desktop that may be another computer's `recorded`
 * project: the same name in its `dev` folder (and on `D:` for Windows). */
export function mockFoldersLike(recorded: string, os: HostOs = mockHostOs()): string[] {
  const name = lastSegment(recorded);
  if (!name) return [];
  const homeDev = joinPath(joinPath(HOMES[os], 'dev'), name);
  return os === 'windows' ? [joinPath('D:\\dev', name), homeDev] : [homeDev];
}
