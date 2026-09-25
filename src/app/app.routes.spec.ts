import { Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';

import { pairedGuard } from './app.routes';
import { AuthService } from './core/auth.service';

@Component({ template: '' })
class BlankComponent {}

describe('pairedGuard', () => {
  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [
        provideRouter([
          { path: 'inbox', component: BlankComponent, canActivate: [pairedGuard] },
          { path: 'pair', component: BlankComponent },
        ]),
      ],
    });
  });

  it('sends an unpaired device back to /pair, saying why', async () => {
    const auth = TestBed.inject(AuthService);
    auth.state.set('unpaired');
    auth.reason.set('revoked');
    const harness = await RouterTestingHarness.create();
    await harness.navigateByUrl('/inbox');
    expect(TestBed.inject(Router).url).toBe('/pair?reason=revoked');
  });

  it('lets a paired device through', async () => {
    TestBed.inject(AuthService).state.set('paired');
    const harness = await RouterTestingHarness.create();
    await harness.navigateByUrl('/inbox');
    expect(TestBed.inject(Router).url).toBe('/inbox');
  });
});
