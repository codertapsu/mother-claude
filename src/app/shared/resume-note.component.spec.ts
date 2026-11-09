import { TestBed } from '@angular/core/testing';

import { SessionSummary } from '../core/models';
import { ResumeNoteComponent } from './resume-note.component';

describe('ResumeNoteComponent', () => {
  const render = (over: Partial<SessionSummary>) => {
    const fixture = TestBed.createComponent(ResumeNoteComponent);
    fixture.componentRef.setInput('session', {
      id: 's1',
      live: true,
      owned: false,
      surface: 'vscode',
      status: 'background',
      activity: { kind: 'background', label: 'Background work', detail: 'Release build' },
      ...over,
    } as SessionSummary);
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    return {
      text: (el.textContent ?? '').replace(/\s+/g, ' ').trim(),
      shown: el.classList.contains('has-note'),
      icon: el.querySelector('mc-icon'),
    };
  };

  it('says a VS Code session with background work running is not done', () => {
    const n = render({});
    expect(n.shown).toBe(true);
    expect(n.text).toBe(
      'Claude picks this up when its background work finishes — closing VS Code now stops it.',
    );
  });

  it('counts down to the wakeup that resumes it', () => {
    const n = render({
      surface: 'terminal',
      activity: {
        kind: 'scheduled',
        label: 'Scheduled to resume',
        resumesAt: Date.now() + 12 * 60_000 - 1_000,
      },
    });
    expect(n.text).toBe(
      'Claude resumes this by itself in 12m — closing its terminal now stops it.',
    );
  });

  it('says nothing of a session that is idle, working or gone', () => {
    for (const over of [
      { status: 'idle' as const, activity: { kind: 'idle' as const, label: 'Idle' } },
      { status: 'working' as const },
      { live: false },
    ]) {
      const n = render(over);
      expect(n.shown).toBe(false);
      expect(n.text).toBe('');
    }
  });
});
