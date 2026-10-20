import { ComponentFixture, TestBed } from '@angular/core/testing';

import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { PassphraseSheetComponent } from './passphrase-sheet.component';

const SECRET = 'ABCD-EFGH-JKLM-NPQR-STUV-WXYZ-2345-6789';

describe('PassphraseSheetComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<PassphraseSheetComponent>;
  let closed: jasmine.Spy;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend);
    asDesktop();
    backend.routes.set('/backup/passphrase', (_m, body) =>
      (body as { generate?: boolean }).generate
        ? { status: 200, body: { passphrase: SECRET, rekeyed: ['Default', 'Work'], left: [] } }
        : { status: 200, body: { passphrase: null, rekeyed: ['Default'], left: ['Personal'] } },
    );
  });

  afterEach(() => setActiveMock(null));

  let saved: jasmine.Spy;

  const open = async (unfinished = false) => {
    fixture = TestBed.createComponent(PassphraseSheetComponent);
    closed = jasmine.createSpy('closed');
    saved = jasmine.createSpy('saved');
    fixture.componentInstance.closed.subscribe(closed);
    fixture.componentInstance.saved.subscribe(saved);
    fixture.componentRef.setInput('changing', true);
    fixture.componentRef.setInput('unfinished', unfinished);
    fixture.componentRef.setInput('open', true);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };
  const settle = async () => {
    await flush();
    fixture.detectChanges();
  };
  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: HTMLElement, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));
  const closeButton = (el: HTMLElement) =>
    el.querySelector('button[aria-label="Close"]') as HTMLButtonElement;

  it('shows a generated passphrase once and closes only once it is written down', async () => {
    const el = await open();
    expect(text(el)).toContain('Without the passphrase no backup can be read');
    button(el, 'Generate a passphrase')?.click();
    await settle();
    expect(backend.bodyOf('POST /backup/passphrase')).toEqual({ generate: true });
    expect(text(el.querySelector('.secret'))).toBe(SECRET);
    // The copy button does not speak the secret.
    expect(el.querySelector(`[aria-label*="${SECRET}"]`)).toBeNull();
    expect(button(el, 'Done')?.disabled).toBeTrue();

    // Escape / the backdrop / ✕ are held back with a nudge.
    closeButton(el).click();
    fixture.detectChanges();
    expect(closed).not.toHaveBeenCalled();
    expect(text(el.querySelector('.notice.warn'))).toContain('I have written it down');

    const box = el.querySelector('.check input') as HTMLInputElement;
    box.checked = true;
    box.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    button(el, 'Done')?.click();
    expect(closed).toHaveBeenCalledTimes(1);

    // Closed: the passphrase is gone from the dialog's state and the page.
    fixture.componentRef.setInput('open', false);
    fixture.detectChanges();
    expect(el.textContent).not.toContain(SECRET);
    fixture.componentRef.setInput('open', true);
    fixture.detectChanges();
    expect(el.textContent).not.toContain(SECRET);
    expect(button(el, 'Generate a passphrase')).toBeDefined();
  });

  it("takes the user's own passphrase, typed twice, of at least 12 characters", async () => {
    const el = await open();
    button(el, 'Use my own')?.click();
    fixture.detectChanges();
    await settle();
    const [first, second] = [...el.querySelectorAll<HTMLInputElement>('input[name^="pp"]')];
    expect(first.type).toBe('password');
    const type = async (input: HTMLInputElement, value: string) => {
      input.value = value;
      input.dispatchEvent(new Event('input'));
      await settle();
    };
    await type(first, 'too short');
    expect(text(el)).toContain('At least 12 characters.');
    expect(button(el, 'Save in the Keychain')?.disabled).toBeTrue();
    await type(first, 'correct horse battery');
    await type(second, 'correct horse batterx');
    expect(text(el)).toContain('Type the same passphrase twice.');
    await type(second, 'correct horse battery');
    button(el, 'Save in the Keychain')?.click();
    await settle();
    expect(backend.bodyOf('POST /backup/passphrase')).toEqual({
      passphrase: 'correct horse battery',
    });
    expect(text(el.querySelector('.notice.ok'))).toContain("Saved in this Mac's Keychain");
    // Nothing typed stays in the form.
    expect(el.querySelector('input[name="pp"]')).toBeNull();
  });

  it('says which backups it re-keyed and which it left as they were', async () => {
    let el = await open();
    button(el, 'Generate a passphrase')?.click();
    await settle();
    expect([...el.querySelectorAll('.done')].map((p) => text(p))).toEqual([
      'The backups of Default and Work open with it now.',
    ]);
    fixture.destroy();
    el = await open();
    button(el, 'Use my own')?.click();
    await settle();
    const [first, second] = [...el.querySelectorAll<HTMLInputElement>('input[name^="pp"]')];
    for (const input of [first, second]) {
      input.value = 'correct horse battery';
      input.dispatchEvent(new Event('input'));
    }
    await settle();
    button(el, 'Save in the Keychain')?.click();
    await settle();
    expect([...el.querySelectorAll('.done')].map((p) => text(p))).toEqual([
      "Default's backup opens with it now.",
      'Left as it was: Personal — a backup in the folder of no account on this Mac that opens with neither passphrase.',
    ]);
  });

  it('keeps showing a generated passphrase some backups already have when the change stops', async () => {
    backend.routes.set('/backup/passphrase', () => ({
      status: 500,
      body: {
        error: {
          code: 'backup_io',
          message: 'the Keychain refused the new passphrase: User interaction is not allowed',
          applied: true,
          passphrase: SECRET,
        },
      },
    }));
    const el = await open();
    button(el, 'Generate a passphrase')?.click();
    await settle();
    // Never lost: shown, behind the same gate.
    expect(text(el.querySelector('.secret'))).toBe(SECRET);
    expect(text(el.querySelector('.notice.warn'))).toBe(
      'The change did not finish: The Keychain refused the new passphrase: User interaction is not allowed. Some backups already open with this new passphrase, so write it down as well. Setting the passphrase again finishes the change.',
    );
    expect(button(el, 'Done')?.disabled).toBeTrue();
    closeButton(el).click();
    fixture.detectChanges();
    expect(closed).not.toHaveBeenCalled();
    // The page reads its view again (the change shows as unfinished).
    expect(saved).toHaveBeenCalled();
  });

  it('says the change did not finish when some backups have the passphrase typed', async () => {
    backend.routes.set('/backup/passphrase', () => ({
      status: 500,
      body: {
        error: {
          code: 'backup_io',
          message: 'writing /b/Work/identity.age: disk full',
          applied: true,
        },
      },
    }));
    const el = await open();
    button(el, 'Use my own')?.click();
    await settle();
    const [first, second] = [...el.querySelectorAll<HTMLInputElement>('input[name^="pp"]')];
    for (const input of [first, second]) {
      input.value = 'correct horse battery';
      input.dispatchEvent(new Event('input'));
    }
    await settle();
    button(el, 'Save in the Keychain')?.click();
    await settle();
    expect(text(el.querySelector('.notice.error'))).toBe(
      'The change did not finish: Writing /b/Work/identity.age: disk full. Some backups already open with the passphrase you typed, so keep it: setting it again finishes the change.',
    );
    // What was typed stays: saving it again finishes the change.
    expect((el.querySelector('input[name="pp"]') as HTMLInputElement).value).toBe(
      'correct horse battery',
    );
    expect(saved).toHaveBeenCalled();
  });

  it('says nothing changed when no backup got the new passphrase', async () => {
    backend.routes.set('/backup/passphrase', () => ({
      status: 422,
      body: {
        error: {
          code: 'invalid_backup',
          message:
            "the backup set Work opens with neither this machine's passphrase nor the new one; enter the passphrase it was made with",
          applied: false,
        },
      },
    }));
    const el = await open();
    button(el, 'Generate a passphrase')?.click();
    await settle();
    expect(el.querySelector('.secret')).toBeNull();
    expect(text(el.querySelector('.notice.error'))).toBe(
      "Nothing changed: The backup set Work opens with neither this machine's passphrase nor the new one; enter the passphrase it was made with.",
    );
    expect(saved).not.toHaveBeenCalled();
  });

  it('opens on finishing a change that did not finish', async () => {
    const el = await open(true);
    expect(text(el.querySelector('.title'))).toBe('Finish the passphrase change');
    expect(text(el.querySelector('.notice.warn'))).toContain(
      'Set it again — the same one or a new one — to finish: every backup then opens with it.',
    );
  });
});
