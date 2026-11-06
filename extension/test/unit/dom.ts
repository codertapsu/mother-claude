// A DOM for unit tests (happy-dom: no layout, so nothing here depends on boxes).
// happy-dom 21 runs no page JavaScript and loads no iframe pages by default
// (srcdoc iframes still load), which is what these fixtures want.

import { Window } from 'happy-dom';

export interface TestPage {
  window: Window;
  document: Document;
  close(): Promise<void>;
}

export function page(html: string, url = 'https://example.com/page'): TestPage {
  const window = new Window({
    url,
    settings: { navigation: { disableMainFrameNavigation: true } },
  });
  const document = window.document as unknown as Document;
  document.write(`<!doctype html><html><head></head><body>${html}</body></html>`);
  return { window, document, close: () => window.happyDOM.close() };
}

/** Wait for happy-dom's srcdoc iframes to load. */
export async function settle(p: TestPage): Promise<void> {
  await p.window.happyDOM.waitUntilComplete();
}
