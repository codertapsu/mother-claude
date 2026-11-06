import {
  folderExample,
  folderPathProblem,
  fullPathText,
  homeFolder,
  hostOsFromUserAgent,
  hostOsOf,
  isAbsoluteAnywhere,
  isHomePath,
  isWindowsAbsolute,
  joinPath,
  lastSegment,
  pathDepth,
  pathWithin,
  samePath,
  tildePath,
  trimSeparators,
} from './paths';

describe('paths', () => {
  it('takes a full path of either system, whatever this browser runs on', () => {
    for (const p of [
      '/',
      '/Users/marcus/dev/app',
      '/Volumes/NAS/Mother Claude',
      'D:\\NAS\\MotherClaude',
      'd:/dev/app',
      'C:\\',
      '\\\\nas\\share\\Mother Claude',
      '\\\\?\\C:\\Users\\m',
    ]) {
      expect(isAbsoluteAnywhere(p)).withContext(p).toBeTrue();
    }
    for (const p of ['', '~', '~/dev', 'dev\\app', 'Drive/MC', 'C:', 'C:dev', '\\\\', '1:/x']) {
      expect(isAbsoluteAnywhere(p)).withContext(p).toBeFalse();
    }
    expect(isWindowsAbsolute('D:\\dev')).toBeTrue();
    expect(isWindowsAbsolute('/Users/m')).toBeFalse();
    expect(['~', '~/x', '~\\x'].every(isHomePath)).toBeTrue();
    expect(['~x', '~user/x', 'x/~'].some(isHomePath)).toBeFalse();
  });

  it("says what is wrong with a typed folder in the desktop's own words", () => {
    expect(folderPathProblem('', 'macos')).toBe("Enter the folder's path.");
    expect(folderPathProblem('D:\\NAS\\MotherClaude', 'windows')).toBe('');
    expect(folderPathProblem('D:/NAS/MotherClaude', 'windows')).toBe('');
    expect(folderPathProblem('\\\\nas\\share\\MC', 'windows')).toBe('');
    expect(folderPathProblem('~\\Backups', 'windows')).toBe('');
    expect(folderPathProblem('/Volumes/NAS/MC', 'macos')).toBe('');
    // The other system's full path passes too: the desktop decides.
    expect(folderPathProblem('D:\\NAS', 'macos')).toBe('');
    expect(folderPathProblem('/Volumes/NAS', 'windows')).toBe('');
    expect(folderPathProblem('Drive/MC', 'macos')).toBe(
      "Enter the folder's full path, starting with / (or ~ for your home folder).",
    );
    expect(folderPathProblem('NAS\\MC', 'windows')).toBe(
      "Enter the folder's full path, starting with a drive like C:\\ or a share like \\\\server\\share (or ~ for your home folder).",
    );
    expect(folderPathProblem('/a/../b', 'macos')).toBe('Enter the path without . or .. in it.');
    expect(folderPathProblem('D:\\a\\..\\b', 'windows')).toContain('without . or ..');
    // Where ~ may not stand in, it is not offered.
    expect(folderPathProblem('~/dev', 'macos', { home: false })).toBe(
      "Enter the folder's full path, starting with /.",
    );
    expect(fullPathText('windows', { noun: 'directory', home: false })).toBe(
      "Enter the directory's full path, starting with a drive like C:\\ or a share like \\\\server\\share.",
    );
  });

  it('shows examples in the form of the desktop', () => {
    expect(folderExample('windows', 'backup')).toBe('D:\\Backups\\Mother Claude');
    expect(folderExample('macos', 'storage')).toBe('/Volumes/NAS/Mother Claude');
    expect(folderExample('macos', 'project')).toBe('/Users/you/project');
    expect(folderExample('windows', 'project')).toBe('C:\\Users\\you\\project');
    expect(folderExample('macos', 'projects')).toBe('/Users/you/dev');
    expect(folderExample('windows', 'projects')).toBe('D:\\dev');
    expect(folderExample('linux', 'projects')).toBe('/home/you/dev');
    expect(homeFolder('windows', '.claude-work')).toBe('~\\.claude-work');
    expect(homeFolder('linux', '.claude-work')).toBe('~/.claude-work');
  });

  it('reads the system from the wire, or from the desktop webview while the server does not say', () => {
    expect(hostOsOf('windows')).toBe('windows');
    expect(hostOsOf('beos')).toBeUndefined();
    expect(hostOsFromUserAgent('Mozilla/5.0 (Windows NT 10.0; Win64; x64)')).toBe('windows');
    expect(hostOsFromUserAgent('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)')).toBe('macos');
    expect(hostOsFromUserAgent('Mozilla/5.0 (X11; Linux x86_64)')).toBe('linux');
    expect(hostOsFromUserAgent('Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)')).toBe(
      undefined,
    );
  });

  it('takes paths apart by their own separators', () => {
    expect(trimSeparators('/a/b/')).toBe('/a/b');
    expect(trimSeparators('/')).toBe('/');
    expect(trimSeparators('D:\\NAS\\')).toBe('D:\\NAS');
    expect(trimSeparators('D:\\')).toBe('D:\\');
    expect(trimSeparators('D:/')).toBe('D:/');
    // A backslash is part of a name on a Mac.
    expect(trimSeparators('/a/b\\')).toBe('/a/b\\');
    expect(lastSegment('/Users/me/app/')).toBe('app');
    expect(lastSegment('D:\\NAS\\MotherClaude')).toBe('MotherClaude');
    expect(lastSegment('D:/NAS/MotherClaude')).toBe('MotherClaude');
    expect(lastSegment('\\\\nas\\share\\Mother Claude')).toBe('Mother Claude');
    expect(lastSegment('C:\\')).toBe('');
    expect(lastSegment('/')).toBe('');
    expect(joinPath('D:\\NAS\\MotherClaude', 'Work')).toBe('D:\\NAS\\MotherClaude\\Work');
    expect(joinPath('D:/NAS/', 'Work')).toBe('D:/NAS/Work');
    expect(joinPath('/Volumes/NAS/MC/', 'Work')).toBe('/Volumes/NAS/MC/Work');
    expect(joinPath('C:\\', 'Work')).toBe('C:\\Work');
    expect(pathDepth('/a/b')).toBe(2);
    expect(pathDepth('C:\\a\\b\\')).toBe(2);
    expect(pathDepth('/')).toBe(0);
  });

  it('compares a Windows path the way Windows does, and a Mac path exactly', () => {
    expect(samePath('C:\\Users\\me\\app', 'c:/users/ME/app/')).toBeTrue();
    expect(samePath('/Users/me/app', '/Users/me/app/')).toBeTrue();
    expect(samePath('/Users/me/app', '/users/me/app')).toBeFalse();
    expect(samePath('/Users/me/app', 'C:\\Users\\me\\app')).toBeFalse();
    expect(pathWithin('C:\\Dev\\app\\src', 'c:/dev/app')).toBeTrue();
    expect(pathWithin('C:\\dev\\app2', 'C:\\dev\\app')).toBeFalse();
    expect(pathWithin('C:\\dev', 'C:\\')).toBeTrue();
    expect(pathWithin('/p/acme/api', '/p/acme')).toBeTrue();
    expect(pathWithin('/p/acme2', '/p/acme')).toBeFalse();
    expect(pathWithin('/anything', '/')).toBeTrue();
    expect(pathWithin('/p/acme', 'C:\\p\\acme')).toBeFalse();
  });

  it("shortens a home folder's path on either system", () => {
    expect(tildePath('/Users/me/dev/app')).toBe('~/dev/app');
    expect(tildePath('/home/me/dev')).toBe('~/dev');
    expect(tildePath('C:\\Users\\me\\dev\\app')).toBe('~\\dev\\app');
    expect(tildePath('c:/users/me/dev')).toBe('~/dev');
    expect(tildePath('D:\\NAS\\MotherClaude')).toBe('D:\\NAS\\MotherClaude');
    expect(tildePath(undefined)).toBe('');
  });
});
