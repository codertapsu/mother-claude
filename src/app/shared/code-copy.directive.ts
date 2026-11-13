import { DestroyRef, Directive, ElementRef, NgZone, inject } from '@angular/core';

import { tr } from '../core/i18n/i18n';
import { NoticeService } from '../core/notice.service';
import { copyText } from './clipboard';

/** How long a Copy control says "Copied". */
export const COPIED_MS = 1600;

/**
 * Makes the Copy controls of rendered markdown work: put it on every element
 * that binds `renderMarkdown`'s HTML (`<div class="md" mcCodeCopy
 * [innerHTML]="…">`). The sanitizer strips buttons and event handlers from
 * that markup, so a code block's Copy is a `span role="button"`; this
 * listens on the host (delegated, so markup that is replaced as a message
 * streams needs nothing new) for a click, Enter, or Space (on release, as a
 * button does). It copies the block's code, and the control says "Copied"
 * (spoken too) for a moment.
 */
@Directive({
  selector: '[mcCodeCopy]',
  host: {
    '(click)': 'onClick($event)',
    '(keydown)': 'onKeydown($event)',
    '(keyup)': 'onKeyup($event)',
  },
})
export class CodeCopyDirective {
  private readonly host = inject(ElementRef<HTMLElement>).nativeElement as HTMLElement;
  private readonly notices = inject(NoticeService);
  private readonly zone = inject(NgZone);
  private readonly timers = new Map<HTMLElement, ReturnType<typeof setTimeout>>();

  constructor() {
    inject(DestroyRef).onDestroy(() => {
      for (const t of this.timers.values()) clearTimeout(t);
      this.timers.clear();
    });
  }

  onClick(e: MouseEvent): void {
    const control = this.control(e.target);
    if (!control) return;
    e.preventDefault();
    void this.copy(control);
  }

  onKeydown(e: KeyboardEvent): void {
    const control = this.control(e.target);
    if (!control || e.altKey || e.ctrlKey || e.metaKey) return;
    if (e.key === 'Enter') {
      e.preventDefault();
      void this.copy(control);
    } else if (e.key === ' ') {
      e.preventDefault(); // no page scroll; it copies on release
    }
  }

  onKeyup(e: KeyboardEvent): void {
    const control = this.control(e.target);
    if (!control || e.key !== ' ') return;
    e.preventDefault();
    void this.copy(control);
  }

  /** The Copy control an event came from, inside this host. */
  private control(target: EventTarget | null): HTMLElement | null {
    if (!(target instanceof Element)) return null;
    const c = target.closest<HTMLElement>('.md-copy');
    return c && this.host.contains(c) ? c : null;
  }

  private async copy(control: HTMLElement): Promise<void> {
    const code = control.closest('.md-pre')?.querySelector('pre code')?.textContent ?? '';
    if (!(await copyText(code))) {
      this.notices.warn(tr("Couldn't copy the code."));
      return;
    }
    const copied = tr('Copied');
    control.classList.add('copied');
    control.textContent = copied;
    control.setAttribute('aria-label', copied);
    this.notices.announce(copied);
    clearTimeout(this.timers.get(control));
    this.zone.runOutsideAngular(() => {
      this.timers.set(
        control,
        setTimeout(() => {
          this.timers.delete(control);
          control.classList.remove('copied');
          control.textContent = tr('Copy');
          control.setAttribute('aria-label', tr('Copy code'));
        }, COPIED_MS),
      );
    });
  }
}
