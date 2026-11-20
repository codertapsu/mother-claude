import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, asDesktop, flush, useFakeServer } from '../core/mock/fake-server.testing';
import { setActiveMock } from '../core/mock/mock-api';
import { PendingItem } from '../core/models';
import { RealtimeService } from '../core/realtime.service';
import { BrowserActionComponent } from './browser-action.component';
import { PendingCardComponent } from './pending-card.component';

const text = (n: Element | null | undefined) => (n?.textContent ?? '').replace(/\s+/g, ' ').trim();

const item = (over: Partial<PendingItem> = {}): PendingItem => ({
  id: 'req-b1',
  sessionId: 's1',
  kind: 'permission',
  source: 'browser',
  toolName: 'browser_fill_form',
  title: 'Fill a form (1 field) on github.com',
  detail: 'It fills a form that has password, card or one-time-code fields.',
  input: {
    action: 'fill',
    browser: 'Chrome on MacBook',
    url: 'https://github.com/login',
    origin: 'https://github.com',
    fields: [{ ref: 'e3', name: 'Username', value: 'marcus' }],
    submit: true,
    reasons: ['It fills a form that has password, card or one-time-code fields.'],
  },
  suggestions: [
    {
      index: 0,
      label:
        'Always allow sessions to fill and submit forms with password or card fields on https://github.com',
      kind: 'browserGrant',
      destination: 'browser',
      standing: true,
      raw: {},
    },
  ],
  dangerous: true,
  dangerousReason: 'It fills a form that has password, card or one-time-code fields.',
  answerable: true,
  createdAt: Date.now(),
  ...over,
});

describe('BrowserActionComponent', () => {
  let backend: FakeBackend;

  beforeEach(() => {
    backend = new FakeBackend();
    backend.routes.set('/sessions/s1/respond', () => ({ status: 200, body: { ok: true } }));
    useFakeServer(backend, [provideRouter([])]);
    // Answers need a live connection.
    (
      TestBed.inject(RealtimeService) as unknown as { connectionS: { set(v: string): void } }
    ).connectionS.set('open');
  });

  afterEach(() => setActiveMock(null));

  it('shows what the action would do', () => {
    const fixture = TestBed.createComponent(BrowserActionComponent);
    fixture.componentRef.setInput('input', {
      action: 'eval',
      url: 'http://localhost:3000/',
      expression: 'document.cookie',
      touchesStorage: true,
    });
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(text(el)).toContain('Run a script in the page');
    expect(text(el.querySelector('.script'))).toBe('document.cookie');
    expect(text(el.querySelector('.warn'))).toContain('cookies, storage or credentials');
  });

  it('says a cookie export goes to a file, and which cookies', () => {
    const fixture = TestBed.createComponent(BrowserActionComponent);
    fixture.componentRef.setInput('input', {
      action: 'cookies',
      url: 'https://www.bilibili.com/',
      values: true,
      export: true,
      names: ['SESSDATA', 'buvid3'],
    });
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(text(el)).toContain('Save cookies to a file for a program');
    expect(text(el)).toContain('SESSDATA, buvid3');
    // Reading the values themselves says so.
    fixture.componentRef.setInput('input', { action: 'cookies', values: true });
    fixture.detectChanges();
    expect(text(el)).toContain('Read cookie values');
  });

  it('shows how large an upload is', () => {
    const fixture = TestBed.createComponent(BrowserActionComponent);
    fixture.componentRef.setInput('input', {
      action: 'upload',
      url: 'https://www.facebook.com/',
      files: [{ name: 'part-1.mp4', size: 734003200 }],
    });
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(text(el)).toContain('Upload files');
    expect(text(el)).toContain('part-1.mp4');
    expect(text(el)).toMatch(/700(\.0)? MB/);
  });

  it('is the body of a browser approval card', async () => {
    const fixture = TestBed.createComponent(PendingCardComponent);
    fixture.componentRef.setInput('item', item());
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelector('mc-browser-action')).not.toBeNull();
    expect(el.querySelector('mc-tool-preview')).toBeNull();
    expect(text(el.querySelector('.meta'))).toContain('in the browser');
    expect(text(el)).toContain('Fill a form');
    expect(text(el)).toContain('https://github.com/login');
    expect(text(el)).toContain('Username: marcus');
    expect(text(el)).toContain('submits the form');
    expect(text(el.querySelector('.reason'))).toContain('password, card or one-time-code');
    // A phone without the grant can only deny.
    const allow = [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) =>
      text(b).includes('Allow once'),
    )!;
    expect(allow.disabled).toBe(true);
  });

  it('on the desktop: allow, or always allow (saved in Mother Claude)', async () => {
    asDesktop();
    const fixture = TestBed.createComponent(PendingCardComponent);
    fixture.componentRef.setInput('item', item());
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    const button = (label: string) =>
      [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).includes(label))!;
    expect(button('Allow once').disabled).toBe(false);
    button('Always').click();
    fixture.detectChanges();
    expect(text(el.querySelector('.panel'))).toContain('Saved in Mother Claude');
    button('Always allow sessions').click();
    await flush();
    expect(backend.bodyOf('POST /sessions/s1/respond')).toEqual({
      requestId: 'req-b1',
      decision: 'allow',
      suggestion: 0,
    });
  });
});
