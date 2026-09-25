import { normalizePairCode } from './pair.component';

describe('normalizePairCode', () => {
  it('uppercases and drops separators', () => {
    expect(normalizePairCode('abcd-2345 wxyz')).toBe('ABCD2345WXYZ');
  });

  it('maps look-alike letters the Crockford way', () => {
    expect(normalizePairCode('O0-Il')).toBe('0011');
  });
});
