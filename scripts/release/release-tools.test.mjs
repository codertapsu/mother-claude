/**
 * The release tooling's own tests: signature verification, and the manifest
 * check built on it.
 *
 * Release tooling had no tests at all until this file. These exist because the
 * failure they guard against is invisible: an update manifest whose signature
 * does not match its archive does not fail a build or a job. It fails on every
 * user's machine, silently, the day the release is published.
 */

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash, generateKeyPairSync, randomBytes, sign } from 'node:crypto';
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { after, describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';

import { checkManifest, fetchRelease } from './check-manifest.mjs';
import {
  MalformedError,
  parsePublicKey,
  parseSignature,
  sameVersion,
  signedVersion,
  verify,
  verifyParsed,
} from './minisign.mjs';

const ROOT = fileURLToPath(new URL('../..', import.meta.url));

/**
 * A real signature, made by the real release key, over a real archive.
 *
 * Mother.Claude.app.tar.gz as published in v0.3.0 (darwin-aarch64 in its
 * latest.json), signed with "ED" — so what was signed is the archive's
 * BLAKE2b-512 digest, and the digest is all this test needs. Everything here is
 * public: the key is in tauri.conf.json, the signature in latest.json on a
 * public release.
 *
 * It is the one test that proves this verifier agrees with the key every
 * installed copy trusts, rather than only with itself — and the key is read
 * from src-tauri/tauri.conf.json, so it is the one compiled into the app.
 */
const REAL = {
  publicKey: JSON.parse(readFileSync(join(ROOT, 'src-tauri', 'tauri.conf.json'), 'utf8')).plugins
    .updater.pubkey,
  keyId: '43FDB9942AC8CABC',
  signature:
    'dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTOHlzZ3FsTG45UTdRUjBBT2JVTzFodlh0aUt5LzMrVXdrQm13bkZTL1J4WHJBWDB1MlF2VVlTM2k0RjJacmJ0d0Z6VzZRSjA3NHdwNjRMdkxFTlJDdkZFck5BVDk4b2dBPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkwMDU4Nzc4CWZpbGU6TW90aGVyIENsYXVkZS5hcHAudGFyLmd6Cm5RMkVCbzhLNlc2ZmZCaUZmcDVFU2VoazU1U2pDay9UYXVySE1TaXg5YzQ4NGZmZDA4RW0vWXRuM1FNN2xyYWlXQUVYSnVDZzFQaEMvcXgyaVZoNENnPT0K',
  digest:
    '38c1e5f78966ec82fdb65c3e28d5dca1fd51c6d993fb655e91d119b570224794e05824bce6f5aa7b60796eb75494a12797742e7d9b244de2b0b801a1e9640e4d',
};

/**
 * A minisign signer over a freshly generated key, producing exactly the forms
 * Tauri does: base64 of a public key file, base64 of a signature file.
 */
function signer({ prehashed = true } = {}) {
  const { publicKey, privateKey } = generateKeyPairSync('ed25519');
  const raw = publicKey.export({ format: 'der', type: 'spki' }).subarray(-32);
  const keyId = randomBytes(8);
  const id = Buffer.from(keyId).reverse().toString('hex').toUpperCase();

  const pubFile = `untrusted comment: minisign public key: ${id}\n${Buffer.concat([Buffer.from('Ed'), keyId, raw]).toString('base64')}\n`;

  return {
    publicKey: Buffer.from(pubFile).toString('base64'),
    sign(file, trusted = 'timestamp:1790000000\tfile:test.tar.gz') {
      const message = prehashed ? createHash('blake2b512').update(file).digest() : file;
      const signature = sign(null, message, privateKey);
      const global = sign(null, Buffer.concat([signature, Buffer.from(trusted)]), privateKey);
      const alg = Buffer.from(prehashed ? 'ED' : 'Ed');
      const sigFile =
        'untrusted comment: signature from tauri secret key\n' +
        `${Buffer.concat([alg, keyId, signature]).toString('base64')}\n` +
        `trusted comment: ${trusted}\n` +
        `${global.toString('base64')}\n`;
      return Buffer.from(sigFile).toString('base64');
    },
  };
}

/** Re-encodes a Tauri-form signature after editing its decoded text. */
function tamperSignature(signature, edit) {
  return Buffer.from(edit(Buffer.from(signature, 'base64').toString('utf8'))).toString('base64');
}

describe('minisign verification', () => {
  it('accepts the real release key’s real signature', () => {
    const pk = parsePublicKey(REAL.publicKey);
    const sig = parseSignature(REAL.signature);

    assert.equal(sig.prehashed, true, 'Tauri signs prehashed ("ED"); the vector says otherwise');
    assert.deepEqual(verifyParsed(Buffer.from(REAL.digest, 'hex'), sig, pk), []);
  });

  // A different key in tauri.conf.json would mean every installed copy refuses
  // every future update. Pin the id the published releases were signed with.
  it('keeps the key every published release was signed with', () => {
    const sig = parseSignature(REAL.signature);
    const pk = parsePublicKey(REAL.publicKey);

    assert.equal(Buffer.from(pk.keyId).reverse().toString('hex').toUpperCase(), REAL.keyId);
    assert.ok(sig.keyId.equals(pk.keyId));
  });

  it('rejects the real signature over any other archive', () => {
    const pk = parsePublicKey(REAL.publicKey);
    const sig = parseSignature(REAL.signature);
    const other = Buffer.from(REAL.digest, 'hex');
    other[0] ^= 0x01;

    assert.deepEqual(verifyParsed(other, sig, pk), [
      'the signature does not match this file under this key',
    ]);
  });

  for (const prehashed of [true, false]) {
    it(`round-trips a ${prehashed ? 'prehashed ("ED")' : 'raw ("Ed")'} signature`, () => {
      const key = signer({ prehashed });
      const file = randomBytes(4096);

      const result = verify(file, key.sign(file), key.publicKey);

      assert.equal(result.valid, true, result.failures.join('; '));
    });
  }

  it('rejects a file with one bit changed', () => {
    const key = signer();
    const file = randomBytes(4096);
    const signature = key.sign(file);
    file[2048] ^= 0x01;

    assert.equal(verify(file, signature, key.publicKey).valid, false);
  });

  // The trusted comment is covered by the global signature; nothing else is.
  // Editing it without the key must be caught.
  it('rejects an altered trusted comment', () => {
    const key = signer();
    const file = randomBytes(1024);
    const signature = tamperSignature(key.sign(file), (text) =>
      text.replace('file:test.tar.gz', 'file:evil.tar.gz'),
    );

    const result = verify(file, signature, key.publicKey);

    assert.equal(result.valid, false);
    assert.ok(
      result.failures.some((f) => f.includes('trusted comment')),
      result.failures.join('; '),
    );
  });

  /**
   * minisign-verify 0.3 (tauri-plugin-updater 2.14) refuses a signature whose
   * S is encoded above the group order — the same scalar, written once more
   * — even where the curve arithmetic would accept it. So does this.
   */
  it('refuses a non-canonical S, as minisign-verify 0.3 does', () => {
    const key = signer();
    const file = randomBytes(512);
    const L = (1n << 252n) + 27742317777372353535851937790883648493n;
    const lines = Buffer.from(key.sign(file), 'base64').toString('utf8').split('\n');
    const bin = Buffer.from(lines[1], 'base64');
    // S is the last 32 bytes of the 64-byte signature, little-endian.
    const s = BigInt(`0x${Buffer.from(bin.subarray(42, 74)).reverse().toString('hex')}`);
    const above = Buffer.from((s + L).toString(16).padStart(64, '0'), 'hex').reverse();
    above.copy(bin, 42);
    lines[1] = bin.toString('base64');
    const tampered = Buffer.from(lines.join('\n')).toString('base64');

    const result = verify(file, tampered, key.publicKey);

    assert.equal(result.valid, false);
    assert.ok(
      result.failures.some((f) => f.includes('non-canonically')),
      result.failures.join('; '),
    );
    // The untouched original still passes: the check is on the encoding only.
    assert.equal(verify(file, key.sign(file), key.publicKey).valid, true);
  });

  it('rejects a signature made by a different key', () => {
    const ours = signer();
    const theirs = signer();
    const file = randomBytes(1024);

    const result = verify(file, theirs.sign(file), ours.publicKey);

    assert.equal(result.valid, false);
    assert.ok(result.failures.some((f) => f.includes('signed by key')));
    // Nothing was tampered with; the key is wrong. Saying otherwise sends the
    // reader after the wrong problem.
    assert.ok(!result.failures.some((f) => f.includes('altered')), result.failures.join('; '));
  });

  /**
   * The updater base64-decodes the manifest's string exactly as it stands, and
   * a newline is an invalid byte to that decoder. `Get-Content -Raw` keeps a
   * file's trailing newline where `$(cat …)` drops it, so this is one shell
   * idiom away from a Windows update every installed copy refuses — while the
   * minisign CLI, which reads files, would call the same signature good.
   */
  it('refuses a signature with a trailing newline, as the updater does', () => {
    const key = signer();
    const file = randomBytes(256);

    assert.throws(() => verify(file, `${key.sign(file)}\n`, key.publicKey), MalformedError);
  });

  // The updater decodes base64 before it parses anything, so raw minisign
  // text in a manifest is not a signature it can read.
  it('refuses raw minisign text where the updater expects base64', () => {
    const key = signer();
    const file = randomBytes(256);
    const raw = Buffer.from(key.sign(file), 'base64').toString('utf8');

    assert.throws(() => verify(file, raw, key.publicKey), MalformedError);
  });

  /**
   * Node's decoder accepts base64 whose final character carries non-zero
   * padding bits; the Rust decoder the updater uses rejects it. A verifier
   * more lenient than the updater is worse than none: it would say "valid" for
   * a signature every installed copy refuses.
   */
  it('refuses non-canonical base64, as the updater does', () => {
    const key = signer();
    const file = randomBytes(256);
    const canonical = key.sign(file);
    // "...=" ends a group carrying 2 meaningful bits in its last character;
    // nudging that character by one keeps the bytes Node decodes and breaks
    // canonical form.
    const lastData = canonical.replace(/=+$/, '').slice(-1);
    const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    const nudged = alphabet[alphabet.indexOf(lastData) + 1];
    const noncanonical = canonical.replace(/[^=](=*)$/, `${nudged}$1`);
    assert.deepEqual(
      Buffer.from(noncanonical, 'base64'),
      Buffer.from(canonical, 'base64'),
      'the fixture should decode to the same bytes under Node',
    );

    assert.throws(() => verify(file, noncanonical, key.publicKey), MalformedError);
  });

  // Not "invalid" — unreadable. The difference matters to the caller: one is
  // a bad release, the other is a bad input to this tool.
  it('refuses malformed input rather than calling it invalid', () => {
    const key = signer();
    for (const bad of ['', 'not base64!', Buffer.from('two\nlines').toString('base64')]) {
      assert.throws(
        () => verify(Buffer.from('x'), bad, key.publicKey),
        MalformedError,
        JSON.stringify(bad),
      );
    }
  });

  // tauri-plugin-updater 2.13's `signed_version`, field for field.
  it('reads the version a signature was made for, as the updater does', () => {
    assert.equal(signedVersion('timestamp:1790000000\tfile:a.tar.gz\tversion:0.4.0'), '0.4.0');
    assert.equal(signedVersion('version:v0.4.0\ttimestamp:1'), 'v0.4.0');
    assert.equal(signedVersion('timestamp:1790058778\tfile:Mother Claude.app.tar.gz'), null);
    assert.equal(sameVersion('v0.4.0', '0.4.0'), true);
    assert.equal(sameVersion('0.3.0', '0.4.0'), false);
  });
});

describe('the manifest check', () => {
  const REPO = 'codertapsu/mother-claude';
  const TAG = 'v0.4.0';
  const url = (name) => `https://github.com/${REPO}/releases/download/${TAG}/${name}`;
  const MAC = 'Mother.Claude.app.tar.gz';
  const EXE = 'Mother.Claude_0.4.0_x64-setup.exe';
  const MSI = 'Mother.Claude_0.4.0_x64_en-US.msi';
  const REQUIRED = [
    'darwin-aarch64',
    'windows-x86_64',
    'windows-x86_64-msi',
    'windows-x86_64-nsis',
  ];

  /** A complete, correct two-leg release, with a way to break it. */
  function release() {
    const key = signer();
    const mac = randomBytes(2048);
    const exe = randomBytes(2048);
    const msi = randomBytes(2048);
    const macSig = key.sign(mac);
    const exeSig = key.sign(exe);
    const msiSig = key.sign(msi);
    const assets = new Map([
      [MAC, mac],
      [`${MAC}.sig`, Buffer.from(macSig)],
      [EXE, exe],
      [`${EXE}.sig`, Buffer.from(exeSig)],
      [MSI, msi],
      [`${MSI}.sig`, Buffer.from(msiSig)],
    ]);
    const nsis = { signature: exeSig, url: url(EXE) };
    const manifest = {
      version: '0.4.0',
      notes: "What's new in 0.4.0: https://github.com/codertapsu/mother-claude/releases/tag/v0.4.0",
      pub_date: '2026-10-13T06:00:00Z',
      platforms: {
        'darwin-aarch64': { signature: macSig, url: url(MAC) },
        'windows-x86_64': nsis,
        'windows-x86_64-msi': { signature: msiSig, url: url(MSI) },
        'windows-x86_64-nsis': nsis,
      },
    };
    return {
      key,
      manifest,
      assets,
      check(overrides = {}) {
        return checkManifest({
          manifest,
          tag: TAG,
          repo: REPO,
          publicKey: key.publicKey,
          require: REQUIRED,
          read: (name) => assets.get(name) ?? null,
          ...overrides,
        });
      },
    };
  }

  it('passes a complete release in which everything verifies', () => {
    const result = release().check();

    assert.equal(result.ok, true, result.problems.join('\n'));
    assert.equal(result.rows.filter((row) => row.valid).length, 4);
  });

  /**
   * The case this was written for. A rebuild uploaded the new archive and was
   * interrupted before it uploaded the new manifest, so latest.json still
   * holds the signature of the archive it replaced. Every asset name is
   * present, and the old check called it complete.
   */
  it('fails a rebuilt archive left beside its predecessor’s signature', () => {
    const r = release();
    r.assets.set(MAC, randomBytes(2048)); // the rebuild

    const result = r.check();

    assert.equal(result.ok, false);
    assert.equal(
      result.rows.find((row) => row.platform === 'darwin-aarch64')?.valid,
      false,
      'a stale signature was accepted',
    );
    assert.ok(
      result.rows.filter((row) => row.platform.startsWith('windows')).every((row) => row.valid),
      'Windows was dragged down too',
    );
  });

  // The MSI entry is checked on its own: an install made from the .msi looks up
  // windows-x86_64-msi first and never reads the NSIS entries.
  it('fails an MSI rebuilt without its manifest entry, and only that entry', () => {
    const r = release();
    r.assets.set(MSI, randomBytes(2048));

    const result = r.check();

    assert.equal(result.ok, false);
    assert.deepEqual(
      result.rows.filter((row) => !row.valid).map((row) => row.platform),
      ['windows-x86_64-msi'],
    );
  });

  /**
   * tauri-plugin-updater 2.13 reads the version out of a verified signature's
   * trusted comment and refuses an update announced as anything else — so an
   * archive left over from another version fails in every copy running 0.4.0
   * or later, even though the signature itself is good.
   */
  it('fails a signature made for another version', () => {
    const r = release();
    const old = randomBytes(2048);
    r.assets.set(MAC, old);
    r.manifest.platforms['darwin-aarch64'].signature = r.key.sign(
      old,
      'timestamp:1790000000\tfile:Mother Claude.app.tar.gz\tversion:0.3.0',
    );
    r.assets.delete(`${MAC}.sig`);

    const result = r.check();

    assert.equal(result.ok, false);
    assert.ok(
      result.problems.some((p) => p.includes('signed for version 0.3.0')),
      result.problems.join('\n'),
    );
  });

  it('accepts a signature made for this version, with or without a "v"', () => {
    for (const recorded of ['0.4.0', 'v0.4.0']) {
      const r = release();
      const archive = randomBytes(2048);
      r.assets.set(MAC, archive);
      r.assets.delete(`${MAC}.sig`);
      r.manifest.platforms['darwin-aarch64'].signature = r.key.sign(
        archive,
        `timestamp:1790000000\tfile:Mother Claude.app.tar.gz\tversion:${recorded}`,
      );

      assert.equal(r.check().ok, true, recorded);
    }
  });

  it('fails a .sig asset that disagrees with the manifest', () => {
    const r = release();
    r.assets.set(`${MAC}.sig`, Buffer.from(r.key.sign(randomBytes(64))));

    const result = r.check();

    assert.equal(result.ok, false);
    assert.ok(
      result.problems.some((p) => p.includes('.sig differs')),
      result.problems.join('\n'),
    );
  });

  // The variant where the Windows leg recreated latest.json from nothing.
  it('fails when a required platform is missing', () => {
    const r = release();
    delete r.manifest.platforms['windows-x86_64-msi'];

    const result = r.check();

    assert.equal(result.ok, false);
    assert.ok(
      result.problems.some((p) => p.startsWith('windows-x86_64-msi is not in the manifest')),
    );
  });

  it('fails a url that names an asset the release does not have', () => {
    const r = release();
    r.assets.delete(EXE);

    assert.ok(r.check().problems.some((p) => p.includes('is not on the release')));
  });

  it('fails a url into another repository or tag', () => {
    for (const bad of [
      `https://github.com/someone/mother-claude/releases/download/v0.4.0/${MAC}`,
      `https://github.com/codertapsu/mother-claude/releases/download/v0.3.0/${MAC}`,
    ]) {
      const r = release();
      r.manifest.platforms['darwin-aarch64'] = {
        ...r.manifest.platforms['darwin-aarch64'],
        url: bad,
      };
      assert.ok(
        r.check().problems.some((p) => p.includes('is not an asset of')),
        bad,
      );
    }
  });

  it('fails a manifest whose version is not the tag', () => {
    const r = release();
    r.manifest.version = '0.3.0';

    assert.ok(r.check().problems.some((p) => p.startsWith('version is "0.3.0"')));
  });

  /**
   * What `(Get-Date).ToString('yyyy-MM-ddTHH:mm:ssZ')` produces on a Windows
   * machine set to Finnish. The updater rejects the whole manifest over it —
   * every platform, not just Windows.
   */
  it('fails a pub_date that is not RFC 3339', () => {
    for (const bad of [
      '2026-10-01T09.15.42Z', // a '.' time separator, from the culture
      '2569-10-01T09:15:42Z_th',
      '01/10/2026 09:15:42',
      // Well-formed and impossible: the updater's parser range-checks, so this does.
      '2026-02-30T09:15:42Z',
      '2025-02-29T00:00:00Z',
      '2026-13-01T00:00:00Z',
      '2026-10-01T24:00:00Z',
      '2026-10-01T09:60:00Z',
      '2026-10-01T09:15:42+24:00',
    ]) {
      const r = release();
      r.manifest.pub_date = bad;
      assert.ok(
        r.check().problems.some((p) => p.startsWith('pub_date')),
        bad,
      );
    }
  });

  // What the `time` crate accepts, this accepts — including the forms RFC 3339
  // allows that nothing here writes. Stricter than the updater is a false alarm
  // on publish day; looser is a broken release.
  it('accepts every pub_date the updater accepts', () => {
    for (const good of [
      '2026-10-01T09:15:42Z',
      '2026-10-01T09:15:42.123Z',
      '2026-10-01T09:15:42+07:00',
      '2026-10-01t09:15:42z',
      '2026-10-01 09:15:42Z',
      '2024-02-29T00:00:00Z',
      null, // `Option<String>`: absent and null are both fine
    ]) {
      const r = release();
      r.manifest.pub_date = good;
      assert.equal(r.check().ok, true, String(good));
    }
  });

  // Half a release is a normal state between the two legs. It must say which
  // half is there, not only that it is incomplete.
  it('reports each platform separately when only one leg has run', () => {
    const r = release();
    for (const platform of ['windows-x86_64', 'windows-x86_64-msi', 'windows-x86_64-nsis']) {
      delete r.manifest.platforms[platform];
    }

    const result = r.check();

    assert.equal(result.ok, false);
    assert.equal(result.rows.length, 1);
    assert.ok(
      result.rows.every((row) => row.valid),
      'the finished leg was reported as broken',
    );
  });
});

describe('fetching a release to check', () => {
  const REPO = 'codertapsu/mother-claude';
  const TAG = 'v0.4.0';

  /** A stand-in for the GitHub API that records every request. */
  function api(files) {
    const requests = [];
    const ids = [...files.keys()].map((name, i) => ({ name, id: 1000 + i }));
    const fetchImpl = async (url, init) => {
      requests.push({ url, accept: init.headers.Accept, auth: init.headers.Authorization });
      if (url.endsWith('/assets?per_page=100')) {
        return new Response(JSON.stringify(ids), { status: 200 });
      }
      const asset = ids.find((a) => url.endsWith(`/releases/assets/${String(a.id)}`));
      return asset
        ? new Response(files.get(asset.name), { status: 200 })
        : new Response('', { status: 404 });
    };
    return { fetchImpl, requests };
  }

  const manifest = {
    version: '0.4.0',
    platforms: {
      'darwin-aarch64': {
        signature: 'x',
        url: `https://github.com/${REPO}/releases/download/${TAG}/Mother.Claude.app.tar.gz`,
      },
    },
  };

  it('downloads the manifest, what it names, and that asset’s .sig — through the API', async () => {
    const files = new Map([
      ['latest.json', Buffer.from(JSON.stringify(manifest))],
      ['Mother.Claude.app.tar.gz', Buffer.from('archive')],
      ['Mother.Claude.app.tar.gz.sig', Buffer.from('sig')],
      ['Mother.Claude_0.4.0_aarch64.dmg', Buffer.from('a big file nobody needs here')],
    ]);
    const { fetchImpl, requests } = api(files);

    const result = await fetchRelease({
      repo: REPO,
      tag: TAG,
      releaseId: 7,
      token: 't0k',
      fetchImpl,
    });

    assert.deepEqual(result.manifest, manifest);
    assert.deepEqual([...result.assets.keys()].sort(), [
      'Mother.Claude.app.tar.gz',
      'Mother.Claude.app.tar.gz.sig',
    ]);
    assert.equal(result.assets.get('Mother.Claude.app.tar.gz').toString(), 'archive');
    // A draft's assets have no public URL: every download must be the API's,
    // asked for as octet-stream, with the token.
    const downloads = requests.filter((r) => r.url.includes('/releases/assets/'));
    assert.equal(downloads.length, 3, 'the .dmg was downloaded, or something was missed');
    assert.ok(
      downloads.every(
        (r) =>
          r.url.startsWith('https://api.github.com/repos/') &&
          r.accept === 'application/octet-stream',
      ),
    );
    assert.ok(requests.every((r) => r.auth === 'Bearer t0k'));
  });

  it('says there is no manifest rather than failing, between the two legs', async () => {
    const { fetchImpl } = api(new Map([['Mother.Claude_0.4.0_aarch64.dmg', Buffer.from('dmg')]]));

    const result = await fetchRelease({
      repo: REPO,
      tag: TAG,
      releaseId: 7,
      token: 't',
      fetchImpl,
    });

    assert.equal(result.manifest, null);
  });

  // A byte-order mark is enough: serde_json does not skip one, so every
  // installed copy would fail to read the file.
  it('reports a latest.json the updater could not read, rather than crashing', async () => {
    for (const bytes of [
      Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), Buffer.from(JSON.stringify(manifest))]),
      Buffer.from('{"version": "0.4.0", "platfo'),
    ]) {
      const { fetchImpl } = api(new Map([['latest.json', bytes]]));

      const result = await fetchRelease({
        repo: REPO,
        tag: TAG,
        releaseId: 7,
        token: 't',
        fetchImpl,
      });

      assert.equal(result.manifest, null);
      assert.match(result.unreadable, /not valid JSON/);
    }
  });

  it('fails loudly when the API refuses, naming the status', async () => {
    const fetchImpl = async () => new Response('', { status: 401 });

    await assert.rejects(
      fetchRelease({ repo: REPO, tag: TAG, releaseId: 7, token: 'bad', fetchImpl }),
      /answered 401/,
    );
  });
});

