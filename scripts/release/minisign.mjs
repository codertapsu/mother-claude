/**
 * Verifies a minisign signature — the format Tauri signs update archives with,
 * and the one every installed copy of Mother Claude checks before it installs
 * one.
 *
 *   import { verify } from './minisign.mjs';
 *   const result = verify(archiveBytes, signatureFromLatestJson, pubkeyFromTauriConf);
 *   result.valid;     // true only if every check passed
 *   result.failures;  // why not, in words
 *
 * # Why this exists
 *
 * A release's `latest.json` carries one signature per platform, and the
 * updater refuses an archive whose signature does not verify against the
 * public key compiled into the app. That refusal is silent from here: the user
 * simply never gets the update, and nothing anywhere reports it. Until this
 * file, nothing in the release process checked the signature at all — only that
 * a file with the right *name* had been uploaded.
 *
 * # Matching the verifier that matters
 *
 * The parsing follows `minisign-verify` 0.3.0, the crate tauri-plugin-updater
 * 2.14.0 verifies with: lines are split as Rust's `str::lines()` splits them,
 * base64 must be strict and canonical, the decoded signature must be UTF-8, and
 * (new in 0.3) each Ed25519 signature's scalar `S` must be canonical, below the
 * group order — the one encoding of a valid signature a verifier must not
 * accept twice. The untrusted comment is never checked by that crate, so
 * problems with it are warnings here rather than failures. The point is to
 * agree with the updater, not to be stricter or looser than it.
 *
 * Since 2.13 the updater also reads a `version:` field out of the trusted
 * comment, once the signature has verified, and refuses an update announced as
 * any other version (`signedVersion` below). Signatures from a Tauri CLI that
 * did not record the version carry none, and pass.
 *
 * Format (https://jedisct1.github.io/minisign/):
 *   public key  base64( "Ed" | key id [8] | Ed25519 public key [32] )
 *   signature   untrusted comment: <text>
 *               base64( "Ed" or "ED" | key id [8] | signature [64] )
 *               trusted comment: <text>
 *               base64( global signature [64] )
 *   "Ed": the signature is over the file;  "ED": over BLAKE2b-512(file).
 *   The global signature is over (signature [64] || trusted comment text), so
 *   the trusted comment cannot be altered without detection.
 *
 * Tauri wraps both in one more layer of base64: `tauri.conf.json`'s pubkey is
 * base64 of a minisign public key *file*, and a `.sig` / `latest.json`
 * signature is base64 of a minisign signature *file*. That outer layer is
 * decoded exactly as the updater decodes it (`base64_to_string` in
 * tauri-plugin-updater): the whole string, strictly, with no trimming and no
 * raw minisign text in its place. A signature with a trailing newline passes
 * the minisign CLI and fails in every installed copy, so it fails here too.
 *
 * Uses only node:crypto and node:buffer, so it runs anywhere the release
 * scripts run without an install step.
 */

import { Buffer, isUtf8 } from 'node:buffer';
import { createHash, createPublicKey, verify as ed25519Verify } from 'node:crypto';

/** The order of the Ed25519 group: a canonical signature's `S` is below it. */
const GROUP_ORDER = (1n << 252n) + 27742317777372353535851937790883648493n;

const ALG_RAW = Buffer.from('Ed', 'latin1'); // signature over the raw file
const ALG_PREHASHED = Buffer.from('ED', 'latin1'); // signature over BLAKE2b-512(file)
const UNTRUSTED_PREFIX = Buffer.from('untrusted comment: ', 'latin1');
const TRUSTED_PREFIX = Buffer.from('trusted comment: ', 'latin1');
const ED25519_SPKI_PREFIX = Buffer.from('302a300506032b6570032100', 'hex');

/** Something about the input makes it unverifiable, as opposed to invalid. */
export class MalformedError extends Error {}

/** Strict base64: standard alphabet, canonical padding, no whitespace. */
function decodeBase64(text, what) {
  if (text.length === 0) {
    throw new MalformedError(`${what} is empty`);
  }
  if (text.length % 4 !== 0 || !/^[A-Za-z0-9+/]*={0,2}$/.test(text)) {
    throw new MalformedError(`${what} is not valid base64`);
  }
  const bytes = Buffer.from(text, 'base64');
  // Re-encoding catches non-zero trailing bits, which Node decodes leniently
  // and the Rust decoder rejects.
  if (bytes.toString('base64') !== text) {
    throw new MalformedError(`${what} is not canonical base64`);
  }
  return bytes;
}

