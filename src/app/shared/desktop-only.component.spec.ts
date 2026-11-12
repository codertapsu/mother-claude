import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { AuthService } from '../core/auth.service';
import { me } from '../core/mock/fake-server.testing';
import { DesktopOnlyComponent } from './desktop-only.component';

@Component({
  imports: [DesktopOnlyComponent],
  template: `
    <mc-desktop-only [value]="value()" [line]="line()">
      <select aria-label="Read usage every">
        <option>5 min</option>
      </select>
    </mc-desktop-only>
  `,
})
class HostComponent {
  readonly value = signal('5 min');
  readonly line = signal('');
}

describe('DesktopOnlyComponent', () => {
  const text = (e: Element | null | undefined) =>
    (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const render = (principal: 'desktop' | 'device') => {
    TestBed.inject(AuthService).me.set(me({ principal }));
    const fixture = TestBed.createComponent(HostComponent);
    fixture.detectChanges();
    return { fixture, el: fixture.nativeElement as HTMLElement };
  };

  it('gives the desktop app its control', () => {
    const { el } = render('desktop');
    expect(el.querySelector('select')).not.toBeNull();
    expect(el.querySelector('.line')).toBeNull();
  });

  it('shows another device the value and where it changes — never a disabled control', () => {
    const { el } = render('device');
    expect(el.querySelector('select')).toBeNull();
    expect(text(el.querySelector('.value'))).toBe('5 min');
    expect(text(el.querySelector('.line'))).toBe('Changed on the desktop app.');
  });

  it('says where else it is done, with no value to show', () => {
    const { fixture, el } = render('device');
    fixture.componentInstance.value.set('');
    fixture.componentInstance.line.set('Answer on the desktop app.');
    fixture.detectChanges();
    expect(el.querySelector('.value')).toBeNull();
    expect(text(el.querySelector('.line'))).toBe('Answer on the desktop app.');
  });
});
