import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { AttentionCardComponent } from './attention-card.component';

@Component({
  imports: [AttentionCardComponent],
  template: `
    <mc-attention-card
      heading="tray-tool changed on both computers"
      why="Nothing syncs until you choose which side to keep."
      meta="code sync"
      kind="sync"
      [tone]="tone()"
      [machine]="machine()"
      [when]="when()"
    >
      @if (action()) {
        <button class="btn small primary" type="button">Choose…</button>
      }
    </mc-attention-card>
  `,
})
class HostComponent {
  readonly tone = signal<'danger' | 'warn' | 'info'>('warn');
  readonly machine = signal('DESKTOP-PC');
  readonly when = signal<number | null>(Date.now() - 5 * 60_000);
  readonly action = signal(true);
}

describe('AttentionCardComponent', () => {
  const text = (e: Element | null | undefined) =>
    (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const render = () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.detectChanges();
    return { fixture, el: fixture.nativeElement as HTMLElement };
  };

  it('says what waits, why, from where, and offers its action', () => {
    const { el } = render();
    const card = el.querySelector('mc-attention-card')!;
    expect(card.getAttribute('data-kind')).toBe('sync');
    expect(card.querySelector('article')?.getAttribute('aria-label')).toBe(
      'tray-tool changed on both computers',
    );
    expect(text(card.querySelector('.title'))).toBe('tray-tool changed on both computers');
    expect(text(card.querySelector('.meta'))).toBe('code sync');
    expect(text(card.querySelector('.mchip'))).toBe('DESKTOP-PC');
    expect(text(card.querySelector('.when'))).toBe('5m ago');
    expect(text(card.querySelector('.why'))).toBe(
      'Nothing syncs until you choose which side to keep.',
    );
    expect(text(card.querySelector('.acts .btn.primary'))).toBe('Choose…');
  });

  it('marks how it stands in the Machines words, and says it to screen readers', () => {
    const { fixture, el } = render();
    const mark = () => text(el.querySelector('.mark'));
    const said = () => text(el.querySelector('.top .visually-hidden'));
    expect([mark(), said()]).toEqual(['!', 'Works, but worse:']);
    fixture.componentInstance.tone.set('danger');
    fixture.detectChanges();
    expect([mark(), said()]).toEqual(['✕', 'Blocks:']);
    expect(el.querySelector('mc-attention-card')?.getAttribute('data-tone')).toBe('danger');
    fixture.componentInstance.tone.set('info');
    fixture.detectChanges();
    expect([mark(), said()]).toEqual(['●', '']);
  });

  it('leaves out the computer, the time and the actions it was not given', () => {
    const { fixture, el } = render();
    fixture.componentInstance.machine.set('');
    fixture.componentInstance.when.set(null);
    fixture.componentInstance.action.set(false);
    fixture.detectChanges();
    expect(el.querySelector('.mchip')).toBeNull();
    expect(el.querySelector('.when')).toBeNull();
    expect(getComputedStyle(el.querySelector('.acts')!).display).toBe('none');
  });
});
