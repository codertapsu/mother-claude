import { ComponentFixture, TestBed } from '@angular/core/testing';

import { ChecklistRow } from '../core/move-data';
import { MoveChecklistComponent } from './move-checklist.component';

const row = (id: string, title: string): ChecklistRow => ({
  id,
  title,
  sub: id,
  tag: '',
  haystack: `${title} ${id}`.toLowerCase(),
});

describe('MoveChecklistComponent', () => {
  let fixture: ComponentFixture<MoveChecklistComponent>;
  let el: HTMLElement;

  const render = (inputs: Record<string, unknown>) => {
    fixture = TestBed.createComponent(MoveChecklistComponent);
    for (const [k, v] of Object.entries(inputs)) fixture.componentRef.setInput(k, v);
    fixture.detectChanges();
    el = fixture.nativeElement as HTMLElement;
  };
  const text = (e: Element | null | undefined) =>
    (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const button = (label: string) =>
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b) === label);
  const search = (q: string) => {
    const input = el.querySelector<HTMLInputElement>('input[type="search"]')!;
    input.value = q;
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();
  };
  const titles = () => [...el.querySelectorAll('.title')].map((t) => text(t));

  const rows = [row('a', 'Xiangqi engine'), row('b', 'Chess clock'), row('c', 'Xiangqi openings')];

  it('selects every row the search matches, and says "matching" while searching', () => {
    render({ rows, selected: new Set(['b']) });
    const picked = jasmine.createSpy('selectAll');
    fixture.componentInstance.selectAll.subscribe(picked);
    expect(text(el.querySelector('.count'))).toBe('1 of 3 selected');
    button('Select all')?.click();
    expect(picked).toHaveBeenCalledWith(['a', 'b', 'c']);

    search('xiangqi');
    expect(titles()).toEqual(['Xiangqi engine', 'Xiangqi openings']);
    button('Select all matching')?.click();
    expect(picked).toHaveBeenCalledWith(['a', 'c']);
  });

  it('shows what the host searched for as it is, and counts what is selected while more wait', () => {
    // The host's server matched these on something the rows do not show.
    render({
      rows: [row('a', 'Engine'), row('c', 'Openings')],
      selected: new Set(['a', 'x']),
      searched: true,
      more: true,
    });
    search('xiangqi');
    expect(titles()).toEqual(['Engine', 'Openings']);
    // Two are ticked, one of them not listed: more are on the Mac.
    expect(text(el.querySelector('.count'))).toBe('2 selected');
    const cleared = jasmine.createSpy('bulk');
    fixture.componentInstance.bulk.subscribe(cleared);
    button('None')?.click();
    expect(cleared).toHaveBeenCalledWith({ ids: ['a', 'x'], on: false });
  });

  it('says when it is selecting, and why it took only what is listed', () => {
    render({ rows, selected: new Set(), selectingAll: true });
    expect(button('Selecting…')?.disabled).toBeTrue();
    fixture.componentRef.setInput('selectingAll', false);
    fixture.componentRef.setInput('selectAllError', 'Only the conversations listed were selected.');
    fixture.detectChanges();
    expect(text(el.querySelector('[role="alert"]'))).toBe(
      'Only the conversations listed were selected.',
    );
  });
});
