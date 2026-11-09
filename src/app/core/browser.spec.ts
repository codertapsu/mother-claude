import {
  actNeedsDesktop,
  activityText,
  browserActionView,
  fileSize,
  normalizeSite,
  outcomeBadge,
  sensitiveCategory,
} from './browser';

const SENSITIVE = [
  { domain: 'paypal.com', category: 'payment' },
  { domain: 'accounts.google.com', category: 'identity' },
];

describe('browser helpers', () => {
  it('normalizes sites like the server', () => {
    expect(normalizeSite('*')).toBe('*');
    expect(normalizeSite('https://GitHub.com/issues?q=1')).toBe('https://github.com');
    expect(normalizeSite('github.com')).toBe('https://github.com');
    expect(normalizeSite('http://localhost:3000/')).toBe('http://localhost:3000');
    expect(normalizeSite('https://github.com:443')).toBe('https://github.com');
    expect(normalizeSite('https://*.Example.com')).toBe('https://*.example.com');
    expect(normalizeSite('https://*.com')).toBeNull();
    expect(normalizeSite('ftp://example.com')).toBeNull();
    expect(normalizeSite('not a site at all')).toBeNull();
  });

  it('says which sites only the desktop opens to acting', () => {
    expect(actNeedsDesktop('*', SENSITIVE)).toBe(true);
    expect(actNeedsDesktop('https://*.example.com', SENSITIVE)).toBe(true);
    expect(actNeedsDesktop('https://www.paypal.com', SENSITIVE)).toBe(true);
    expect(actNeedsDesktop('https://github.com', SENSITIVE)).toBe(false);
    expect(sensitiveCategory('https://accounts.google.com', SENSITIVE)).toBe('identity');
    expect(sensitiveCategory('https://notpaypal.com', SENSITIVE)).toBeUndefined();
  });

  it('describes activity without values', () => {
    expect(
      activityText({
        at: 1,
        session: 's',
        tool: 'browser_click',
        action: 'click',
        page: 'https://github.com/acme/app/pulls',
        outcome: 'ok',
      }),
    ).toBe('Clicked · github.com/acme/app/pulls');
    expect(activityText({ at: 1, session: 's', tool: 'lease.open', outcome: 'ok' })).toBe(
      'Started using the browser',
    );
    expect(outcomeBadge('refused')).toEqual({ label: 'refused', cls: 'warn' });
    expect(outcomeBadge('ok').cls).toBe('ok');
    expect(fileSize(2048)).toBe('2 KB');
  });

  it('reads an approval input tolerantly', () => {
    const v = browserActionView({
      action: 'fill',
      url: 'https://x.test/login',
      fields: [{ ref: 'e1', name: 'Username', value: 'marcus' }, 'junk'],
      submit: true,
      reasons: ['It fills a form.', 3],
      future: { a: 1 },
    })!;
    expect(v.action).toBe('fill');
    expect(v.fields[0]).toEqual({ ref: 'e1', name: 'Username', value: 'marcus' });
    expect(v.submit).toBe(true);
    expect(v.reasons).toEqual(['It fills a form.']);
    expect(browserActionView(null)).toBeNull();
    expect(browserActionView({ url: 'x' })).toBeNull();
  });
});
