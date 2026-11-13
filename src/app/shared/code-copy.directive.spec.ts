import { Component, signal } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';

import { NoticeService } from '../core/notice.service';
import { COPIED_MS, CodeCopyDirective } from './code-copy.directive';
import { renderMarkdown } from './markdown';

/** A message as the transcript shows one: rendered markdown bound through
 * the sanitizer, with two code blocks. */
@Component({
  imports: [CodeCopyDirective],
  template: `<div class="md" mcCodeCopy [innerHTML]="html()"></div>`,
})
class HostComponent {
  readonly html = signal(
    renderMarkdown(
      'Run:\n\n```ts\nconst ok = 1 < 2 && "yes";\n  indented();\n```\n\n```\nsecond\n```',
    ),
  );
}

/** Lets the clipboard's promises settle. */
async function settle(): Promise<void> {
  for (let i = 0; i < 10; i++) await Promise.resolve();
}

describe('mcCodeCopy: the Copy control of a code block', () => {
  let fixture: ComponentFixture<HostComponent>;
  let root: HTMLElement;
  let notices: { warn: ReturnType<typeof vi.fn>; announce: ReturnType<typeof vi.fn> };

  beforeEach(() => {
    notices = { warn: vi.fn(), announce: vi.fn() };
    TestBed.configureTestingModule({ providers: [{ provide: NoticeService, useValue: notices }] });
    fixture = TestBed.createComponent(HostComponent);
    fixture.detectChanges();
    root = fixture.nativeElement as HTMLElement;
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  const controls = () => Array.from(root.querySelectorAll<HTMLElement>('.md-copy'));
  const key = (el: HTMLElement, type: 'keydown' | 'keyup', k: string) => {
    const e = new KeyboardEvent(type, { key: k, bubbles: true, cancelable: true });
    el.dispatchEvent(e);
    return e;
  };

  it('survives the sanitizer as a focusable, labelled button', () => {
    const [first] = controls();
    expect(controls().length).toBe(2);
    expect(first.getAttribute('role')).toBe('button');
    expect(first.getAttribute('tabindex')).toBe('0');
    expect(first.getAttribute('aria-label')).toBe('Copy code');
    expect(first.textContent).toBe('Copy');
    expect(root.querySelector('.md-lang')?.textContent).toBe('ts');
  });

  it("copies its own block's code on a click, says so for a moment, then goes back", async () => {
    vi.useFakeTimers();
    const write = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined);
    const [first, second] = controls();
    first.click();
    await settle();
    expect(write).toHaveBeenCalledWith('const ok = 1 < 2 && "yes";\n  indented();');
    expect(first.textContent).toBe('Copied');
    expect(first.classList).toContain('copied');
    expect(first.getAttribute('aria-label')).toBe('Copied');
    expect(notices.announce).toHaveBeenCalledWith('Copied');
    expect(second.textContent).toBe('Copy');

    vi.advanceTimersByTime(COPIED_MS - 1);
    expect(first.textContent).toBe('Copied');
    vi.advanceTimersByTime(1);
    expect(first.textContent).toBe('Copy');
    expect(first.classList).not.toContain('copied');
    expect(first.getAttribute('aria-label')).toBe('Copy code');
  });

  it('copies on Enter, and on Space once it is released, without scrolling', async () => {
    const write = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined);
    const second = controls()[1];
    expect(key(second, 'keydown', 'Enter').defaultPrevented).toBe(true);
    await settle();
    expect(write).toHaveBeenLastCalledWith('second');

    write.mockClear();
    expect(key(second, 'keydown', ' ').defaultPrevented, 'no page scroll').toBe(true);
    await settle();
    expect(write).not.toHaveBeenCalled();
    expect(key(second, 'keyup', ' ').defaultPrevented).toBe(true);
    await settle();
    expect(write).toHaveBeenCalledWith('second');
  });

  it('leaves other clicks and keys alone', async () => {
    const write = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined);
    const click = new MouseEvent('click', { bubbles: true, cancelable: true });
    root.querySelector('code')?.dispatchEvent(click);
    expect(click.defaultPrevented).toBe(false);
    expect(key(controls()[0], 'keydown', 'a').defaultPrevented).toBe(false);
    await settle();
    expect(write).not.toHaveBeenCalled();
  });

  it('falls back to a hidden text field where the Clipboard API fails', async () => {
    vi.spyOn(navigator.clipboard, 'writeText').mockRejectedValue(new Error('denied'));
    const exec = vi.spyOn(document, 'execCommand').mockReturnValue(true);
    controls()[1].click();
    await settle();
    expect(exec).toHaveBeenCalledWith('copy');
    expect(controls()[1].textContent).toBe('Copied');
    expect(document.querySelector('textarea')).toBeNull();
  });

  it('says so when the code could not be copied at all', async () => {
    vi.spyOn(navigator.clipboard, 'writeText').mockRejectedValue(new Error('denied'));
    vi.spyOn(document, 'execCommand').mockReturnValue(false);
    controls()[0].click();
    await settle();
    expect(notices.warn).toHaveBeenCalledWith("Couldn't copy the code.");
    expect(controls()[0].textContent).toBe('Copy');
    expect(notices.announce).not.toHaveBeenCalled();
  });

  it('works for markup replaced after it started, as a streaming message is', async () => {
    const write = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined);
    fixture.componentInstance.html.set(renderMarkdown('- step\n  ```sh\n  npm test\n  ```'));
    fixture.detectChanges();
    expect(controls().length).toBe(1);
    controls()[0].click();
    await settle();
    expect(write).toHaveBeenCalledWith('npm test');
  });
});
