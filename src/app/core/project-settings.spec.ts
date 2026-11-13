import { TestBed } from '@angular/core/testing';

import { RootsView } from './machines';
import { ProjectSettingsService, projectPlaces, projectRef } from './project-settings';

const HERE = '0'.repeat(32);
const PC = 'f'.repeat(32);
const BOX = 'e'.repeat(32);
const GONE = 'd'.repeat(32);

const roots = (over: Partial<RootsView> = {}): RootsView => ({
  machine: HERE,
  machines: [
    { id: HERE, name: 'MacBook', os: 'macos', here: true, paired: false },
    { id: PC, name: 'DESKTOP-PC', os: 'windows', here: false, paired: true },
    { id: BOX, name: 'BUILD-BOX', os: 'linux', here: false, paired: true },
    { id: GONE, name: 'OLD-PC', os: 'windows', here: false, paired: false },
  ],
  roots: [
    {
      id: 'rt_dev',
      name: 'dev',
      paths: { [HERE]: '/Users/me/dev', [PC]: 'D:\\dev', [GONE]: 'C:\\old' },
      updatedAt: 1,
      by: HERE,
    },
    {
      id: 'rt_work',
      name: 'work',
      paths: { [HERE]: '/Users/me/dev/work', [PC]: 'E:\\work', [BOX]: '/srv/work' },
      updatedAt: 1,
      by: HERE,
    },
  ],
  max: 16,
  ...over,
});

describe("a project's settings (docs/UI-REDESIGN.md §9, stage 10)", () => {
  it('names a project by its folder unless told', () => {
    expect(projectRef('/Users/me/dev/api')).toEqual({ cwd: '/Users/me/dev/api', name: 'api' });
    expect(projectRef('D:\\dev\\tray', 'tray-tool')).toEqual({
      cwd: 'D:\\dev\\tray',
      name: 'tray-tool',
    });
  });

  it('finds a project on each paired computer through the shared folder holding it', () => {
    const p = projectPlaces(roots(), '/Users/me/dev/api/server')!;
    expect(p.root.name).toBe('dev');
    expect(p.folders.map((f) => [f.machine.name, f.path])).toEqual([
      ['MacBook', '/Users/me/dev/api/server'],
      // Written the other computer's way; one the shared folder skips has none.
      ['DESKTOP-PC', 'D:\\dev\\api\\server'],
      ['BUILD-BOX', null],
    ]);
  });

  it('takes the deepest shared folder that holds it', () => {
    const p = projectPlaces(roots(), '/Users/me/dev/work/site')!;
    expect(p.root.name).toBe('work');
    expect(p.folders.map((f) => f.path)).toEqual([
      '/Users/me/dev/work/site',
      'E:\\work\\site',
      '/srv/work/site',
    ]);
  });

  it('says when no shared folder holds it, or nothing is known yet', () => {
    expect(projectPlaces(roots(), '/Users/me/elsewhere/x')).toBeNull();
    // A folder that only starts like one is not in it.
    expect(projectPlaces(roots(), '/Users/me/devtools')).toBeNull();
    expect(projectPlaces(null, '/Users/me/dev/api')).toBeNull();
    expect(projectPlaces(roots({ machines: [] }), '/Users/me/dev/api')).toBeNull();
  });

  it('maps a Windows project, in any case', () => {
    const view = roots({
      machine: PC,
      machines: [
        { id: PC, name: 'DESKTOP-PC', os: 'windows', here: true, paired: false },
        { id: HERE, name: 'MacBook', os: 'macos', here: false, paired: true },
      ],
    });
    const p = projectPlaces(view, 'd:\\Dev\\api')!;
    expect(p.folders.map((f) => f.path)).toEqual(['d:\\Dev\\api', '/Users/me/dev/api']);
  });

  it('opens and closes, and asks the shell for the sheet once', () => {
    const svc = TestBed.inject(ProjectSettingsService);
    expect(svc.wanted()).toBe(false);
    svc.open({ cwd: '/a', name: 'a' });
    expect(svc.project()).toEqual({ cwd: '/a', name: 'a' });
    expect(svc.wanted()).toBe(true);
    svc.close();
    expect(svc.project()).toBeNull();
    expect(svc.wanted()).toBe(true);
  });
});
