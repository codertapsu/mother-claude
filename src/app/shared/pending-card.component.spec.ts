import { WritableSignal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, flush, useFakeServer } from '../core/mock/fake-server.testing';
import { setActiveMock } from '../core/mock/mock-api';
import { PendingItem } from '../core/models';
import { NoticeService } from '../core/notice.service';
import { PendingResolution, PendingService } from '../core/pending.service';
import { PendingCardComponent } from './pending-card.component';

interface Reply {
  status: number;
  body: unknown;
}

const item: PendingItem = {
  id: 'req-1',
  sessionId: 's1',
  kind: 'permission',
  source: 'owned',
  toolName: 'Bash',
  title: 'Allow Bash?',
  input: { command: 'rm -rf build' },
  suggestions: [],
  dangerous: false,
  answerable: true,
  createdAt: Date.now(),
};

/** The server's echo of an answer, as a `pendingResolved` frame records it. */
const echo = (): PendingResolution => ({
  id: item.id,
  session: item.sessionId,
  outcome: 'denied',
  by: { kind: 'device', name: 'Phone' },
  at: Date.now(),
  mine: false,
});

describe('PendingCardComponent', () => {
  let backend: FakeBackend;
  let answer: (r: Reply) => void;

  beforeEach(() => {
    backend = new FakeBackend();
    backend.routes.set('/sessions/s1/respond', () => new Promise<Reply>((r) => (answer = r)));
    useFakeServer(backend, [provideRouter([])]);
  });

  afterEach(() => setActiveMock(null));

  /** Deny with a note, and let the echo land before the HTTP response. */
  async function denyWithEchoFirst() {
    const fixture = TestBed.createComponent(PendingCardComponent);
    fixture.componentRef.setInput('item', item);
    fixture.detectChanges();
    const card = fixture.componentInstance as unknown as {
      feedback: WritableSignal<string>;
      denyWithFeedback(): Promise<void>;
    };
    const info = spyOn(TestBed.inject(NoticeService), 'info').and.callThrough();
    card.feedback.set('use the clean script instead');
    const sent = card.denyWithFeedback();
    await flush();
    const pendings = TestBed.inject(PendingService) as unknown as {
      noteResolved(r: PendingResolution): void;
    };
    pendings.noteResolved(echo());
    fixture.componentRef.setInput('gone', true);
    fixture.detectChanges();
    return { fixture, info, sent };
  }

  it('does not report its own answer as settled elsewhere', async () => {
    const { fixture, info, sent } = await denyWithEchoFirst();
    const text = () => (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(info).not.toHaveBeenCalled();
    expect(text()).not.toContain('Send your note as a message instead');

    answer({ status: 200, body: {} });
    await sent;
    await flush();
    fixture.detectChanges();
    expect(info).not.toHaveBeenCalled();
    expect(text()).toContain('Denied with feedback');
    expect(text()).not.toContain('Send your note as a message instead');
  });

  it('still says so when someone else really answered first', async () => {
    const { fixture, info, sent } = await denyWithEchoFirst();
    answer({ status: 409, body: { error: { code: 'already_decided', message: 'Already' } } });
    await sent;
    await flush();
    fixture.detectChanges();
    expect(info).toHaveBeenCalledWith("Denied on Phone. Your note wasn't sent.");
  });
});
