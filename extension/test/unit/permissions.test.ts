import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { beforeEach, describe, it } from 'node:test';

// What the build defines (extension/build.mjs), before the modules that read it.
(globalThis as Record<string, unknown>)['__MC_VERSION__'] = '0.0.0-test';
const { DEBUGGER_GRANT_KEY, grantedList, hasPermission, setDebuggerGranted } =
  await import('../../src/background/env.ts');

const manifest = JSON.parse(
  readFileSync(join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'manifest.json'), 'utf8'),
) as { permissions: string[]; optional_permissions?: string[] };

/**
 * Chrome's "Permissions that can not be specified as optional"
 * (developer.chrome.com/docs/extensions/reference/api/permissions), and the
 * two older Chromes also refused. Listed as optional, Chrome drops the
 * permission without a word and every request for it fails with "Only
 * permissions specified in the manifest may be requested".
 */
const NEVER_OPTIONAL = [
  'debugger',
  'declarativeNetRequest',
  'devtools',
  'experimental',
  'geolocation',
  'mdns',
  'proxy',
  'tts',
  'ttsEngine',
  'wallpaper',
];

describe('the manifest', () => {
  it('lists no optional permission Chrome refuses to make optional', () => {
    const refused = (manifest.optional_permissions ?? []).filter((p) => NEVER_OPTIONAL.includes(p));
    assert.deepEqual(refused, []);
  });

  it('requires the debugger, which the options page then grants', () => {
    assert.ok(manifest.permissions.includes('debugger'));
  });
});

describe('the debugger grant', () => {
  /** What Chrome holds, and chrome.storage.local. */
  let held: Set<string>;
  let stored: Record<string, unknown>;

  beforeEach(() => {
    held = new Set(['storage', 'tabs', 'debugger', 'webNavigation']);
    stored = {};
    (globalThis as Record<string, unknown>)['chrome'] = {
      permissions: {
        contains: async (p: { permissions?: string[] }) =>
          (p.permissions ?? []).every((name) => held.has(name)),
        getAll: async () => ({ permissions: [...held], origins: ['https://github.com/*'] }),
      },
      storage: {
        local: {
          get: async (key: string) => (key in stored ? { [key]: stored[key] } : {}),
          set: async (items: Record<string, unknown>) => void Object.assign(stored, items),
          remove: async (key: string) => void delete stored[key],
        },
      },
    };
  });

  it('is off while Chrome holds the debugger and nobody pressed Grant', async () => {
    assert.equal(await hasPermission('debugger'), false);
    assert.deepEqual(await grantedList(), ['webNavigation', 'https://github.com/*']);
  });

  it('is on after Grant, and off again after Remove', async () => {
    await setDebuggerGranted(true);
    assert.equal(stored[DEBUGGER_GRANT_KEY], true);
    assert.equal(await hasPermission('debugger'), true);
    assert.deepEqual(await grantedList(), ['debugger', 'webNavigation', 'https://github.com/*']);
    await setDebuggerGranted(false);
    assert.equal(DEBUGGER_GRANT_KEY in stored, false);
    assert.equal(await hasPermission('debugger'), false);
    assert.ok(!(await grantedList()).includes('debugger'));
  });

  it('needs Chrome to hold the debugger as well (Firefox has none)', async () => {
    held.delete('debugger');
    await setDebuggerGranted(true);
    assert.equal(await hasPermission('debugger'), false);
    assert.ok(!(await grantedList()).includes('debugger'));
  });

  it('leaves the other permissions to Chrome alone', async () => {
    assert.equal(await hasPermission('webNavigation'), true);
    assert.equal(await hasPermission('downloads'), false);
    held.add('downloads');
    assert.equal(await hasPermission('downloads'), true);
  });

  it('counts a stored value other than true as not granted', async () => {
    stored[DEBUGGER_GRANT_KEY] = 'yes';
    assert.equal(await hasPermission('debugger'), false);
  });
});
