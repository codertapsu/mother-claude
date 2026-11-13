import { WritableSignal } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { FakeBackend, flush, useFakeServer } from '../../core/mock/fake-server.testing';
import { setActiveMock } from '../../core/mock/mock-api';
import { ControlCaps, SessionDetail } from '../../core/models';
import { NoticeService } from '../../core/notice.service';
import { RealtimeService } from '../../core/realtime.service';
import {
  ComposerComponent,
  pasteMeansText,
  pastedName,
  remainderAfterSend,
  transferFiles,
} from './composer.component';
import { SessionContext } from './session-context';

interface Reply {
  status: number;
  body: unknown;
}

interface ComposerInternals {
  text: WritableSignal<string>;
  palette: WritableSignal<'slash' | 'files' | null>;
  submit(how: 'send'): Promise<void>;
}

describe('remainderAfterSend', () => {
  it('keeps what was typed after the sent text', () => {
    expect(remainderAfterSend('hello', 'hello')).toBe('');
    expect(remainderAfterSend('hello\nand more', 'hello')).toBe('and more');
    expect(remainderAfterSend('rewritten', 'hello')).toBe('rewritten');
  });
});

describe('ComposerComponent', () => {
  let backend: FakeBackend;
  let fixture: ComponentFixture<ComposerComponent>;
  let cmp: ComposerInternals;
  let ta: HTMLTextAreaElement;

  const type = (value: string) => {
    ta.value = value;
    ta.setSelectionRange(value.length, value.length);
    ta.dispatchEvent(new Event('input'));
  };

  beforeEach(() => {
    backend = new FakeBackend();
    useFakeServer(backend, [provideRouter([]), SessionContext]);
    const ctx = TestBed.inject(SessionContext);
    ctx.id.set('s1');
    ctx.detail.set({
      id: 's1',
      live: true,
      status: 'idle',
      pending: [],
      control: { message: 'direct', slashCommands: true, limits: [] } as unknown as ControlCaps,
    } as unknown as SessionDetail);
    // Live: sending is allowed.
    const rt = TestBed.inject(RealtimeService) as unknown as {
      connectionS: WritableSignal<string>;
    };
    rt.connectionS.set('open');
    fixture = TestBed.createComponent(ComposerComponent);
    fixture.detectChanges();
    cmp = fixture.componentInstance as unknown as ComposerInternals;
    ta = (fixture.nativeElement as HTMLElement).querySelector('textarea')!;
  });

  afterEach(() => setActiveMock(null));

  it('keeps text typed while a send is in flight', async () => {
    let answer!: (r: Reply) => void;
    backend.routes.set('/sessions/s1/message', () => new Promise<Reply>((r) => (answer = r)));
    type('first message');
    const sent = cmp.submit('send');
    await flush();
    type('first message\nsecond thought');
    answer({ status: 200, body: { delivery: 'sent', messageId: 'm1' } });
    await sent;
    expect(cmp.text()).toBe('second thought');
  });

  it('ignores a /commands answer that arrives after the text moved on', async () => {
    let answer!: (r: Reply) => void;
    backend.routes.set('/sessions/s1/commands', () => new Promise<Reply>((r) => (answer = r)));
    type('/');
    await flush();
    type('');
    type('hello');
    answer({ status: 200, body: { commands: [{ name: 'clear' }] } });
    await flush();
    expect(cmp.palette()).toBeNull();
  });

  it('sends to a session that resumes by itself as to an idle one', () => {
    const ctx = TestBed.inject(SessionContext);
    const at = (status: string, kind: string) =>
      ctx.detail.set({
        id: 's1',
        live: true,
        status,
        activity: { kind, label: '' },
        pending: [],
        control: { message: 'direct', slashCommands: true, limits: [] } as unknown as ControlCaps,
      } as unknown as SessionDetail);
    // Between turns, its background work or a wakeup resumes it later:
    // not in a turn, so a message goes in now rather than in the queue.
    at('background', 'background');
    expect(ctx.busy()).toBe(false);
    at('background', 'scheduled');
    expect(ctx.busy()).toBe(false);
    at('working', 'tool');
    expect(ctx.busy()).toBe(true);
    at('idle', 'idle');
    expect(ctx.busy()).toBe(false);
  });

  it('sends while Claude works, with Send now for at once', async () => {
    const ctx = TestBed.inject(SessionContext);
    const detail = (queuedMessages: unknown[] = []) =>
      ctx.detail.set({
        id: 's1',
        live: true,
        status: 'working',
        activity: { kind: 'tool', label: 'Running a command' },
        pending: [],
        queuedMessages,
        control: {
          channel: 'owned',
          message: 'direct',
          interrupt: 'immediate',
          slashCommands: true,
          limits: [],
        } as unknown as ControlCaps,
      } as unknown as SessionDetail);
    detail();
    backend.routes.set('/sessions/s1/message', () => ({
      status: 200,
      body: { delivery: 'queued', messageId: 'q1' },
    }));
    backend.routes.set('/sessions/s1/queue/q1/send-now', () => ({
      status: 200,
      body: { ok: true, sendNow: 'stopped' },
    }));
    backend.routes.set('/sessions/s1', () => ({ status: 200, body: { id: 's1' } }));
    type('also check the tests');
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    const text = (e: Element | null) => (e?.textContent ?? '').replace(/\s+/g, ' ').trim();
    // Not a queue: Claude reads it after its current step.
    expect(text(el.querySelector('.send'))).toBe('Send');
    expect(text(el.querySelector('.under .hint'))).toBe(
      'Claude reads it after its current step · Send now for at once',
    );
    const now = [...el.querySelectorAll<HTMLButtonElement>('.under .btn')].find(
      (b) => text(b) === 'Send now',
    );
    expect(now).toBeDefined();
    now!.click();
    await flush();
    expect(backend.calls).toContain('POST /sessions/s1/message');
    expect(backend.calls).toContain('POST /sessions/s1/queue/q1/send-now');

    // A message that waits says when Claude reads it, and offers Send now.
    detail([
      {
        id: 'q2',
        text: 'and the docs',
        createdAt: 1,
        via: 'owned',
        status: 'queued',
        cancellable: true,
      },
    ]);
    fixture.detectChanges();
    const row = el.querySelector('.queued li');
    expect(text(row?.querySelector('.qstate') ?? null)).toBe('after its current step');
    (row?.querySelector('[aria-label="Send now: and the docs"]') as HTMLButtonElement).click();
    await flush();
    expect(backend.calls).toContain('POST /sessions/s1/queue/q2/send-now');
  });

  it('opens the palette when the answer is still current', async () => {
    backend.routes.set('/sessions/s1/commands', () => ({
      status: 200,
      body: { commands: [{ name: 'clear' }] },
    }));
    type('/');
    await flush();
    expect(cmp.palette()).toBe('slash');
  });

  describe('pasting and dropping files', () => {
    /** A 1×1 PNG, as a screenshot comes off the clipboard. */
    const PNG =
      'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';
    const screenshot = () =>
      new File([Uint8Array.from(atob(PNG), (c) => c.charCodeAt(0))], 'image.png', {
        type: 'image/png',
      });
    const carrying = (files: File[], text = '') => {
      const dt = new DataTransfer();
      for (const f of files) dt.items.add(f);
      if (text) dt.setData('text/plain', text);
      return dt;
    };
    const paste = (files: File[], text = '') => {
      const e = new ClipboardEvent('paste', {
        clipboardData: carrying(files, text),
        bubbles: true,
        cancelable: true,
      });
      ta.dispatchEvent(e);
      return e;
    };
    const takes = (attach: 'inline' | 'path' | 'none', live = true) => {
      TestBed.inject(SessionContext).detail.set({
        id: 's1',
        live,
        status: 'idle',
        pending: [],
        control: {
          message: 'direct',
          attach,
          continueOwned: true,
          slashCommands: true,
          limits: [],
        } as unknown as ControlCaps,
      } as unknown as SessionDetail);
      fixture.detectChanges();
    };
    /** Waits for the attachment work (decoding an image takes a few turns
     * of the event loop). */
    const until = async (done: () => boolean) => {
      for (let i = 0; i < 100 && !done(); i++) {
        await flush();
        await new Promise((r) => setTimeout(r, 10));
      }
    };
    const attachments = () =>
      (
        fixture.componentInstance as unknown as { attachments: () => { name: string }[] }
      ).attachments();
    const toasts = () =>
      TestBed.inject(NoticeService)
        .toasts()
        .map((t) => t.text);

    it('attaches a pasted screenshot where the session takes images', async () => {
      takes('inline');
      const e = paste([screenshot()]);
      expect(e.defaultPrevented).toBe(true);
      await until(() => attachments().length > 0);
      expect(attachments().length).toBe(1);
      expect(attachments()[0].name).toMatch(/^pasted-image-\d{8}-\d{6}\.png$/);
      expect(cmp.text()).toBe('');
    });

    it('uploads a pasted file and puts its path where the caret is', async () => {
      takes('path');
      backend.routes.set('/sessions/s1/uploads', () => ({
        status: 200,
        body: { path: '/data/uploads/s1/ab12-notes.txt' },
      }));
      type('see  please');
      ta.focus();
      ta.setSelectionRange(4, 4);
      paste([new File(['hello'], 'notes.txt', { type: 'text/plain' })], 'notes.txt');
      await until(() => cmp.text().includes('@'));
      expect(backend.calls).toContain('POST /sessions/s1/uploads');
      expect(cmp.text()).toBe('see @/data/uploads/s1/ab12-notes.txt please');
    });

    it('leaves a paste that carries text to the box', () => {
      takes('inline');
      // Cells copied from a spreadsheet carry a picture of themselves too.
      const e = paste([screenshot()], 'Q1\t42\nQ2\t57');
      expect(e.defaultPrevented).toBe(false);
      expect(attachments().length).toBe(0);
    });

    it('says why a session that takes no files refuses a paste or a drop', async () => {
      takes('none');
      expect(paste([screenshot()]).defaultPrevented).toBe(true);
      await until(() => toasts().length > 0);
      expect(toasts()).toContain("This session can't take images or files from here.");

      const host = fixture.nativeElement as HTMLElement;
      const over = new DragEvent('dragover', {
        dataTransfer: carrying([screenshot()]),
        bubbles: true,
        cancelable: true,
      });
      host.querySelector('.box')!.dispatchEvent(over);
      fixture.detectChanges();
      expect(host.querySelector('.drop-hint')?.textContent).toContain(
        "This session can't take images or files from here.",
      );
    });

    it('attaches a file dropped on the box', async () => {
      takes('inline');
      const host = fixture.nativeElement as HTMLElement;
      const box = host.querySelector('.box')!;
      const dt = carrying([screenshot()]);
      box.dispatchEvent(
        new DragEvent('dragover', { dataTransfer: dt, bubbles: true, cancelable: true }),
      );
      fixture.detectChanges();
      expect(box.classList).toContain('dropping');
      expect(host.querySelector('.drop-hint')?.textContent).toContain('Drop to attach');
      const drop = new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true });
      box.dispatchEvent(drop);
      expect(drop.defaultPrevented).toBe(true);
      await until(() => attachments().length > 0);
      fixture.detectChanges();
      expect(box.classList).not.toContain('dropping');
      expect(attachments().length).toBe(1);
      // A dropped file keeps its own name.
      expect(attachments()[0].name).toBe('image.png');
    });
  });
});

