import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { NoticeService } from './notice.service';

describe('NoticeService', () => {
  let notices: NoticeService;

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 10, 9, 12, 0, 0));
    TestBed.configureTestingModule({ providers: [provideRouter([])] });
    notices = TestBed.inject(NoticeService);
  });

  afterEach(() => vi.useRealTimers());

  it('keeps each level up long enough to be read, errors and warnings longest', () => {
    const levels = {
      info: notices.info('Saved'),
      success: notices.success('Backed up'),
      warning: notices.warn('Skipped one'),
      error: notices.show('error', 'Failed'),
    };
    const left = () => notices.toasts().map((t) => t.text);
    expect(notices.toasts().map((t) => t.timeout)).toEqual([6_000, 5_000, 10_000, 12_000]);
    vi.advanceTimersByTime(5_001);
    expect(left()).toEqual(['Saved', 'Skipped one', 'Failed']);
    vi.advanceTimersByTime(1_000);
    expect(left()).toEqual(['Skipped one', 'Failed']);
    vi.advanceTimersByTime(6_000);
    expect(left()).toEqual([]);
    expect(Object.keys(levels).length).toBe(4);
  });

  it('stops the time while a toast is pointed at, and lets it run on after', () => {
    const id = notices.warn('Read me', { title: 'Needs a look' });
    expect(notices.toasts()[0].title).toBe('Needs a look');
    vi.advanceTimersByTime(8_000);
    notices.hold(id);
    vi.advanceTimersByTime(60_000);
    expect(notices.toasts().length, 'held while it is read').toBe(1);
    notices.release(id);
    vi.advanceTimersByTime(1_999);
    expect(notices.toasts().length).toBe(1);
    vi.advanceTimersByTime(1);
    expect(notices.toasts().length).toBe(0);
  });

  it('keeps one of the same message and at most four', () => {
    notices.info('Same');
    notices.info('Same');
    expect(notices.toasts().length).toBe(1);
    for (const t of ['a', 'b', 'c', 'd']) notices.info(t);
    expect(notices.toasts().map((t) => t.text)).toEqual(['a', 'b', 'c', 'd']);
    // One that stays until closed.
    const id = notices.info('Stays', { timeout: 0 });
    vi.advanceTimersByTime(120_000);
    expect(notices.toasts().map((t) => t.id)).toEqual([id]);
  });
});
