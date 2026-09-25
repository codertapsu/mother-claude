#!/usr/bin/env node
// Generates the PWA icons in public/ (no dependencies: a tiny rasteriser plus
// a hand-rolled PNG encoder over node:zlib). Re-run after changing the design:
//
//   node scripts/gen-icons.mjs
//
// Output:
//   public/icons/icon-192.png           manifest "any" icon, rounded square
//   public/icons/icon-512.png           manifest "any" icon, rounded square
//   public/icons/icon-maskable-512.png  manifest "maskable" icon, full bleed,
//                                       artwork inside the 80% safe zone
//   public/apple-touch-icon.png         180 px, opaque (iOS rounds it itself)

import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { deflateSync } from 'node:zlib';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', 'public');

// --- PNG encoding ---------------------------------------------------------

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

function encodePng(size, rgba) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  const raw = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0; // filter: none
    rgba.copy(raw, y * (size * 4 + 1) + 1, y * size * 4, (y + 1) * size * 4);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

// --- Artwork --------------------------------------------------------------

const hex = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16) / 255);
const BG_EDGE = hex('#0e0f13');
const BG_CENTER = hex('#241d3d');
const ACCENT = hex('#7c5cc9');
const ACCENT_HI = hex('#b39cf5');
const DOTS = [
  { angle: -40, color: hex('#4ade80') },
  { angle: 80, color: hex('#fbbf24') },
  { angle: 200, color: hex('#60a5fa') },
];

/** Colour (premultiplied-free RGBA, 0..1) of the artwork at unit coords u,v ∈ [0,1]. */
function sample(u, v, { rounded }) {
  const x = u - 0.5;
  const y = v - 0.5;
  // Background: rounded square (any) or full bleed (maskable / apple).
  if (rounded) {
    const r = 0.22;
    const qx = Math.abs(x) - (0.5 - r);
    const qy = Math.abs(y) - (0.5 - r);
    const outside = Math.hypot(Math.max(qx, 0), Math.max(qy, 0)) - r;
    if (outside > 0) return [0, 0, 0, 0];
  }
  const d = Math.hypot(x, y);
  const t = Math.min(1, d / 0.7);
  let c = BG_CENTER.map((a, i) => a + (BG_EDGE[i] - a) * t);

  const over = (col, alpha) => {
    c = c.map((a, i) => a + (col[i] - a) * alpha);
  };

  // Soft halo around the core.
  const halo = Math.max(0, 1 - Math.max(0, d - 0.18) / 0.16);
  over(ACCENT, 0.35 * halo * halo);

  // Orbit ring.
  const ring = Math.abs(d - 0.335);
  if (ring < 0.012) over(ACCENT, 0.55);

  // Core: a lit sphere.
  if (d < 0.19) {
    const lx = x + 0.06;
    const ly = y + 0.07;
    const light = Math.max(0, 1 - Math.hypot(lx, ly) / 0.22);
    over(ACCENT, 1);
    over(ACCENT_HI, 0.75 * light * light);
  }

  // Session dots on the orbit.
  for (const dot of DOTS) {
    const a = (dot.angle * Math.PI) / 180;
    const dx = x - 0.335 * Math.cos(a);
    const dy = y - 0.335 * Math.sin(a);
    const dd = Math.hypot(dx, dy);
    if (dd < 0.05) over(BG_EDGE, 1); // gap in the ring
    if (dd < 0.04) over(dot.color, 1);
  }
  return [...c, 1];
}

function render(size, opts) {
  const ss = 4; // 4x4 supersampling
  const out = Buffer.alloc(size * size * 4);
  for (let py = 0; py < size; py++) {
    for (let px = 0; px < size; px++) {
      let r = 0;
      let g = 0;
      let b = 0;
      let a = 0;
      for (let sy = 0; sy < ss; sy++) {
        for (let sx = 0; sx < ss; sx++) {
          const u = (px + (sx + 0.5) / ss) / size;
          const v = (py + (sy + 0.5) / ss) / size;
          const [cr, cg, cb, ca] = sample(u, v, opts);
          r += cr * ca;
          g += cg * ca;
          b += cb * ca;
          a += ca;
        }
      }
      const n = ss * ss;
      const i = (py * size + px) * 4;
      out[i] = a ? Math.round((r / a) * 255) : 0;
      out[i + 1] = a ? Math.round((g / a) * 255) : 0;
      out[i + 2] = a ? Math.round((b / a) * 255) : 0;
      out[i + 3] = Math.round((a / n) * 255);
    }
  }
  return encodePng(size, out);
}

const targets = [
  ['icons/icon-192.png', 192, { rounded: true }],
  ['icons/icon-512.png', 512, { rounded: true }],
  ['icons/icon-maskable-512.png', 512, { rounded: false }],
  ['apple-touch-icon.png', 180, { rounded: false }],
];

for (const [file, size, opts] of targets) {
  const path = join(root, file);
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, render(size, opts));
  console.log(`wrote ${file} (${size}x${size})`);
}