/**
 * Lines as Rust's `str::lines()` produces them: split on "\n", dropping one
 * "\r" before it, with no empty line after a final "\n". Never re-encodes.
 */
function lines(bytes) {
  const out = [];
  let start = 0;
  for (let i = 0; i < bytes.length; i += 1) {
    if (bytes[i] === 0x0a) {
      const end = i > start && bytes[i - 1] === 0x0d ? i - 1 : i;
      out.push(bytes.subarray(start, end));
      start = i + 1;
    }
  }
  if (start < bytes.length) {
    out.push(bytes.subarray(start));
  }
  return out;
}

function startsWith(bytes, prefix) {
  return bytes.length >= prefix.length && bytes.subarray(0, prefix.length).equals(prefix);
}

/** A key id as minisign prints it: the little-endian u64, upper-case hex. */
export function keyIdHex(id) {
  return Buffer.from(id).reverse().toString('hex').toUpperCase();
}

/**
 * The updater's `base64_to_string`: the exact string, strictly decoded, and the
 * result must be UTF-8.
 */
function decodeOuter(text, what) {
  const bytes = decodeBase64(text, what);
  if (!isUtf8(bytes)) {
    throw new MalformedError(`${what} does not decode to UTF-8, which the updater rejects`);
  }
  return bytes;
}

/**
 * Parses a public key in the form `tauri.conf.json` stores it: base64 of a
 * minisign public key file.
 */
export function parsePublicKey(text) {
  const warnings = [];
  const keyLines = lines(decodeOuter(text, 'the public key'));
  if (keyLines.length < 2) {
    throw new MalformedError('the public key file has no key line after its comment');
  }
  if (!startsWith(keyLines[0], UNTRUSTED_PREFIX)) {
    warnings.push('the public key comment does not start with "untrusted comment: "');
  }
  const keyBytes = decodeBase64(keyLines[1].toString('latin1'), 'the public key line');

  if (keyBytes.length !== 42) {
    throw new MalformedError(`the public key is ${String(keyBytes.length)} bytes, not 42`);
  }
  if (!keyBytes.subarray(0, 2).equals(ALG_RAW)) {
    throw new MalformedError('the public key is not an Ed25519 minisign key');
  }

  const keyId = keyBytes.subarray(2, 10);
  let key;
  try {
    key = createPublicKey({
      key: Buffer.concat([ED25519_SPKI_PREFIX, keyBytes.subarray(10, 42)]),
      format: 'der',
      type: 'spki',
    });
  } catch (error) {
    throw new MalformedError(`the public key is not a usable Ed25519 key (${error.message})`);
  }
  return { keyId, key, warnings };
}

/**
 * Parses a signature in the form a `latest.json` entry and a Tauri `.sig` file
 * both use: base64 of a minisign signature file.
 */
export function parseSignature(text) {
  const warnings = [];
  const sigLines = lines(decodeOuter(text, 'the signature'));
  if (sigLines.length < 4) {
    throw new MalformedError(`the signature has ${String(sigLines.length)} line(s), not 4`);
  }
  const [untrusted, sigLine, trustedLine, globalLine] = sigLines;

  if (!startsWith(untrusted, UNTRUSTED_PREFIX)) {
    warnings.push(
      'the signature does not start with "untrusted comment: " (the updater does not check it)',
    );
  }
  const bin = decodeBase64(sigLine.toString('latin1'), 'the signature line');
  if (bin.length !== 74) {
    throw new MalformedError(`the signature line is ${String(bin.length)} bytes, not 74`);
  }
  if (!startsWith(trustedLine, TRUSTED_PREFIX)) {
    throw new MalformedError('the third line is not a "trusted comment: "');
  }
  const globalSignature = decodeBase64(globalLine.toString('latin1'), 'the global signature');
  if (globalSignature.length !== 64) {
    throw new MalformedError(
      `the global signature is ${String(globalSignature.length)} bytes, not 64`,
    );
  }

  const alg = bin.subarray(0, 2);
  if (!alg.equals(ALG_PREHASHED) && !alg.equals(ALG_RAW)) {
    throw new MalformedError(
      `the signature algorithm ${JSON.stringify(alg.toString('latin1'))} is not one minisign defines`,
    );
  }

  return {
    prehashed: alg.equals(ALG_PREHASHED),
    keyId: bin.subarray(2, 10),
    signature: bin.subarray(10, 74),
    trustedComment: trustedLine.subarray(TRUSTED_PREFIX.length),
    globalSignature,
    warnings,
  };
}

