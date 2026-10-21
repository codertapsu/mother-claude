import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import {
  FakeBackend,
  flush,
  serveAccounts,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { MOCK_ACCOUNT_WORK, buildAccounts } from '../../core/mock/fixtures';
import { setActiveMock } from '../../core/mock/mock-api';
import { StorageService } from '../../core/storage.service';
import { StorageSectionComponent } from './storage-section.component';

const DRIVE = 'stor_5d2c8e1f0a7b';

const view = (extra: Record<string, unknown>[] = []) => ({
  settings: { folder: '/Users/me/iCloud/Mother Claude', everyHours: 6, pullOnLaunch: false },
  passphraseSet: true,
  sets: [],
  suggestions: [],
  jobs: [],
  storage: [
    {
      id: 'backup',
      label: 'iCloud Drive',
      path: '/Users/me/iCloud/Mother Claude',
      kind: 'icloud',
      primary: true,
      available: true,
      sets: [
        { name: 'Default', account: 'default', exists: true, sessions: 143, archived: 2 },
        { name: 'Work', account: MOCK_ACCOUNT_WORK, exists: true, keyMissing: true },
      ],
    },
    {
      id: DRIVE,
      label: 'Google Drive',
      path: '/Users/me/Drive/Mother Claude',
      kind: 'gdrive',
      primary: false,
      available: true,
      sets: [{ name: 'Shared', exists: true, sessions: 3, archived: 1 }],
    },
    ...extra,
  ],
});

describe('StorageSectionComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<StorageSectionComponent>;
  let current: ReturnType<typeof view>;

  beforeEach(() => {
    backend = new FakeBackend();
    // A phone: storages are anyone's to change (docs/ACCOUNTS.md §13.9).
    useFakeServer(backend, [provideRouter([])]);
    serveAccounts(backend, buildAccounts(Date.now(), true));
    current = view();
    backend.routes.set('/backup', () => ({ status: 200, body: current }));
  });

  afterEach(() => setActiveMock(null));

  const render = async () => {
    TestBed.inject(StorageService).accept(current);
    fixture = TestBed.createComponent(StorageSectionComponent);
    fixture.componentRef.setInput('suggestions', [
      { label: 'Google Drive (me@example.com)', path: '/Users/me/Drive/Mother Claude' },
    ]);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };
  const settle = async (times = 3) => {
    for (let i = 0; i < times; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: Element, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));
  const type = async (input: HTMLInputElement, value: string) => {
    input.value = value;
    input.dispatchEvent(new Event('input'));
    await settle();
  };

  it('lists every storage with its sets, the backup folder first', async () => {
    const el = await render();
    const stores = [...el.querySelectorAll('.store')];
    expect(stores.map((s) => text(s.querySelector('.stname')))).toEqual([
      'iCloud Drive',
      'Google Drive',
    ]);
    expect(text(stores[0].querySelector('.badge'))).toBe('Backup folder');
    const sets = [...stores[0].querySelectorAll('.stset')].map((r) => text(r));
    expect(sets[0]).toContain("143 conversations · 2 in storage only · Default's backup");
    // A set without its key here offers nothing.
    expect(sets[1]).toContain('its key is not in the folder here');
    expect(stores[0].querySelectorAll('.stset')[1].querySelector('button')).toBeNull();
    expect(text(stores[1].querySelector('.stset'))).toContain(
      '3 conversations · 1 in storage only · only Move data uses it',
    );
    // The backup folder is set above: no rename or remove here.
    expect(button(stores[0], 'Rename')).toBeUndefined();
    expect(button(stores[1], 'Rename')).toBeDefined();
  });

  it('adds a storage from a typed path, with a label', async () => {
    backend.routes.set('/backup/storage', () => {
      current = view([
        {
          id: 'stor_aaaaaaaaaaaa',
          label: 'NAS',
          path: '/Volumes/NAS/MC',
          kind: 'folder',
          available: true,
          sets: [],
        },
      ]);
      return { status: 201, body: current.storage[2] };
    });
    const el = await render();
    button(el, 'Add storage')?.click();
    await settle();
    const sheet = el.querySelector('mc-storage-sheet') as HTMLElement;
    expect(text(sheet)).toContain('Google Drive');
    const path = sheet.querySelector<HTMLInputElement>('input[name="path"]')!;
    await type(path, 'Volumes/NAS');
    expect(text(sheet)).toContain("Enter the folder's full path");
    await type(path, '/Volumes/NAS/MC');
    await type(sheet.querySelector<HTMLInputElement>('input[name="label"]')!, 'NAS');
    button(sheet, 'Add storage')?.click();
    await settle();
    expect(backend.bodyOf('POST /backup/storage')).toEqual({
      path: '/Volumes/NAS/MC',
      label: 'NAS',
    });
    expect(text(el.querySelector('.notice.ok'))).toContain('NAS is a storage now');
    expect([...el.querySelectorAll('.stname')].map((s) => text(s))).toContain('NAS');
    expect(text([...el.querySelectorAll('.store')][2])).toContain('No sets in it yet.');
  });

  it('renames a storage, and removes one with a second tap, leaving its files', async () => {
    backend.routes.set(`/backup/storage/${DRIVE}`, (method) => {
      if (method === 'DELETE') {
        current = { ...view(), storage: view().storage.slice(0, 1) };
        // (The fake backend can't send a 204: an empty answer is the same.)
        return { status: 200, body: {} };
      }
      return { status: 200, body: { ...current.storage[1], label: 'Drive' } };
    });
    const el = await render();
    const drive = () => [...el.querySelectorAll('.store')][1];
    button(drive(), 'Rename')?.click();
    await settle();
    const sheet = el.querySelector('mc-storage-sheet') as HTMLElement;
    const label = sheet.querySelector<HTMLInputElement>('input[name="label"]')!;
    expect(label.value).toBe('Google Drive');
    await type(label, 'Drive');
    button(sheet, 'Save')?.click();
    await settle();
    expect(backend.calls).toContain(`PATCH /backup/storage/${DRIVE}`);
    expect(backend.bodyOf(`PATCH /backup/storage/${DRIVE}`)).toEqual({ label: 'Drive' });

    button(drive(), 'Remove')?.click();
    fixture.detectChanges();
    expect(backend.calls).not.toContain(`DELETE /backup/storage/${DRIVE}`);
    expect(text(el.querySelector('.forget'))).toContain('stay where they are');
    button(drive(), 'Tap again to remove')?.click();
    await settle();
    expect(backend.calls).toContain(`DELETE /backup/storage/${DRIVE}`);
    expect(text(el.querySelector('.notice.ok'))).toContain('its files are where they were');
    expect(el.querySelectorAll('.store').length).toBe(1);
  });

  it('brings a set back into an account, and moves into a set', async () => {
    const el = await render();
    const drive = [...el.querySelectorAll('.store')][1];
    button(drive, 'Bring back')?.click();
    await settle();
    const sheet = el.querySelector('mc-move-data-sheet') as HTMLElement;
    const value = (name: string) =>
      sheet.querySelector<HTMLSelectElement>(`select[name="${name}"]`)?.value;
    expect(value('from')).toBe(`s:${DRIVE}:Shared`);
    expect(value('to')).toBe('a:default');
    expect(text(sheet.querySelector('.allline'))).toBe(
      'Everything in Google Drive · Shared (3 conversations).',
    );
    sheet.querySelector<HTMLButtonElement>('button[aria-label="Close"]')?.click();
    await settle();

    button([...el.querySelectorAll('.store')][0].querySelector('.stset')!, 'Move here')?.click();
    await settle();
    expect(value('from')).toBe('a:default');
    expect(value('to')).toBe('s:backup:Default');
    sheet.querySelector<HTMLButtonElement>('button[aria-label="Close"]')?.click();
    await settle();

    button(drive, 'New set')?.click();
    await settle();
    expect(value('to')).toBe(`n:${DRIVE}`);
    expect(sheet.querySelector('input[name="newSet"]')).not.toBeNull();
  });
});
