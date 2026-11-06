import { ComponentFixture, TestBed } from '@angular/core/testing';

import { FolderPairDraftService } from '../../core/folder-pair-draft.service';
import {
  FakeBackend,
  asDesktop,
  asPhoneOf,
  flush,
  useFakeServer,
} from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { FolderPairsView } from '../../core/models';
import { FolderPairsComponent } from './folder-pairs.component';

const MAC = '9f3c2a7e5b1d4c6f8a0b2c4d6e8f0a1b';
const WIN = '7b9d1f3a5c7e9b1d3f5a7c9e1b3d5f7a';
const AIR = '4e6a8c0e2a4c6e8a0c2e4a6c8e0a2c4e';

const view = (over: Partial<FolderPairsView> = {}): FolderPairsView => ({
  machine: MAC,
  host: 'marcus-mbp',
  os: 'macos',
  machines: [
    { id: WIN, name: 'DESKTOP-PC', os: 'windows' },
    { id: AIR, name: 'marcus-air', os: 'macos' },
  ],
  pairs: [],
  readAt: Date.now(),
  ...over,
});

const paired = (): FolderPairsView =>
  view({
    pairs: [
      {
        id: 'fp_0123456789abcdef',
        folders: [
          {
            machine: MAC,
            name: 'marcus-mbp',
            os: 'macos',
            folder: '/Users/marcus/dev',
            here: true,
          },
          { machine: WIN, name: 'DESKTOP-PC', os: 'windows', folder: 'D:\\dev', here: false },
        ],
        createdAt: 1,
      },
    ],
  });