describe('composer paste helpers', () => {
  const file = (name: string, type = 'image/png') => new File(['x'], name, { type });

  it('names what a clipboard leaves nameless, and keeps a real name', () => {
    const at = new Date(2026, 10, 13, 9, 5, 7);
    expect(pastedName(file('image.png'), at)).toBe('pasted-image-20261113-090507.png');
    expect(pastedName(file('', 'image/jpeg'), at)).toBe('pasted-image-20261113-090507.jpg');
    expect(pastedName(file('blob', 'image/svg+xml'), at)).toBe('pasted-image-20261113-090507.svg');
    expect(pastedName(file('', 'text/plain'), at)).toBe('pasted-file-20261113-090507.txt');
    expect(pastedName(file('Screenshot 2026-11-13.png'), at)).toBe('Screenshot 2026-11-13.png');
  });

  it('tells a file copied in Finder from text that came with a picture', () => {
    const notes = [file('notes.txt', 'text/plain')];
    expect(pasteMeansText('notes.txt', notes)).toBe(false);
    expect(pasteMeansText('/Users/me/notes.txt', notes)).toBe(false);
    expect(pasteMeansText('', notes)).toBe(false);
    expect(pasteMeansText('Q1\t42', [file('image.png')])).toBe(true);
  });

  it('reads the files of a transfer from its items when files is empty', () => {
    const f = file('a.png');
    const fake = {
      files: [] as unknown as FileList,
      items: [
        { kind: 'string', getAsFile: () => null },
        { kind: 'file', getAsFile: () => f },
      ],
    } as unknown as DataTransfer;
    expect(transferFiles(fake)).toEqual([f]);
    expect(transferFiles(null)).toEqual([]);
  });
});
