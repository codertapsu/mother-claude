import { isStillPending } from './app.component';

describe('isStillPending', () => {
  it('drops expired and settled pair requests', () => {
    const now = 10_000;
    expect(isStillPending({ id: 'a', expiresAt: now + 1 }, now)).toBe(true);
    expect(isStillPending({ id: 'b' }, now)).toBe(true);
    expect(isStillPending({ id: 'c', expiresAt: now }, now)).toBe(false);
    expect(isStillPending({ id: 'd', status: 'denied', expiresAt: now + 1 }, now)).toBe(false);
    expect(isStillPending({ id: 'e', status: 'approved' }, now)).toBe(false);
  });
});
