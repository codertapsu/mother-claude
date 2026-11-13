import { Component, signal } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { StreamBubble } from '../../core/live-stream';
import { UiEvent } from '../../core/models';
import { TranscriptStore } from '../../core/transcript-store';
import { TranscriptComponent } from './transcript.component';

/** What the chat reads of a transcript store. */
function fakeStore() {
  return {
    events: signal<UiEvent[]>([]),
    loading: signal(true),
    loadingEarlier: signal(false),
    hasEarlier: signal(false),
    error: signal(''),
    refresh: () => Promise.resolve(),
    loadEarlier: () => Promise.resolve(),
  };
}

@Component({
  imports: [TranscriptComponent],
  template: `
    <div #scroller class="scroller">
      <mc-transcript [scroller]="scroller" [store]="store" [bubbles]="bubbles()" />
    </div>
  `,
  styles: [
    `
      .scroller {
        height: 300px;
        overflow: auto;
      }
    `,
  ],
})
class HostComponent {
  readonly fake = fakeStore();
  readonly store = this.fake as unknown as TranscriptStore;
  readonly bubbles = signal<StreamBubble[]>([]);
}

const prompt = (id: string): UiEvent => ({ id, role: 'user', kind: 'prompt', text: `ask ${id}` });
const reply = (id: string, messageId?: string): UiEvent => ({
  id,
  role: 'assistant',
  kind: 'text',
  text: `**answer** ${id}`,
  messageId,
});
const tool = (id: string): UiEvent => ({
  id,
  role: 'assistant',
  kind: 'tool',
  tool: { id: `tu-${id}`, name: 'Bash', summary: 'ls' },
});

describe('TranscriptComponent: messages that arrive while you watch fade in', () => {
  let fixture: ComponentFixture<HostComponent>;
  let host: HostComponent;

  beforeEach(() => {
    TestBed.configureTestingModule({ providers: [provideRouter([])] });
    fixture = TestBed.createComponent(HostComponent);
    host = fixture.componentInstance;
    fixture.detectChanges();
  });

  const rows = () =>
    Array.from((fixture.nativeElement as HTMLElement).querySelectorAll('mc-transcript-row'));
  const arriving = () =>
    rows()
      .filter((r) => r.classList.contains('arrive'))
      .map((r) => r.getAttribute('data-row'));
  const add = (...events: UiEvent[]) => {
    host.fake.events.update((list) => [...list, ...events]);
    fixture.detectChanges();
  };

  it('leaves the first load still, then fades in each new message', () => {
    host.fake.events.set([prompt('p1'), reply('r1'), prompt('p2'), reply('r2')]);
    host.fake.loading.set(false);
    fixture.detectChanges();
    expect(rows().length).toBe(4);
    expect(arriving()).toEqual([]);

    add(prompt('p3'));
    expect(arriving()).toEqual(['p3']);
    add(reply('r3'));
    expect(arriving()).toEqual(['p3', 'r3']);
    expect(rows().length).toBe(6);
  });

  it('does not fade in a burst, a tool call, or a message its live bubble showed', () => {
    host.fake.loading.set(false);
    fixture.detectChanges();
    add(prompt('a'), reply('b'), prompt('c'), reply('d'));
    expect(arriving(), 'catching up after a reconnect').toEqual([]);
    add(tool('e'));
    expect(arriving(), 'a tool call').toEqual([]);

    host.bubbles.set([
      {
        messageId: 'm1',
        blocks: [{ index: 0, kind: 'text', text: 'streaming…' }],
        updatedAt: Date.now(),
        done: false,
      },
    ]);
    fixture.detectChanges();
    expect((fixture.nativeElement as HTMLElement).querySelector('mc-stream-bubble')).not.toBeNull();
    host.bubbles.set([]);
    add(reply('f', 'm1'));
    expect(arriving(), 'its bubble showed it as it streamed').toEqual([]);

    add(reply('g', 'm2'));
    expect(arriving()).toEqual(['g']);
  });
});
