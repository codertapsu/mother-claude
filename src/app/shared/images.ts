/** Image handling for attachments (no dependencies, canvas only). */

export const MAX_IMAGE_BYTES = 5 * 1024 * 1024;
export const MAX_IMAGES = 10;
export const MAX_EDGE = 2048;
const JPEG_QUALITY = 0.85;
const KEEP_AS_IS = new Set(['image/png', 'image/jpeg', 'image/webp', 'image/gif']);

export interface PreparedImage {
  mediaType: string;
  /** Base64 without the data: prefix. */
  data: string;
  blob: Blob;
  bytes: number;
}

export function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => {
      const s = String(r.result ?? '');
      resolve(s.slice(s.indexOf(',') + 1));
    };
    r.onerror = () => reject(r.error ?? new Error('Could not read the file.'));
    r.readAsDataURL(blob);
  });
}

async function decode(file: Blob): Promise<{ w: number; h: number; draw: CanvasImageSource }> {
  if (typeof createImageBitmap === 'function') {
    try {
      const bmp = await createImageBitmap(file);
      return { w: bmp.width, h: bmp.height, draw: bmp };
    } catch {
      /* fall back to <img> (e.g. HEIC on Safari decodes there) */
    }
  }
  const url = URL.createObjectURL(file);
  try {
    const img = new Image();
    img.decoding = 'async';
    img.src = url;
    await img.decode();
    return { w: img.naturalWidth, h: img.naturalHeight, draw: img };
  } finally {
    URL.revokeObjectURL(url);
  }
}

/**
 * Make a phone photo sendable: at most 2048 px on the long edge, re-encoded
 * as JPEG (q 0.85) when it had to shrink or isn't a type Claude takes.
 * Small png/jpeg/webp/gif files pass through untouched (animated GIFs stay
 * animated). Rejects when the result is still over 5 MB.
 */
export async function prepareImage(file: File): Promise<PreparedImage> {
  const type = file.type || 'application/octet-stream';
  if (
    KEEP_AS_IS.has(type) &&
    file.size <= MAX_IMAGE_BYTES &&
    (type === 'image/gif' || file.size < 1_500_000)
  ) {
    const dims = await decode(file).catch(() => null);
    if (type === 'image/gif' || (dims && Math.max(dims.w, dims.h) <= MAX_EDGE)) {
      return { mediaType: type, data: await blobToBase64(file), blob: file, bytes: file.size };
    }
  }
  const { w, h, draw } = await decode(file);
  const scale = Math.min(1, MAX_EDGE / Math.max(w, h, 1));
  const cw = Math.max(1, Math.round(w * scale));
  const ch = Math.max(1, Math.round(h * scale));
  const canvas = document.createElement('canvas');
  canvas.width = cw;
  canvas.height = ch;
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('This browser cannot resize images.');
  ctx.drawImage(draw, 0, 0, cw, ch);
  const blob = await new Promise<Blob>((resolve, reject) =>
    canvas.toBlob(
      (b) => (b ? resolve(b) : reject(new Error('Could not encode the image.'))),
      'image/jpeg',
      JPEG_QUALITY,
    ),
  );
  if (blob.size > MAX_IMAGE_BYTES)
    throw new Error(`${file.name} is too large even after resizing.`);
  return { mediaType: 'image/jpeg', data: await blobToBase64(blob), blob, bytes: blob.size };
}
