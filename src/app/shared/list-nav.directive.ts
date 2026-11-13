import { Directive, ElementRef, inject } from '@angular/core';

/** Where arrows and Enter edit, or pick among their own choices. */
export function isField(el: Element | null): boolean {
  if (!el) return false;
  if ((el as HTMLElement).isContentEditable) return true;
  return el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT';
}

/** Shown on the page (a collapsed or hidden item is skipped). */
function shown(el: HTMLElement): boolean {
  return el.getClientRects().length > 0;
}

/** What takes focus for an item: itself when it can, else its link. */
export function focusTarget(item: HTMLElement): HTMLElement {
  if (item.matches('a[href], button, [tabindex]')) return item;
  return item.querySelector<HTMLElement>('[data-nav-open]') ?? item;
}

/**
 * Keys for a list of items, as a mail list has them (docs/UI-REDESIGN.md
 * §9, stage 9): ↓ and ↑ move from item to item, and Enter on an item
 * opens it. The list marks its items `data-nav-item`, and in each the link
 * that opens it `data-nav-open` (an item that cannot take focus itself,
 * such as a session's card, focuses that link); `data-nav-entry` on a
 * field (the list's search) lets ↓ go from it into the list. Keys typed in
 * a field, with a modifier, or that a control used already are left alone,
 * and so are arrows away from the items (they scroll).
 */
@Directive({
  selector: '[mcListNav]',
  host: { '(keydown)': 'onKey($event)' },
})
export class ListNavDirective {
  private readonly host = inject(ElementRef<HTMLElement>).nativeElement as HTMLElement;

  onKey(e: KeyboardEvent): void {
    if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
    const down = e.key === 'ArrowDown';
    const up = e.key === 'ArrowUp';
    if (!down && !up && e.key !== 'Enter') return;
    const target = e.target instanceof HTMLElement ? e.target : null;
    if (!target) return;
    const items = this.items();
    if (isField(target)) {
      if (down && target.hasAttribute('data-nav-entry') && items.length) {
        e.preventDefault();
        focusTarget(items[0]).focus();
      }
      return;
    }
    const i = items.findIndex((el) => el === target || el.contains(target));
    if (i < 0) return;
    if (e.key === 'Enter') {
      // A link or a button inside the item does its own.
      if (items[i] !== target) return;
      const open = items[i].querySelector<HTMLElement>('[data-nav-open]');
      if (!open) return;
      e.preventDefault();
      open.click();
      return;
    }
    const next = items[down ? i + 1 : i - 1];
    if (!next) return;
    e.preventDefault();
    focusTarget(next).focus();
  }

  /** The list's items, in their order on the page. */
  private items(): HTMLElement[] {
    return [...this.host.querySelectorAll<HTMLElement>('[data-nav-item]')].filter(shown);
  }
}
