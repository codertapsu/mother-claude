import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { useLanguage } from '../../core/i18n/i18n';
import { MachinesSettings } from '../../core/machines';
import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { serverText } from './machines-text';
import { OptionsFormComponent, relayText } from './options-form.component';

describe('OptionsFormComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<OptionsFormComponent>;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([])]);
    asDesktop();
    backend.routes.set('/machines/settings', (_method, body) => ({ status: 200, body }));
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (el: Element, label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).startsWith(label));
  const settle = async () => {
    for (let i = 0; i < 3; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const type = async (input: HTMLInputElement, value: string) => {
    input.value = value;
    input.dispatchEvent(new Event('input'));
    await settle();
  };

  it('saves your own relay, and refuses an address that is not https', async () => {
    fixture = TestBed.createComponent(OptionsFormComponent);
    fixture.componentRef.setInput('settings', {
      relay: { mode: 'public', url: null },
      keepAwake: true,
    } satisfies MachinesSettings);
    const saved: MachinesSettings[] = [];
    fixture.componentInstance.saved.subscribe((s) => saved.push(s));
    fixture.detectChanges();
    await settle();
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelector<HTMLInputElement>('input[value="public"]')!.checked).toBe(true);
    expect(el.querySelector('input[name="url"]')).toBeNull();

    el.querySelector<HTMLInputElement>('input[value="custom"]')!.click();
    await settle();
    const url = el.querySelector<HTMLInputElement>('input[name="url"]')!;
    await type(url, 'relay.example.com');
    button(el, 'Save')!.click();
    await settle();
    expect(text(el.querySelector('[role="alert"]'))).toBe(
      "Your relay's address must start with https://.",
    );
    expect(backend.calls).not.toContain('PUT /machines/settings');

    await type(url, 'https://relay.example.com');
    el.querySelector<HTMLInputElement>('input[name="keepAwake"]')!.click();
    await settle();
    button(el, 'Save')!.click();
    await settle();
    const code = { transfer: 'bundle', apply: 'checkout', keepRemoteRefs: false };
    expect(backend.bodyOf('PUT /machines/settings')).toEqual({
      relay: { mode: 'custom', url: 'https://relay.example.com' },
      keepAwake: false,
      code,
    });
    expect(saved).toEqual([
      { relay: { mode: 'custom', url: 'https://relay.example.com' }, keepAwake: false, code },
    ]);
  });

  it('keeps the code options it was given, and offers keeping hidden refs only for a remote', async () => {
    fixture = TestBed.createComponent(OptionsFormComponent);
    fixture.componentRef.setInput('settings', {
      relay: { mode: 'public', url: null },
      keepAwake: true,
      code: { transfer: 'bundle', apply: 'worktree', keepRemoteRefs: false },
    } satisfies MachinesSettings);
    fixture.detectChanges();
    await settle();
    const el = fixture.nativeElement as HTMLElement;
    expect(
      el.querySelector<HTMLInputElement>('input[name="apply"][value="worktree"]')!.checked,
    ).toBe(true);
    expect(el.querySelector('input[name="keepRemoteRefs"]')).toBeNull();
    el.querySelector<HTMLInputElement>('input[name="transfer"][value="remote"]')!.click();
    await settle();
    expect(el.querySelector('input[name="keepRemoteRefs"]')).not.toBeNull();
    button(el, 'Save')!.click();
    await settle();
    expect(backend.bodyOf('PUT /machines/settings')).toEqual({
      relay: { mode: 'public', url: null },
      keepAwake: true,
      code: { transfer: 'remote', apply: 'worktree', keepRemoteRefs: false },
    });
  });

  it("pins your own relay's certificate: 64 hex digits over https, kept lowercase", async () => {
    const hex = 'ab'.repeat(32);
    fixture = TestBed.createComponent(OptionsFormComponent);
    fixture.componentRef.setInput('settings', {
      relay: { mode: 'custom', url: 'https://relay.example.com', certSha256: hex },
      keepAwake: true,
    } satisfies MachinesSettings);
    fixture.detectChanges();
    await settle();
    const el = fixture.nativeElement as HTMLElement;
    const pin = el.querySelector<HTMLInputElement>('input[name="certSha256"]')!;
    expect(pin.value).toBe(hex);

    await type(pin, 'abcd');
    button(el, 'Save')!.click();
    await settle();
    expect(text(el.querySelector('[role="alert"]'))).toBe(
      "A certificate's SHA-256 is 64 hexadecimal digits (colons allowed).",
    );
    expect(backend.calls).not.toContain('PUT /machines/settings');

    // As openssl prints it: colons, capitals.
    const colons = hex.toUpperCase().match(/../g)!.join(':');
    await type(pin, colons);
    const url = el.querySelector<HTMLInputElement>('input[name="url"]')!;
    await type(url, 'http://localhost:3340');
    button(el, 'Save')!.click();
    await settle();
    expect(text(el.querySelector('[role="alert"]'))).toBe(
      'Pinning a certificate needs an https:// address.',
    );
    await type(url, 'https://relay.example.com');
    button(el, 'Save')!.click();
    await settle();
    expect(backend.bodyOf('PUT /machines/settings')).toMatchObject({
      relay: { mode: 'custom', url: 'https://relay.example.com', certSha256: hex },
    });

    // Cleared, no pin is sent: none is kept.
    await type(pin, '');
    button(el, 'Save')!.click();
    await settle();
    const last = backend.bodyOf('PUT /machines/settings') as { relay: Record<string, unknown> };
    expect(last.relay).toEqual({ mode: 'custom', url: 'https://relay.example.com' });
  });

  it("names a pinned certificate in the relay's words", () => {
    const hex = '0123456789ab'.padEnd(64, 'f');
    expect(relayText({ mode: 'custom', url: 'https://r.example', certSha256: hex })).toBe(
      'Your own relay: https://r.example, its certificate pinned (0123456789ab…)',
    );
    expect(relayText({ mode: 'custom', url: 'https://r.example' })).toBe(
      'Your own relay: https://r.example',
    );
  });
  it('reads the relay check of a pinned certificate in the language shown', async () => {
    const other =
      "Your relay presented another certificate than the one you pinned (its SHA-256 starts 0123456789abcdef…). If you renewed it, enter its new SHA-256 in Options; if not, something is in the way. Across networks your computers can't meet until then.";
    expect(serverText(other)).toBe(other);
    await useLanguage('vi');
    try {
      expect(serverText('Connected to relay.example.com, with the certificate you pinned.')).toBe(
        'Đã kết nối tới relay.example.com, với chứng chỉ bạn đã ghim.',
      );
      expect(serverText('Connected to relay.example.com.')).toBe(
        'Đã kết nối tới relay.example.com.',
      );
      expect(serverText(other)).toContain('bắt đầu bằng 0123456789abcdef…');
    } finally {
      await useLanguage('en');
    }
  });
});