describe('merging the update manifest', () => {
  const SCRIPT = fileURLToPath(new URL('./update-manifest.mjs', import.meta.url));
  const dir = mkdtempSync(join(tmpdir(), 'mother-claude-manifest-'));
  after(() => rmSync(dir, { recursive: true, force: true }));

  const key = signer();
  const exe = randomBytes(4096);
  const exePath = join(dir, 'Mother Claude_0.4.0_x64-setup.exe');
  writeFileSync(exePath, exe);
  const exeSig = key.sign(exe);
  const darwin = {
    signature: 'mac-signature',
    url: 'https://example.invalid/Mother.Claude.app.tar.gz',
  };

  /** Runs the merger the way the Windows leg does: the manifest on stdin. */
  function merge({
    stdin = '',
    signature = exeSig,
    now = '2026-10-13T06:00:00Z',
    archive = exePath,
    platforms = ['windows-x86_64', 'windows-x86_64-nsis'],
  } = {}) {
    const run = spawnSync(
      process.execPath,
      [
        SCRIPT,
        ...['--version', '0.4.0', '--tag', 'v0.4.0', '--now', now],
        ...['--url', 'https://example.invalid/Mother.Claude_0.4.0_x64-setup.exe'],
        ...['--signature', signature, '--archive', archive, '--pubkey', key.publicKey],
        ...platforms.flatMap((platform) => ['--platform', platform]),
      ],
      { input: stdin, encoding: 'utf8' },
    );
    return {
      status: run.status,
      stderr: run.stderr,
      manifest: run.status === 0 ? JSON.parse(run.stdout) : null,
    };
  }

  // The reason this is a merge: the Mac's entry must survive Windows, and the
  // MSI's entry must survive the NSIS step that runs after it.
  it('keeps the other machine’s entries', () => {
    const existing = {
      version: '0.4.0',
      platforms: { 'darwin-aarch64': darwin, 'windows-x86_64-msi': darwin },
    };

    const { status, stderr, manifest } = merge({ stdin: JSON.stringify(existing) });

    assert.equal(status, 0, stderr);
    assert.deepEqual(Object.keys(manifest.platforms), [
      'darwin-aarch64',
      'windows-x86_64',
      'windows-x86_64-msi',
      'windows-x86_64-nsis',
    ]);
    assert.deepEqual(manifest.platforms['darwin-aarch64'], darwin);
    assert.deepEqual(manifest.platforms['windows-x86_64-msi'], darwin);
    assert.equal(manifest.platforms['windows-x86_64'].signature, exeSig);
    assert.equal(manifest.platforms['windows-x86_64-nsis'].signature, exeSig);
  });

  // Shown as text in Settings → Software updates, so it reads as a sentence.
  it('writes notes a person can read, pointing at the release', () => {
    const { manifest } = merge();

    assert.equal(
      manifest.notes,
      "What's new in 0.4.0: https://github.com/codertapsu/mother-claude/releases/tag/v0.4.0",
    );
  });

  /**
   * The shape the old PowerShell leg produced from a latest.json that
   * Invoke-RestMethod could not parse: ConvertTo-Json over the raw text, so one
   * JSON string. It has no version, and the merge used to drop every other
   * platform over it and exit 0.
   */
  it('refuses an existing manifest that is not a JSON object', () => {
    for (const stdin of [JSON.stringify('{\n  "version": "0.4.0", "platfo'), '[]']) {
      const { status, stderr, manifest } = merge({ stdin });

      assert.equal(status, 1, stdin);
      assert.equal(manifest, null);
      assert.match(stderr, /not a JSON object/);
    }
  });

  it('drops entries left from another version', () => {
    const stale = { version: '0.3.0', platforms: { 'darwin-aarch64': darwin } };

    const { manifest } = merge({ stdin: JSON.stringify(stale) });

    assert.deepEqual(Object.keys(manifest.platforms), ['windows-x86_64', 'windows-x86_64-nsis']);
  });

  /**
   * The write-time half of the check-manifest guard: a `.sig` from an earlier
   * build beside a new archive is refused before anything is uploaded.
   */
  it('refuses a signature that does not verify against the archive', () => {
    const { status, stderr, manifest } = merge({ signature: key.sign(randomBytes(4096)) });

    assert.equal(status, 1);
    assert.equal(manifest, null);
    assert.match(stderr, /does not verify/);
  });

  // The write-time half of the version check: an installer from another
  // version's build is refused before anything is uploaded.
  it('refuses an archive signed for another version', () => {
    const { status, stderr, manifest } = merge({
      signature: key.sign(exe, 'timestamp:1790000000\tfile:x-setup.exe\tversion:0.3.0'),
    });

    assert.equal(status, 1);
    assert.equal(manifest, null);
    assert.match(stderr, /signed for version 0\.3\.0, not 0\.4\.0/);
  });

  it('writes the signature without the trailing newline Get-Content -Raw keeps', () => {
    const { status, stderr, manifest } = merge({ signature: `${exeSig}\r\n` });

    assert.equal(status, 0, stderr);
    assert.equal(manifest.platforms['windows-x86_64'].signature, exeSig);
  });

  it('refuses a path where the signature belongs', () => {
    const { status, stderr } = merge({
      signature: 'src-tauri/target/release/bundle/nsis/Mother Claude_0.4.0_x64-setup.exe.sig',
    });

    assert.equal(status, 2);
    assert.match(stderr, /CONTENTS of the \.sig file/);
  });

  // Without the archive there is nothing to verify the signature against, and
  // a merge that cannot verify must not write.
  it('requires the archive the signature is for', () => {
    const { status, stderr } = merge({ archive: '' });

    assert.equal(status, 2);
    assert.match(stderr, /--archive is required/);
  });

  it('refuses a --now that is not RFC 3339', () => {
    const { status, stderr } = merge({ now: '2026-10-13T09.15.42Z' });

    assert.equal(status, 2);
    assert.match(stderr, /not RFC 3339/);
  });
});

