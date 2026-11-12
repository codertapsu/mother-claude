import { TestBed } from '@angular/core/testing';

import { FakeBackend, flush, useFakeServer } from '../mock/fake-server.testing';
import { setActiveMock } from '../mock/mock-api';
import { useLanguage } from './i18n';
import { I18nService } from './i18n.service';

interface Internals {
  __TAURI_INTERNALS__?: { invoke: (cmd: string, args: unknown) => unknown };
}

describe('I18nService', () => {
  afterEach(async () => {
    delete (window as unknown as Internals).__TAURI_INTERNALS__;
    setActiveMock(null);
    await useLanguage('en');
  });

  it("tells the desktop's tray the language its window shows", async () => {
    const calls: [string, unknown][] = [];
    (window as unknown as Internals).__TAURI_INTERNALS__ = {
      invoke: (cmd, args) => {
        calls.push([cmd, args]);
        return Promise.resolve();
      },
    };
    // Tauri's module loads on demand: wait for its call.
    const told = async (lang: string) => {
      for (
        let i = 0;
        i < 100 && !calls.some(([, a]) => (a as { lang?: string })?.lang === lang);
        i++
      ) {
        TestBed.tick();
        await new Promise((r) => setTimeout(r, 10));
      }
    };
    useFakeServer(new FakeBackend());
    TestBed.inject(I18nService);
    await told('en');
    expect(calls).toContainEqual(['tray_language', { lang: 'en' }]);
    await useLanguage('vi');
    await told('vi');
    expect(calls).toContainEqual(['tray_language', { lang: 'vi' }]);
  });

  it('tells nobody in a browser', async () => {
    const backend = new FakeBackend();
    useFakeServer(backend);
    TestBed.inject(I18nService);
    TestBed.tick();
    await useLanguage('vi');
    TestBed.tick();
    await flush();
    expect(backend.calls).toEqual([]);
  });
});
