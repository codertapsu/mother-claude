import { TestBed } from '@angular/core/testing';

import { FakeBackend, asDesktop, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { CheckListComponent } from './check-list.component';
import { check } from './machines.testing';

const text = (e: Element | null | undefined) => (e?.textContent ?? '').replace(/\s+/g, ' ').trim();

describe('CheckListComponent', () => {
  beforeEach(() => useFakeServer(new FakeBackend(), []));
  afterEach(() => setActiveMock(null));

  const render = (readonly: boolean) => {
    const fixture = TestBed.createComponent(CheckListComponent);
    fixture.componentRef.setInput('checks', [
      check({
        id: 'awake',
        title: 'Stays awake while used',
        state: 'warning',
        severity: 'recommended',
        fix: { kind: 'auto', label: 'Keep awake while used' },
      }),
    ]);
    fixture.componentRef.setInput('readonly', readonly);
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  };

  it("offers a failing check's fix on the desktop", () => {
    asDesktop();
    const el = render(false);
    const fix = el.querySelector<HTMLButtonElement>('.cacts button')!;
    expect(text(fix)).toBe('Keep awake while used');
    expect(fix.disabled).toBe(false);
  });

  it('shows a phone the fix and where it is done, never a disabled button', () => {
    const el = render(true);
    expect(el.querySelector('.cacts button')).toBeNull();
    const line = el.querySelector('.cacts mc-desktop-only')!;
    expect(text(line.querySelector('.value'))).toBe('Keep awake while used');
    expect(text(line)).toContain('Fix it on the desktop app.');
  });
});
