import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { MachinesSettings } from '../../core/machines';
import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { OptionsFormComponent } from './options-form.component';

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
});
