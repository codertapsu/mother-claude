import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { EmptyStateComponent } from './empty-state.component';

@Component({
  imports: [EmptyStateComponent],
  template: `
    <mc-empty-state
      heading="No phone paired yet"
      [text]="text()"
      [help]="help()"
      icon="phone"
      tone="accent"
    >
      @if (action()) {
        <button class="btn primary" type="button">Pair a phone</button>
      }
    </mc-empty-state>
  `,
})
class HostComponent {
  readonly text = signal('Pair your phone to answer requests away from this computer.');
  readonly help = signal('phone');
  readonly action = signal(true);
}

describe('EmptyStateComponent', () => {
  beforeEach(() => TestBed.configureTestingModule({ providers: [provideRouter([])] }));

  const text = (e: Element | null | undefined) =>
    (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const render = () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.detectChanges();
    return { fixture, el: fixture.nativeElement as HTMLElement };
  };

  it('says what will show here and offers the one thing that makes it show', () => {
    const { el } = render();
    const card = el.querySelector('.card.empty-state')!;
    expect(card.getAttribute('data-tone')).toBe('accent');
    expect(card.querySelector('mc-icon')).not.toBeNull();
    expect(text(card.querySelector('strong'))).toBe('No phone paired yet');
    expect(text(card.querySelector('p'))).toBe(
      'Pair your phone to answer requests away from this computer.',
    );
    expect(text(card.querySelector('.acts .btn.primary'))).toBe('Pair a phone');
    const how = card.querySelector<HTMLAnchorElement>('.acts a.how')!;
    expect(text(how)).toBe('How it works ›');
    expect(how.getAttribute('href')).toBe('/help/phone');
  });

  it('leaves out what it was not given', () => {
    const { fixture, el } = render();
    fixture.componentInstance.text.set('');
    fixture.componentInstance.help.set('');
    fixture.componentInstance.action.set(false);
    fixture.detectChanges();
    expect(el.querySelector('.empty-state p')).toBeNull();
    expect(el.querySelector('.empty-state a')).toBeNull();
    const acts = el.querySelector('.acts') as HTMLElement;
    expect(getComputedStyle(acts).display).toBe('none');
  });
});
