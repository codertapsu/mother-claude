import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { PageHeadComponent, PageMenuDirective, PageState } from './page-head.component';

@Component({
  imports: [PageHeadComponent, PageMenuDirective],
  template: `
    <mc-page-head heading="Computers" [lede]="lede()" [help]="help()" [state]="state()">
      <button class="btn primary" type="button">Pair a computer</button>
      @if (withMenu()) {
        <button class="btn" type="button" mcPageMenu (click)="chosen.set('setup')">
          Run the setup again
        </button>
      }
    </mc-page-head>
  `,
})
class HostComponent {
  readonly lede = signal('Use your other computers from here.');
  readonly help = signal('machines-setup');
  readonly state = signal<PageState | null>({ tone: 'ok', text: 'All good' });
  readonly withMenu = signal(true);
  readonly chosen = signal('');
}

describe('PageHeadComponent', () => {
  beforeEach(() => TestBed.configureTestingModule({ providers: [provideRouter([])] }));

  const text = (e: Element | null | undefined) =>
    (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const render = () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.detectChanges();
    return { fixture, el: fixture.nativeElement as HTMLElement };
  };

  it('shows the name, its state in the Machines words, and the primary action', () => {
    const { el } = render();
    expect(text(el.querySelector('h1'))).toBe('Computers');
    const badge = el.querySelector('.badge.state');
    expect(text(badge)).toBe('✓ All good');
    expect(badge?.classList).toContain('ok');
    expect(text(el.querySelector('.page-head .btn.primary'))).toBe('Pair a computer');
  });

  it('marks a state that works but worse, and one that blocks', () => {
    const { fixture, el } = render();
    fixture.componentInstance.state.set({ tone: 'warn', text: '1 recommended' });
    fixture.detectChanges();
    expect(text(el.querySelector('.badge.state'))).toBe('! 1 recommended');
    fixture.componentInstance.state.set({ tone: 'danger', text: '2 required' });
    fixture.detectChanges();
    expect(text(el.querySelector('.badge.state'))).toBe('✕ 2 required');
    expect(el.querySelector('.badge.state')?.classList).toContain('danger');
    fixture.componentInstance.state.set(null);
    fixture.detectChanges();
    expect(el.querySelector('.badge.state')).toBeNull();
  });

  it('says what the place is for in one sentence, with How it works › to its Help topic', () => {
    const { fixture, el } = render();
    const lede = el.querySelector('p.lede');
    expect(text(lede)).toBe('Use your other computers from here. How it works ›');
    expect(lede?.querySelector('a')?.getAttribute('href')).toBe('/help/machines-setup');

    fixture.componentInstance.help.set('');
    fixture.detectChanges();
    expect(el.querySelector('p.lede a')).toBeNull();
    fixture.componentInstance.lede.set('');
    fixture.detectChanges();
    expect(el.querySelector('p.lede')).toBeNull();
  });

  it('puts the other actions under ⋯, and closes it when one is chosen', () => {
    const { fixture, el } = render();
    const more = el.querySelector<HTMLButtonElement>('button[aria-label="More actions"]');
    expect(more).not.toBeNull();
    const dialog = el.querySelector('dialog')!;
    expect(dialog.open).toBe(false);
    more!.click();
    fixture.detectChanges();
    expect(dialog.open).toBe(true);

    const item = [...dialog.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => text(b) === 'Run the setup again',
    )!;
    item.click();
    fixture.detectChanges();
    expect(fixture.componentInstance.chosen()).toBe('setup');
    expect(dialog.open).toBe(false);
  });

  it('shows no ⋯ without other actions', () => {
    const { fixture, el } = render();
    fixture.componentInstance.withMenu.set(false);
    fixture.detectChanges();
    expect(el.querySelector('button[aria-label="More actions"]')).toBeNull();
    // Nor its sheet: a page's own dialogs are the only ones there.
    expect(el.querySelector('dialog')).toBeNull();
  });
});