describe('FolderPairsComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<FolderPairsComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend);
    asDesktop({ hostOs: 'macos' });
  });

  afterEach(() => setActiveMock(null));

  const render = async (v: FolderPairsView | null) => {
    fixture = TestBed.createComponent(FolderPairsComponent);
    fixture.componentRef.setInput('pairs', v);
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
  const button = (el: HTMLElement, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));
  const type = (el: HTMLElement, name: string, value: string) => {
    const input = el.querySelector(`input[name="${name}"]`) as HTMLInputElement;
    input.value = value;
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();
  };
  const pick = (el: HTMLElement, machine: string) => {
    const select = el.querySelector('select[name="machine"]') as HTMLSelectElement;
    select.value = machine;
    select.dispatchEvent(new Event('change'));
    fixture.detectChanges();
  };

  it('explains itself in a sentence and lists the pairs, this Mac first', async () => {
    const el = await render(paired());
    expect(text(el.querySelector('.lede'))).toBe(
      'Pair a folder on this Mac with the folder that holds the same projects on another machine: a session pulled from there goes into the matching project here (and back), so Claude Code lists it where you work.',
    );
    const sides = [...el.querySelectorAll('.prow .side')].map((s) => [
      text(s.querySelector('.who')),
      text(s.querySelector('.path')),
    ]);
    expect(sides).toEqual([
      ['This Mac', '/Users/marcus/dev'],
      ['DESKTOP-PC (Windows)', 'D:\\dev'],
    ]);
    // The other machines, each with how it writes paths.
    const options = [...el.querySelectorAll('select[name="machine"] option')].map((o) => text(o));
    expect(options).toEqual(['DESKTOP-PC (Windows)', 'marcus-air (macOS)']);
    // Each folder's example in its own machine's form.
    const input = (name: string) => el.querySelector(`input[name="${name}"]`) as HTMLInputElement;
    expect(input('here').placeholder).toBe('/Users/you/dev');
    expect(input('there').placeholder).toBe('D:\\dev');
    pick(el, AIR);
    expect(input('there').placeholder).toBe('/Users/you/dev');
  });

  it('checks each folder the way its machine writes paths before it asks', async () => {
    const el = await render(view());
    type(el, 'here', 'dev');
    type(el, 'there', 'D:\\dev');
    button(el, 'Add pair')?.click();
    await settle();
    expect(text(el.querySelector('.notice.error'))).toBe(
      "On this Mac: Enter the folder's full path, starting with / (or ~ for your home folder).",
    );
    type(el, 'here', '/Users/marcus/dev/../x');
    button(el, 'Add pair')?.click();
    await settle();
    expect(text(el.querySelector('.notice.error'))).toBe(
      'On this Mac: Enter the path without . or .. in it.',
    );
    // The other machine's in its own form, and never `~` (that would be
    // this desktop's home folder).
    type(el, 'here', '~/dev');
    for (const there of ['/Users/marcus/dev', '~\\dev', 'dev']) {
      type(el, 'there', there);
      button(el, 'Add pair')?.click();
      await settle();
      expect(text(el.querySelector('.notice.error')))
        .withContext(there)
        .toBe(
          "On DESKTOP-PC: Enter the folder's full path, starting with a drive like C:\\ or a share like \\\\server\\share.",
        );
    }
    expect(backend.calls.some((c) => c.startsWith('POST'))).toBeFalse();
    // `~` is this machine's home folder: the desktop expands it.
    backend.routes.set('/backup/pairs', () => ({ status: 201, body: paired() }));
    type(el, 'there', 'd:/dev');
    button(el, 'Add pair')?.click();
    await settle();
    expect(backend.bodyOf('POST /backup/pairs')).toEqual({
      folder: '~/dev',
      machine: WIN,
      otherFolder: 'd:/dev',
    });
  });

  it("follows a Windows desktop's form here, and a Mac's on the other side", async () => {
    asPhoneOf('windows');
    const el = await render(view({ os: 'windows' }));
    expect(text(el.querySelector('.lede'))).toContain('Pair a folder on this PC with');
    const input = (name: string) => el.querySelector(`input[name="${name}"]`) as HTMLInputElement;
    expect(input('here').placeholder).toBe('D:\\dev');
    pick(el, AIR);
    expect(input('there').placeholder).toBe('/Users/you/dev');
    type(el, 'here', 'D:\\dev');
    type(el, 'there', 'D:\\dev');
    button(el, 'Add pair')?.click();
    await settle();
    expect(text(el.querySelector('.notice.error'))).toBe(
      "On marcus-air: Enter the folder's full path, starting with /.",
    );
    backend.routes.set('/backup/pairs', () => ({ status: 201, body: paired() }));
    type(el, 'there', '/Users/marcus/dev');
    button(el, 'Add pair')?.click();
    await settle();
    expect(backend.bodyOf('POST /backup/pairs')).toEqual({
      folder: 'D:\\dev',
      machine: AIR,
      otherFolder: '/Users/marcus/dev',
    });
  });

  it('adds a pair and shows what the backups say now', async () => {
    backend.routes.set('/backup/pairs', () => ({ status: 201, body: paired() }));
    const el = await render(view());
    type(el, 'here', ' /Users/marcus/dev ');
    type(el, 'there', 'D:\\dev');
    button(el, 'Add pair')?.click();
    await settle();
    expect(backend.bodyOf('POST /backup/pairs')).toEqual({
      folder: '/Users/marcus/dev',
      machine: WIN,
      otherFolder: 'D:\\dev',
    });
    expect(text(el.querySelector('.notice.ok'))).toBe(
      'Paired. Sessions pulled from there now go into the matching project here.',
    );
    expect(el.querySelectorAll('.prow').length).toBe(1);
    expect((el.querySelector('input[name="here"]') as HTMLInputElement).value).toBe('');
  });

  it("says the server's refusal in its words", async () => {
    backend.routes.set('/backup/pairs', () => ({
      status: 422,
      body: { error: { code: 'invalid_pair', message: 'these two folders are paired already' } },
    }));
    const el = await render(paired());
    type(el, 'here', '/Users/marcus/dev');
    type(el, 'there', 'D:\\dev');
    button(el, 'Add pair')?.click();
    await settle();
    expect(text(el.querySelector('.notice.error'))).toBe('These two folders are paired already.');
  });

  it('removes a pair', async () => {
    backend.routes.set('/backup/pairs/fp_0123456789abcdef', () => ({ status: 200, body: view() }));
    const el = await render(paired());
    (
      el.querySelector('[aria-label="Remove the pair of /Users/marcus/dev"]') as HTMLButtonElement
    ).click();
    await settle();
    expect(backend.calls).toContain('DELETE /backup/pairs/fp_0123456789abcdef');
    expect(el.querySelectorAll('.prow').length).toBe(0);
    expect(text(el.querySelector('.notice.ok'))).toBe('The pair is removed from the backups.');
  });

  it('waits for another machine, and looks again when asked', async () => {
    backend.routes.set('/backup/pairs', () => ({ status: 200, body: view() }));
    const el = await render(view({ machines: [] }));
    expect(el.querySelector('form')).toBeNull();
    expect(text(el.querySelector('.none'))).toBe(
      'Another machine shows here once it has backed up to or pulled from these backups.',
    );
    button(el, 'Look again')?.click();
    await settle();
    expect(backend.urls).toContain('/backup/pairs?refresh=true');
    expect(el.querySelector('form')).not.toBeNull();
  });

  it('is filled from a project that kept another machine’s path', async () => {
    const el = await render(view());
    TestBed.inject(FolderPairDraftService).offer(WIN, 'D:\\dev\\shop');
    await settle();
    expect((el.querySelector('select[name="machine"]') as HTMLSelectElement).value).toBe(WIN);
    expect((el.querySelector('input[name="there"]') as HTMLInputElement).value).toBe('D:\\dev');
    expect(TestBed.inject(FolderPairDraftService).draft()).toBeNull();
  });

  it('offers the folder above a project, in its own form', () => {
    const drafts = TestBed.inject(FolderPairDraftService);
    for (const [project, above] of [
      ['/Users/m/dev/app', '/Users/m/dev'],
      ['/Users/m/dev/app/', '/Users/m/dev'],
      ['D:\\dev\\app\\', 'D:\\dev'],
      ['D:/dev/app', 'D:/dev'],
      ['D:\\app', 'D:\\'],
      ['\\\\nas\\share\\app', '\\\\nas\\share'],
      ['/app', '/'],
    ]) {
      drafts.offer(AIR, project);
      expect(drafts.take()?.otherFolder).withContext(project).toBe(above);
    }
  });
});