/**
 * Checks the signature against an already-computed message: the file itself
 * for "Ed", or its BLAKE2b-512 digest for "ED". Exported so a test can pin a
 * real signature from the real release key against a 64-byte digest, rather
 * than committing the ten-megabyte archive it was made over.
 */
export function verifyParsed(message, sig, pk) {
  const failures = [];
  const sameKey = sig.keyId.equals(pk.keyId);
  if (!sameKey) {
    failures.push(
      `signed by key ${keyIdHex(sig.keyId)}, but the public key is ${keyIdHex(pk.keyId)}`,
    );
  }
  // Checked here, not left to node:crypto: minisign-verify 0.3 refuses a
  // non-canonical S outright, and this verifier must say what the updater
  // will say, whatever OpenSSL happens to accept.
  if (!canonicalScalar(sig.signature)) {
    failures.push('the signature encodes its S value non-canonically, which the updater refuses');
  }
  if (!canonicalScalar(sig.globalSignature)) {
    failures.push(
      'the global signature encodes its S value non-canonically, which the updater refuses',
    );
  }
  const signatureHolds = ed25519Verify(null, message, pk.key, sig.signature);
  if (!signatureHolds) {
    failures.push('the signature does not match this file under this key');
  }
  const globalHolds = ed25519Verify(
    null,
    Buffer.concat([sig.signature, sig.trustedComment]),
    pk.key,
    sig.globalSignature,
  );
  // Blaming the trusted comment is only right when everything else held: under
  // the wrong key the global signature fails too, and "altered" would send
  // whoever reads this looking for tampering instead of for the right key.
  if (!globalHolds) {
    failures.push(
      sameKey && signatureHolds
        ? 'the trusted comment was altered after signing'
        : 'the global signature does not verify either',
    );
  }
  return failures;
}

/** Whether a 64-byte Ed25519 signature's `S` (its last 32 bytes, little-endian) is below the group order. */
function canonicalScalar(signature) {
  const s = BigInt(`0x${Buffer.from(signature.subarray(32, 64)).reverse().toString('hex')}`);
  return s < GROUP_ORDER;
}

/**
 * Verifies `file` against `signature` under `publicKey`.
 *
 * Returns rather than throws for an invalid signature — that is an answer, not
 * an error. Throws `MalformedError` only when the input cannot be read as a
 * signature or key at all.
 */
export function verify(file, signature, publicKey) {
  const pk = parsePublicKey(publicKey);
  const sig = parseSignature(signature);
  const message = sig.prehashed ? createHash('blake2b512').update(file).digest() : file;
  const failures = verifyParsed(message, sig, pk);
  return {
    valid: failures.length === 0,
    failures,
    warnings: [...pk.warnings, ...sig.warnings],
    keyId: keyIdHex(sig.keyId),
    trustedComment: sig.trustedComment.toString('utf8'),
  };
}

/**
 * The version a signature was made for, as tauri-plugin-updater 2.13 reads it
 * (`signed_version`): the value of a `version:` field among the tab-separated
 * `key:value` pairs of the trusted comment, e.g.
 * `timestamp:1700000000\tfile:app.tar.gz\tversion:1.2.3`. `null` when the
 * comment has none, which is what every signature made before the Tauri CLI
 * started recording it looks like.
 *
 * Only meaningful once `verify` has said the signature is valid: until then the
 * trusted comment is whatever an attacker wrote.
 */
export function signedVersion(trustedComment) {
  for (const field of String(trustedComment).split('\t')) {
    if (field.startsWith('version:')) {
      return field.slice('version:'.length);
    }
  }
  return null;
}

/**
 * Whether the updater would accept a signature made for `signed` in an update
 * announced as `announced`: equal once a leading "v" is dropped from each. (The
 * updater compares the two as semver, which for the plain x.y.z versions this
 * project uses is the same thing.)
 */
export function sameVersion(signed, announced) {
  return String(signed).replace(/^v/, '') === String(announced).replace(/^v/, '');
}
