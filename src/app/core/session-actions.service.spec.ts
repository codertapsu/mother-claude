import { Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';

import { FolderChoiceService } from './folder-choice.service';
import { FakeBackend, flush, useFakeServer } from './mock/fake-server.testing';
import { setActiveMock } from './mock/mock-api';
import { NoticeService } from './notice.service';
import { SessionActionsService } from './session-actions.service';

@Component({ template: '' })
class BlankComponent {}

const ID = '2d2e9c5e-3a52-4d4c-9d59-5f0b9a1e2c3d';
const RECORDED = '/Users/marcus/development/projects/app';
const HERE = 'D:\\development\\projects\\app';

const elsewhere = {
  status: 409,
  body: {
    error: {
      code: 'folder_elsewhere',
      message: `This conversation was started on another computer, in ${RECORDED}. Choose the folder it works in on this PC.`,
      recorded: RECORDED,
      otherMachine: true,
      suggestions: [HERE, 42],
      machines: [{ id: 'm1', name: 'marcus-mbp', os: 'macos' }, { name: 'no id' }],
    },
  },
};

describe('SessionActionsService: a conversation from another computer', () => {
  let backend: FakeBackend;
  let actions: SessionActionsService;
  let choices: FolderChoiceService;
  let errors: jasmine.Spy;

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([{ path: 'session/:id', component: BlankComponent }])]);
    actions = TestBed.inject(SessionActionsService);
    choices = TestBed.inject(FolderChoiceService);
    errors = spyOn(TestBed.inject(NoticeService), 'error').and.callThrough();
  });

  afterEach(() => setActiveMock(null));

  const continues = () => backend.calls.filter((c) => c === `POST /sessions/${ID}/continue`);

  it('asks for its folder here, then continues there', async () => {
    backend.routes.set(`/sessions/${ID}/continue`, (_m, body) =>
      (body as { folder?: string }).folder ? { status: 201, body: { id: ID } } : elsewhere,
    );
    const done = actions.continueHere({ id: ID, live: false }, { prompt: 'go on' });
    await flush();
    expect(choices.question()).toEqual({
      message: elsewhere.body.error.message,
      recorded: RECORDED,
      otherMachine: true,
      suggestions: [HERE],
      machines: [{ id: 'm1', name: 'marcus-mbp', os: 'macos' }],
    });
    expect(actions.busy().has(ID)).toBeFalse();

    choices.answer(HERE);
    expect(await done).toBe(ID);
    expect(continues().length).toBe(2);
    expect(backend.bodyOf(`POST /sessions/${ID}/continue`)).toEqual({
      prompt: 'go on',
      folder: HERE,
    });
    expect(TestBed.inject(Router).url).toBe(`/session/${ID}`);
    expect(errors).not.toHaveBeenCalled();
  });

  it('closing the question continues nothing and says nothing', async () => {
    backend.routes.set(`/sessions/${ID}/continue`, () => elsewhere);
    const done = actions.continueHere({ id: ID, live: false });
    await flush();
    choices.answer(null);
    expect(await done).toBeNull();
    expect(continues().length).toBe(1);
    expect(errors).not.toHaveBeenCalled();
  });

  it('continuing on another account asks too, and hands back the folder for a retry', async () => {
    const consent = {
      status: 409,
      body: {
        error: {
          code: 'copy_consent_required',
          message: 'Allow copies first.',
          from: 'default',
          to: 'acc_work',
        },
      },
    };
    backend.routes.set(`/sessions/${ID}/continue`, (_m, body) =>
      (body as { folder?: string }).folder ? consent : elsewhere,
    );
    const done = actions.continueOn({ id: ID, live: false }, { id: 'acc_work', name: 'Work' });
    await flush();
    choices.answer(HERE);
    const res = await done;
    expect(res.ok).toBeFalse();
    expect(res.ok ? undefined : res.folder).toBe(HERE);
    expect(backend.bodyOf(`POST /sessions/${ID}/continue`)).toEqual({
      account: 'acc_work',
      folder: HERE,
    });

    // The retry sends it again and asks nothing.
    backend.routes.set(`/sessions/${ID}/continue`, () => ({
      status: 201,
      body: { id: 'f0f0f0f0-3a52-4d4c-9d59-5f0b9a1e2c3d', forked: true },
    }));
    const again = await actions.continueOn(
      { id: ID, live: false },
      { id: 'acc_work', name: 'Work' },
      HERE,
    );
    expect(again.ok).toBeTrue();
    expect(choices.question()).toBeNull();

    // Closed: cancelled, for the sheet to show nothing.
    backend.routes.set(`/sessions/${ID}/continue`, () => elsewhere);
    const closed = actions.continueOn({ id: ID, live: false }, { id: 'acc_work', name: 'Work' });
    await flush();
    choices.answer(null);
    const res2 = await closed;
    expect(res2.ok ? false : res2.cancelled).toBeTrue();
  });
});
