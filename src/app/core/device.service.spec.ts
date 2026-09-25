import { TestBed } from '@angular/core/testing';

import { ApiService } from './api.service';
import { AuthService } from './auth.service';
import { ConfigService } from './config.service';
import { DeviceService } from './device.service';
import { FakeBackend, flush, useFakeServer } from './mock/fake-server.testing';
import { setActiveMock } from './mock/mock-api';

interface Reply {
  status: number;
  body: unknown;
}

describe('DeviceService', () => {
  let backend: FakeBackend;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend);
  });

  afterEach(() => setActiveMock(null));

  it('revokes the device on the Mac before forgetting the token', async () => {
    backend.routes.set('/me', () => ({ status: 200, body: {} }));
    await TestBed.inject(DeviceService).forgetThisDevice();
    expect(backend.calls).toContain('DELETE /me');
    expect(TestBed.inject(AuthService).state()).toBe('unpaired');
    expect(TestBed.inject(ConfigService).token()).toBe('');
  });

  it('still forgets locally when the Mac is unreachable', async () => {
    backend.routes.set('/me', () => Promise.reject(new TypeError('Load failed')));
    await TestBed.inject(DeviceService).forgetThisDevice();
    expect(TestBed.inject(AuthService).state()).toBe('unpaired');
  });
});

describe('ApiService 401 handling', () => {
  let backend: FakeBackend;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend);
  });

  afterEach(() => setActiveMock(null));

  it('ignores a 401 for a token that was replaced while in flight', async () => {
    let answer!: (r: Reply) => void;
    backend.routes.set('/me', () => new Promise<Reply>((r) => (answer = r)));
    const call = TestBed.inject(ApiService)
      .getMe()
      .catch(() => undefined);
    await flush();
    TestBed.inject(ConfigService).setToken('new-token');
    answer({ status: 401, body: {} });
    await call;
    expect(TestBed.inject(AuthService).state()).toBe('paired');
    expect(TestBed.inject(ConfigService).token()).toBe('new-token');
  });

  it('unpairs on a 401 for the current token', async () => {
    backend.routes.set('/me', () => ({ status: 401, body: {} }));
    await TestBed.inject(ApiService)
      .getMe()
      .catch(() => undefined);
    expect(TestBed.inject(AuthService).state()).toBe('unpaired');
  });
});
