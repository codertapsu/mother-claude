import { mockFolder, mockHostOs } from './mock-os';

describe("the mock desktop's system", () => {
  afterEach(() => localStorage.removeItem('mc_mock_os'));

  it('is a Mac unless asked to be another', () => {
    expect(mockHostOs()).toBe('macos');
    localStorage.setItem('mc_mock_os', 'windows');
    expect(mockHostOs()).toBe('windows');
    localStorage.setItem('mc_mock_os', 'beos');
    expect(mockHostOs()).toBe('macos');
  });

  it('takes a folder by its own rules, as the real desktop does', () => {
    expect(mockFolder('~/Backups', 'macos')).toEqual({ path: '/Users/marcus/Backups' });
    expect(mockFolder('/Volumes/NAS/MC/', 'macos')).toEqual({ path: '/Volumes/NAS/MC' });
    expect(mockFolder('D:\\NAS\\MotherClaude', 'macos')).toEqual({
      error:
        'D:\\NAS\\MotherClaude is not a full path on this computer: start it with / (or ~ for your home folder)',
    });
    // `~\` is a home folder on Windows only.
    expect('error' in mockFolder('~\\Backups', 'macos')).toBe(true);
    expect(mockFolder('D:\\NAS\\MotherClaude', 'windows')).toEqual({
      path: 'D:\\NAS\\MotherClaude',
    });
    expect(mockFolder('D:/NAS/MotherClaude/', 'windows')).toEqual({
      path: 'D:\\NAS\\MotherClaude',
    });
    expect(mockFolder('~\\Backups', 'windows')).toEqual({ path: 'C:\\Users\\marcus\\Backups' });
    expect(mockFolder('\\\\nas\\share\\MC', 'windows')).toEqual({ path: '\\\\nas\\share\\MC' });
    expect(mockFolder('/Users/me/x', 'windows')).toEqual({
      error:
        '/Users/me/x is not a full path on this computer: start it with a drive like C:\\ or a share like \\\\server\\share (or ~ for your home folder)',
    });
    expect(mockFolder('E:\\a\\..\\b', 'windows')).toEqual({
      error: 'the directory cannot contain . or ..',
    });
  });
});
