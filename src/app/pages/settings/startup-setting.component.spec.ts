import { ComponentFixture, TestBed } from '@angular/core/testing';

import { AutostartView } from '../../core/machines';
import { FakeBackend, asDesktop, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { StartupSettingComponent } from './startup-setting.component';

/** *Start Mother Claude when you sign in, in the tray* (plan §5.13). */
describe('StartupSettingComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<StartupSettingComponent>;

  const view = (over: Partial<AutostartView> = {}): AutostartView => ({
    supported: true,
    enabled: false,
    overridden: false,
    startedInTray: false,
    ...over,
  });

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, []);
    asDesktop();
  });

  afterEach(() => {
    fixture?.destroy();
    setActiveMock(null);
  });

  const text = (el: Element | null | undefined) =>
    (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  const settle = async () => {
    for (let i = 0; i < 3; i++) {
      await flush();
      fixture.detectChanges();
    }
  };
  const render = async (v: AutostartView) => {
    let current = v;
    backend.routes.set('/app/autostart', (method, body) => {
      if (method === 'PUT')
        current = { ...current, enabled: (body as { enabled: boolean }).enabled };
      return { status: 200, body: current };
    });
    fixture = TestBed.createComponent(StartupSettingComponent);
    fixture.detectChanges();
    await settle();
    return fixture.nativeElement as HTMLElement;
  };

  it('is off by default, and the switch turns it on', async () => {
    const el = await render(view());
    expect(text(el)).toContain('Start Mother Claude when you sign in, in the tray');
    const box = el.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
    expect(box.checked).toBe(false);
    box.click();
    await settle();
    expect(backend.bodyOf('PUT /app/autostart')).toEqual({ enabled: true });
    expect(el.querySelector<HTMLInputElement>('input[type="checkbox"]')!.checked).toBe(true);
  });

  it('says when the system switched it off, and when this run started in the tray', async () => {
    const el = await render(view({ enabled: true, overridden: true, startedInTray: true }));
    expect(el.querySelector<HTMLInputElement>('input[type="checkbox"]')!.checked).toBe(false);
    expect(text(el)).toContain('Switched off in the system’s startup apps');
    expect(text(el)).toContain('closing the window keeps it running in the tray');
  });

  it('shows nothing where the system has no login item for it', async () => {
    const el = await render(view({ supported: false }));
    expect(el.querySelector('input')).toBeNull();
  });
});
