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
        ? { status: 200, body: { passphrase: SECRET } }
        : { status: 200, body: { passphrase: null } },
    );
  });

  afterEach(() => setActiveMock(null));

  const open = async () => {
    fixture = TestBed.createComponent(PassphraseSheetComponent);
    closed = jasmine.createSpy('closed');
    fixture.componentInstance.closed.subscribe(closed);
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
});
