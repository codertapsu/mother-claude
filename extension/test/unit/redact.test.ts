import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { isSecretField, soundsSecret, words } from '../../src/content/redact.ts';
import { page } from './dom.ts';

describe('secret names', () => {
  it('splits camelCase, snake_case and kebab-case into words', () => {
    assert.equal(words('apiKey_value'), 'api key value');
    assert.equal(words('cc-number'), 'cc number');
    assert.equal(words('OTPCode'), 'otp code');
  });

  it('recognises the protocol’s secret words', () => {
    for (const s of [
      'password',
      'user_password',
      'newPassword',
      'cc-number',
      'cc-csc',
      'one-time-code',
      'ssn',
      'api key',
      'apiKey',
      'access_token',
      'client-secret',
      'cardNumber',
      'security code',
      'pin',
    ]) {
      assert.equal(soundsSecret(s), true, s);
    }
  });

  it('leaves ordinary names alone', () => {
    for (const s of [
      'shipping',
      'spinner',
      'username',
      'email',
      'search',
      'pinned',
      'q',
      'title',
      'opinion',
    ]) {
      assert.equal(soundsSecret(s), false, s);
    }
  });
});

describe('secret fields', () => {
  it('flags password inputs, autocomplete secrets, names, ids, labels and redacted subtrees', async () => {
    const p = page(`
      <input id="a" type="password">
      <input id="b" autocomplete="current-password">
      <input id="c" name="cvc">
      <input id="d" aria-label="Your API key">
      <label>One-time code <input id="e"></label>
      <div data-mc-redact><textarea id="f"></textarea></div>
      <input id="g" name="city">
      <input id="h" type="checkbox" name="remember_password">`);
    const el = (id: string): Element => {
      const e = p.document.getElementById(id);
      assert.ok(e);
      return e;
    };
    for (const id of ['a', 'b', 'c', 'd', 'e', 'f']) assert.equal(isSecretField(el(id)), true, id);
    assert.equal(isSecretField(el('g')), false);
    // A checkbox holds no typed secret even when its name mentions one.
    assert.equal(isSecretField(el('h')), false);
    await p.close();
  });
});
