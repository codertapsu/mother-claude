import { ComponentFixture, TestBed } from '@angular/core/testing';

import { FolderChoiceService, FolderQuestion } from '../core/folder-choice.service';
import { FakeBackend, asDesktop, flush, useFakeServer } from '../core/mock/fake-server.testing';
import { setActiveMock } from '../core/mock/mock-api';
import { FolderChoiceSheetComponent } from './folder-choice-sheet.component';

const MAC = '4e6a8c0e2a4c6e8a0c2e4a6c8e0a2c4e';
const RECORDED = '/Users/marcus/development/projects/multilingual-dubbed-video';
const HERE = 'D:\\development\\projects\\multilingual-dubbed-video';

const question = (over: Partial<FolderQuestion> = {}): FolderQuestion => ({
  message: `This conversation was started on another computer, in ${RECORDED}. Choose the folder it works in on this PC.`,
  recorded: RECORDED,
  otherMachine: true,
  suggestions: [HERE, 'C:\\Users\\marcus\\dev\\multilingual-dubbed-video'],
  machines: [{ id: MAC, name: 'marcus-mbp', os: 'macos' }],
  ...over,
});

describe('FolderChoiceSheetComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<FolderChoiceSheetComponent>;
  let choices: FolderChoiceService;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend);
    asDesktop({ hostOs: 'windows' });
    backend.routes.set('/backup/pairs', () => ({ status: 201, body: {} }));
    choices = TestBed.inject(FolderChoiceService);
    fixture = TestBed.createComponent(FolderChoiceSheetComponent);
    fixture.detectChanges();
  });

  afterEach(() => setActiveMock(null));

  const el = () => fixture.nativeElement as HTMLElement;
  const text = (e: Element | null | undefined) =>
    (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const ask = (q: FolderQuestion) => {
    const answer = choices.ask(q);
    fixture.detectChanges();
    return answer;
  };
  const options = () => [...el().querySelectorAll<HTMLButtonElement>('[role="radio"]')];
  const button = (label: string) =>
    [...el().querySelectorAll<HTMLButtonElement>('.actions .btn')].find((b) => text(b) === label);
  const remember = () => el().querySelector<HTMLInputElement>('.remember input');
  const typeFolder = (value: string) => {
    const input = el().querySelector<HTMLInputElement>('input[name="folder"]')!;
    input.value = value;
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();
  };

  it('offers the folders here that look like it, the likeliest picked', () => {
    void ask(question());
    expect(text(el().querySelector('.title'))).toBe('Choose its folder on this PC');
    expect(text(el().querySelector('.lede'))).toContain(`in ${RECORDED}.`);
    expect(options().map(text)).toEqual([
      HERE,
      'C:\\Users\\marcus\\dev\\multilingual-dubbed-video',
      'Another folder',
    ]);
    expect(options()[0].getAttribute('aria-checked')).toBe('true');
    // Same name on both: the folders above them are paired.
    expect(text(el().querySelector('.remember'))).toBe(
      'Pair D:\\development\\projects on this PC with /Users/marcus/development/projects on marcus-mbp (macOS) — its other conversations there continue here without asking.',
    );
    expect(remember()!.checked).toBe(true);
  });

  it('answers the folder and pairs the folders above both projects', async () => {
    const answer = ask(question());
    button('Continue here')!.click();
    expect(await answer).toBe(HERE);
    await flush();
    expect(backend.bodyOf('POST /backup/pairs')).toEqual({
      folder: 'D:\\development\\projects',
      machine: MAC,
      otherFolder: '/Users/marcus/development/projects',
    });
    fixture.detectChanges();
    expect(el().querySelector('[role="radio"]')).toBeNull();
  });

  it('pairs nothing when asked not to', async () => {
    const answer = ask(question());
    remember()!.click();
    fixture.detectChanges();
    button('Continue here')!.click();
    expect(await answer).toBe(HERE);
    await flush();
    expect(backend.calls).not.toContain('POST /backup/pairs');
  });

  it('takes a folder typed in full, and pairs the two projects when their names differ', async () => {
    const answer = ask(question());
    options()[2].click();
    fixture.detectChanges();
    typeFolder('dubbing');
    button('Continue here')!.click();
    fixture.detectChanges();
    expect(text(el().querySelector('[role="alert"]'))).toContain('full path');
    typeFolder('E:\\work\\dubbing');
    expect(text(el().querySelector('.remember'))).toContain(
      `Pair E:\\work\\dubbing on this PC with ${RECORDED} on marcus-mbp`,
    );
    button('Continue here')!.click();
    expect(await answer).toBe('E:\\work\\dubbing');
    await flush();
    expect(backend.bodyOf('POST /backup/pairs')).toEqual({
      folder: 'E:\\work\\dubbing',
      machine: MAC,
      otherFolder: RECORDED,
    });
  });

  it('a folder gone from here offers no pair, and Cancel answers nothing', async () => {
    const answer = ask(
      question({
        message:
          'D:\\old\\app is not on this PC any more. Choose the folder this conversation works in here.',
        recorded: 'D:\\old\\app',
        otherMachine: false,
        suggestions: [],
        machines: [],
      }),
    );
    expect(options().length).toBe(0);
    expect(el().querySelector('input[name="folder"]')).not.toBeNull();
    expect(remember()).toBeNull();
    button('Cancel')!.click();
    expect(await answer).toBeNull();
    expect(backend.calls).not.toContain('POST /backup/pairs');
  });
});
