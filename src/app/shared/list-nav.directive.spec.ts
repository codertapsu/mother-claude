import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';

import { ListNavDirective, focusTarget, isField } from './list-nav.directive';

/** A list as the app's are: a search field, cards whose name is the link,
 * a card that takes focus itself, a link that is its own item, a hidden
 * item, and something that is no item. */
@Component({
  imports: [ListNavDirective],
  template: `
    <div mcListNav>
      <input class="search" data-nav-entry />
      <select class="pick">
        <option>1</option>
      </select>
      <div class="card" data-nav-item id="c1">
        <a data-nav-open href="#one" (click)="open('one', $event)">one</a>
        <button type="button" class="inner">Allow</button>
      </div>
      <div class="card" data-nav-item id="c2" tabindex="0">
        <span>two</span>
        <a data-nav-open href="#two" (click)="open('two', $event)">open two</a>
      </div>
      <div class="card" data-nav-item hidden id="gone">
        <a data-nav-open href="#gone">gone</a>
      </div>
      <a data-nav-item href="#three" id="c3" (click)="open('three', $event)">three</a>
      <button type="button" class="other">not an item</button>
    </div>
  `,
})
class HostComponent {
  opened: string[] = [];
  open(what: string, e: Event): void {
    e.preventDefault();
    this.opened.push(what);
  }
}

describe('mcListNav: ↓/↑ move through a list, Enter opens', () => {
  let fixture: ComponentFixture<HostComponent>;
  let root: HTMLElement;

  beforeEach(() => {
    fixture = TestBed.createComponent(HostComponent);
    fixture.detectChanges();
    root = fixture.nativeElement as HTMLElement;
    // Focus needs it in the page.
    if (!root.isConnected) document.body.appendChild(root);
  });

  const $ = (sel: string) => root.querySelector(sel) as HTMLElement;
  const press = (el: HTMLElement, key: string, init: KeyboardEventInit = {}) => {
    el.focus();
    const e = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init });
    el.dispatchEvent(e);
    return e;
  };

  it('moves down and up through the items, focusing a card’s link', () => {
    const one = $('#c1 a');
    expect(press(one, 'ArrowDown').defaultPrevented).toBe(true);
    // The second card takes focus itself.
    expect(document.activeElement).toBe($('#c2'));
    press($('#c2'), 'ArrowDown');
    // The hidden item is skipped: the link that is its own item.
    expect(document.activeElement).toBe($('#c3'));
    press($('#c3'), 'ArrowUp');
    expect(document.activeElement).toBe($('#c2'));
    press($('#c2'), 'ArrowUp');
    expect(document.activeElement).toBe(one);
  });

  it('moves from a button inside an item too, and stops at the ends', () => {
    press($('#c1 .inner'), 'ArrowDown');
    expect(document.activeElement).toBe($('#c2'));
    const top = press($('#c1 a'), 'ArrowUp');
    expect(top.defaultPrevented).toBe(false);
    expect(document.activeElement).toBe($('#c1 a'));
    const bottom = press($('#c3'), 'ArrowDown');
    expect(bottom.defaultPrevented).toBe(false);
  });

  it('goes from the search field into the list with ↓', () => {
    expect(press($('.search'), 'ArrowDown').defaultPrevented).toBe(true);
    expect(document.activeElement).toBe($('#c1 a'));
  });

  it('opens a focused card with Enter, through its link', () => {
    const e = press($('#c2'), 'Enter');
    expect(e.defaultPrevented).toBe(true);
    expect(fixture.componentInstance.opened).toEqual(['two']);
  });

  it('leaves Enter on a link or a button to it', () => {
    expect(press($('#c1 a'), 'Enter').defaultPrevented).toBe(false);
    expect(press($('#c1 .inner'), 'Enter').defaultPrevented).toBe(false);
    expect(fixture.componentInstance.opened).toEqual([]);
  });

  it('leaves fields, modifiers and what is no item alone', () => {
    expect(press($('.pick'), 'ArrowDown').defaultPrevented).toBe(false);
    expect(press($('#c1 a'), 'ArrowDown', { shiftKey: true }).defaultPrevented).toBe(false);
    expect(press($('#c1 a'), 'ArrowDown', { metaKey: true }).defaultPrevented).toBe(false);
    expect(press($('#c1 a'), 'ArrowDown', { altKey: true }).defaultPrevented).toBe(false);
    expect(press($('.other'), 'ArrowDown').defaultPrevented).toBe(false);
    expect(document.activeElement).toBe($('.other'));
  });

  it('tells fields from the rest, and what an item focuses', () => {
    expect(isField($('.search'))).toBe(true);
    expect(isField($('.pick'))).toBe(true);
    expect(isField($('.other'))).toBe(false);
    expect(isField(null)).toBe(false);
    expect(focusTarget($('#c1'))).toBe($('#c1 a'));
    expect(focusTarget($('#c2'))).toBe($('#c2'));
    expect(focusTarget($('#c3'))).toBe($('#c3'));
  });
});