describe('setting the version', () => {
  const SCRIPT = fileURLToPath(new URL('./set-version.mjs', import.meta.url));
  const FILES = [
    'package.json',
    'package-lock.json',
    'src-tauri/Cargo.toml',
    'src-tauri/Cargo.lock',
    'src-tauri/tauri.conf.json',
  ];

  /** A copy of the five files beside a copy of the script, so nothing real moves. */
  function sandbox() {
    const dir = mkdtempSync(join(tmpdir(), 'mother-claude-version-'));
    mkdirSync(join(dir, 'scripts', 'release'), { recursive: true });
    mkdirSync(join(dir, 'src-tauri'));
    copyFileSync(SCRIPT, join(dir, 'scripts', 'release', 'set-version.mjs'));
    for (const file of FILES) copyFileSync(join(ROOT, file), join(dir, file));
    const run = (...args) =>
      spawnSync(process.execPath, [join(dir, 'scripts', 'release', 'set-version.mjs'), ...args], {
        encoding: 'utf8',
      });
    return { dir, run };
  }

  it('agrees with itself in this repository', () => {
    const run = spawnSync(process.execPath, [SCRIPT, '--check'], { encoding: 'utf8' });

    assert.equal(run.status, 0, run.stdout + run.stderr);
    assert.match(run.stdout, /All 6 version spots agree/);
  });

  // Both lockfiles included: a bump that left them behind would be rewritten by
  // the first install or build of the release, and the tree would come out dirty.
  it('writes every spot, and changes nothing else', () => {
    const { dir, run } = sandbox();
    after(() => rmSync(dir, { recursive: true, force: true }));

    const write = run('v9.8.7');
    assert.equal(write.status, 0, write.stderr);
    const check = run('--check', '--expect', 'v9.8.7');
    assert.equal(check.status, 0, check.stdout + check.stderr);

    for (const file of FILES) {
      const before = readFileSync(join(ROOT, file), 'utf8').split('\n');
      const now = readFileSync(join(dir, file), 'utf8').split('\n');
      assert.equal(now.length, before.length, file);
      const changed = now.filter((line, i) => line !== before[i]);
      assert.ok(changed.length >= 1, `${file} was not bumped`);
      assert.ok(
        changed.every((line) => line.includes('9.8.7')),
        `${file} changed more than its version: ${changed.join(' | ')}`,
      );
    }
  });

  // What a Windows clone made without .gitattributes holds. The version check
  // used to fail there with "no version where it belongs", naming the wrong
  // problem; it now reads through the endings and keeps them.
  it('reads and writes a checkout with Windows line endings', () => {
    const { dir, run } = sandbox();
    after(() => rmSync(dir, { recursive: true, force: true }));
    for (const file of FILES) {
      const path = join(dir, file);
      writeFileSync(path, readFileSync(path, 'utf8').replace(/\n/g, '\r\n'));
    }

    const check = run('--check');
    assert.equal(check.status, 0, check.stdout + check.stderr);
    const write = run('9.8.7');
    assert.equal(write.status, 0, write.stderr);
    assert.equal(run('--check', '--expect', 'v9.8.7').status, 0);
    for (const file of FILES) {
      const text = readFileSync(join(dir, file), 'utf8');
      assert.ok(!/[^\r]\n/.test(text), `${file} lost its CRLF endings`);
    }
  });

  it('refuses a tree whose spots disagree, and a tag it does not match', () => {
    const { dir, run } = sandbox();
    after(() => rmSync(dir, { recursive: true, force: true }));

    assert.equal(run('--check', '--expect', 'v9.9.9').status, 1);

    const conf = join(dir, 'src-tauri', 'tauri.conf.json');
    writeFileSync(
      conf,
      readFileSync(conf, 'utf8').replace(/"version": "[^"]*"/, '"version": "0.0.1"'),
    );
    const check = run('--check');
    assert.equal(check.status, 1);
    assert.match(check.stderr, /the version spots disagree/);
  });
});
