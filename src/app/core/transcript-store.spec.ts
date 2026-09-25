import { UiEvent } from './models';
import { spliceTail } from './transcript-store';

const e = (id: string): UiEvent => ({ id, role: 'assistant', kind: 'text', text: id });
const ids = (list: UiEvent[]) => list.map((x) => x.id);

describe('spliceTail', () => {
  it('keeps older history and appends what the page adds', () => {
    const prev = ['1', '2', '3', '4'].map(e);
    const page = ['3', '4', '5', '6'].map(e);
    expect(ids(spliceTail(prev, page))).toEqual(['1', '2', '3', '4', '5', '6']);
  });

  it('keeps live events the page does not have yet, after the page', () => {
    const prev = ['1', '2', '3', '9'].map(e);
    const page = ['2', '3', '4'].map(e);
    expect(ids(spliceTail(prev, page))).toEqual(['1', '2', '3', '4', '9']);
  });

  it('lets the page win when it reaches further back', () => {
    const prev = ['5', '6'].map(e);
    const page = ['3', '4', '5', '6', '7'].map(e);
    expect(ids(spliceTail(prev, page))).toEqual(['3', '4', '5', '6', '7']);
  });

  it('replaces known events with the page copy', () => {
    const prev = [e('1'), { ...e('2'), text: 'old' }];
    const page = [e('1'), { ...e('2'), text: 'new' }];
    expect(spliceTail(prev, page)[1].text).toBe('new');
  });
});
