import { isStillPending } from './app.component';

describe('isStillPending', () => {
  it('drops expired and settled pair requests', () => {
    const now = 10_000;
    expect(isStillPending({ id: 'a', expiresAt: now + 1 }, now)).toBeTrue();
    expect(isStillPending({ id: 'b' }, now)).toBeTrue();
    expect(isStillPending({ id: 'c', expiresAt: now }, now)).toBeFalse();
    expect(isStillPending({ id: 'd', status: 'denied', expiresAt: now + 1 }, now)).toBeFalse();
    expect(isStillPending({ id: 'e', status: 'approved' }, now)).toBeFalse();
  });
});
