import { TestBed } from '@angular/core/testing';

import { ApiService } from '../core/api.service';
import { MediaImageComponent } from './media-image.component';

describe('MediaImageComponent', () => {
  it('makes no object URL when it is destroyed before the image arrives', async () => {
    let deliver!: (b: Blob) => void;
    let signal: AbortSignal | undefined;
    TestBed.configureTestingModule({
      providers: [
        {
          provide: ApiService,
          useValue: {
            fetchMedia: (_ref: string, s?: AbortSignal) => {
              signal = s;
              return new Promise<Blob>((r) => (deliver = r));
            },
          },
        },
      ],
    });
    const create = spyOn(URL, 'createObjectURL').and.callThrough();
    const fixture = TestBed.createComponent(MediaImageComponent);
    fixture.componentRef.setInput('ref', '/api/sessions/s1/media/1/u/0');
    document.body.appendChild(fixture.nativeElement);
    fixture.detectChanges();
    // Wait for the observer to start the download.
    for (let i = 0; i < 20 && !deliver; i++) await new Promise((r) => setTimeout(r, 10));
    expect(deliver).toBeDefined();
    fixture.destroy();
    (fixture.nativeElement as HTMLElement).remove();
    expect(signal?.aborted).toBeTrue();
    deliver(new Blob(['x'], { type: 'image/png' }));
    await new Promise((r) => setTimeout(r, 0));
    expect(create).not.toHaveBeenCalled();
  });
});
