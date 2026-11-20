// An upload by path (docs/BROWSER-PROTOCOL.md, `upload` with `paths`): the
// content script marks the file input with a one-time token, and the service
// worker finds it through the debugger from a world of its own, then hands it
// the files with DOM.setFileInputFiles. Chrome reads them from the disk.

import { CommandError } from './errors.ts';

/** The attribute that marks the file input for the service worker. */
export const UPLOAD_MARK = 'data-mc-upload';

/** A one-time token: 32 hex characters. */
export function uploadToken(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
}

export function isUploadToken(token: unknown): token is string {
  return typeof token === 'string' && /^[0-9a-f]{32}$/.test(token);
}

/**
 * An expression that finds the marked element: in the document, its open
 * shadow roots and its same-origin frames (the content script reaches those
 * too). Evaluated in an isolated world, so the page's own scripts cannot
 * stand in for `querySelector`.
 */
export function findMarkedExpression(token: string): string {
  if (!isUploadToken(token)) throw new CommandError('unsupported', 'a bad upload token');
  const selector = JSON.stringify(`[${UPLOAD_MARK}="${token}"]`);
  return `(() => {
  const sel = ${selector};
  const seen = new Set();
  const walk = (root) => {
    const hit = root.querySelector(sel);
    if (hit) return hit;
    for (const el of root.querySelectorAll('*')) {
      const shadow = el.shadowRoot;
      if (shadow && !seen.has(shadow)) {
        seen.add(shadow);
        const found = walk(shadow);
        if (found) return found;
      }
      if (el.localName === 'iframe' || el.localName === 'frame') {
        let doc = null;
        try {
          doc = el.contentDocument;
        } catch (e) {
          doc = null;
        }
        if (doc && !seen.has(doc)) {
          seen.add(doc);
          const found = walk(doc);
          if (found) return found;
        }
      }
    }
    return null;
  };
  return walk(document);
})()`;
}

/** Removes the mark again (called on the element, with the token). */
export const UNMARK_FUNCTION = `function (token) { if (this.getAttribute(${JSON.stringify(UPLOAD_MARK)}) === token) this.removeAttribute(${JSON.stringify(UPLOAD_MARK)}); }`;

/** How many files the input holds now (called on the element). */
export const FILE_COUNT_FUNCTION = 'function () { return this.files ? this.files.length : 0; }';

/** The paths of an `upload` by path: strings, or `{path, …}` as the desktop sends them. */
export function uploadPaths(raw: unknown): string[] {
  if (!Array.isArray(raw)) return [];
  return raw
    .map((p: unknown) =>
      typeof p === 'string'
        ? p
        : typeof p === 'object' && p !== null && typeof (p as { path?: unknown }).path === 'string'
          ? (p as { path: string }).path
          : '',
    )
    .filter((p) => p.trim() !== '');
}

export function fileAccessOff(): CommandError {
  return new CommandError(
    'file_access_off',
    'Chrome does not let the extension hand files from this computer to a page: turn on “Allow access to file URLs” for Mother Claude (chrome://extensions → Mother Claude → Details)',
  );
}
