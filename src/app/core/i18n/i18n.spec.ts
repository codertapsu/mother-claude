import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { relativeTime } from '../../shared/util';
import { plural } from '../data-jobs';
import { computerNoun, fill, hostOsSignal, language, locale, tr, useLanguage } from './i18n';
import { TPipe } from './t.pipe';

@Component({
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [TPipe],
  template: `<h1>{{ 'Settings' | t }}</h1>
    <p>{{ 'Theme: {theme}' | t: { theme: 'x' } }}</p>`,
})
class ProbeComponent {}

describe('i18n', () => {
  afterEach(async () => {
    await useLanguage('en');
    hostOsSignal.set(undefined);
    localStorage.removeItem('mc_lang');
  });

  it('shows the English key, without its note, with its placeholders filled', () => {
    expect(tr('Back up now')).toBe('Back up now');
    expect(tr('System||theme')).toBe('System');
    expect(tr('Backed up {time}', { time: '2h ago' })).toBe('Backed up 2h ago');
    expect(fill('{a} and {b}', { a: 1 })).toBe('1 and {b}');
  });

  it('switches to Vietnamese, loading its dictionary, and back', async () => {
    await useLanguage('vi', true);
    expect(language()).toBe('vi');
    expect(document.documentElement.lang).toBe('vi');
    expect(localStorage.getItem('mc_lang')).toBe('vi');
    expect(tr('Settings')).toBe('Cài đặt');
    expect(tr('System||theme')).toBe('Theo hệ thống');
    expect(tr('Theme: {theme}', { theme: 'Tối' })).toBe('Giao diện: Tối');
    // Not translated yet: English.
    expect(tr('A string nobody translated')).toBe('A string nobody translated');
    expect(plural(3, 'session')).toBe('3 phiên');
    expect(relativeTime(Date.now() - 2 * 3_600_000)).toBe('2 giờ trước');
    expect(locale()).toBe('vi-VN');
    await useLanguage('en');
    expect(tr('Settings')).toBe('Settings');
    expect(plural(3, 'session')).toBe('3 sessions');
  });

  it("names the desktop's computer after its system", async () => {
    hostOsSignal.set('windows');
    expect(tr('Answer it on the {computer}.', { computer: computerNoun() })).toBe(
      'Answer it on the PC.',
    );
    hostOsSignal.set('macos');
    expect(computerNoun()).toBe('Mac');
    await useLanguage('vi');
    expect(computerNoun()).toBe('máy Mac');
  });

  it('switches a view at once, OnPush included', async () => {
    const fixture = TestBed.createComponent(ProbeComponent);
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelector('h1')?.textContent).toBe('Settings');
    await useLanguage('vi');
    await fixture.whenStable();
    fixture.detectChanges();
    expect(el.querySelector('h1')?.textContent).toBe('Cài đặt');
    expect(el.querySelector('p')?.textContent).toBe('Giao diện: x');
  });
});
