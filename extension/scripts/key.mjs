#!/usr/bin/env node
// The extension's development id comes from the public key in manifest.json
// ("key"): Chrome takes the SHA-256 of that DER-encoded SubjectPublicKeyInfo,
// keeps the first 16 bytes and writes each of their 32 hex digits as a letter
// a–p (0 → a … f → p). The desktop pins that id (Origin
// chrome-extension://<id>), so the key never changes casually.
//
//   node extension/scripts/key.mjs            print the id of manifest.json's key
//   node extension/scripts/key.mjs generate <private-key.pem>
//                                             make a new key pair, write the
//                                             private key OUTSIDE the repository
//                                             and print the manifest "key" + id
//
// The private key is never committed (*.pem is gitignored) and is not needed to
// load the unpacked build: only packing a .crx with the same id needs it.

import { createHash, generateKeyPairSync } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));

/** The extension id Chrome derives from a base64 DER public key. */
export function extensionIdFromKey(base64Key) {
  const der = Buffer.from(base64Key, 'base64');
  const hex = createHash('sha256').update(der).digest('hex').slice(0, 32);
  return [...hex].map((c) => String.fromCharCode(97 + parseInt(c, 16))).join('');
}

/** The id of the key in extension/manifest.json. */
export function manifestExtensionId() {
  const manifest = JSON.parse(readFileSync(join(here, '..', 'manifest.json'), 'utf8'));
  return extensionIdFromKey(manifest.key);
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const [command, out] = process.argv.slice(2);
  if (command === 'generate') {
    if (!out) {
      console.error('usage: key.mjs generate <private-key.pem>  (a path outside the repository)');
      process.exit(2);
    }
    const { publicKey, privateKey } = generateKeyPairSync('rsa', { modulusLength: 2048 });
    writeFileSync(out, privateKey.export({ type: 'pkcs8', format: 'pem' }), { mode: 0o600 });
    const key = publicKey.export({ type: 'spki', format: 'der' }).toString('base64');
    console.log(JSON.stringify({ key, id: extensionIdFromKey(key) }, null, 2));
  } else {
    console.log(manifestExtensionId());
  }
}
