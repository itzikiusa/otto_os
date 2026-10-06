// S18-04: autofill must re-check the tab's origin AFTER the confirmation and
// the reveal (a redirect while the dialog is open must not receive the
// password), and the injected script re-checks the page's own origin.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildFillScript, fillResult, fillTargetOk, matchDomain } from '../src/modules/browser/autofill.ts';

type Field = { value: string; focus(): void; dispatchEvent(): void };
function field(): Field {
  return { value: '', focus() {}, dispatchEvent() {} };
}

/** Run the injected script against a fake page at `url`. */
function runAt(url: string, js: string) {
  const u = new URL(url);
  const pwd = field();
  const user = field();
  const document = {
    querySelector(sel: string) {
      if (sel.includes('password')) return pwd;
      if (sel.includes('email')) return user;
      return null;
    },
  };
  class Event { type: string; constructor(type: string) { this.type = type; } }
  const fn = new Function('location', 'document', 'Event', `return ${js};`);
  const result = fn({ hostname: u.hostname, protocol: u.protocol }, document, Event) as string;
  return { result, pwd, user };
}

test('the script fills only on the credential domain or its subdomains', () => {
  const js = buildFillScript('example.com', 'ann', 's3cr3t');
  const ok = runAt('https://login.example.com/sso', js);
  assert.equal(ok.result, 'filled');
  assert.equal(ok.pwd.value, 's3cr3t');
  assert.equal(ok.user.value, 'ann');

  for (const url of ['https://evil.example/login', 'https://example.com.evil.io/', 'https://notexample.com/']) {
    const r = runAt(url, js);
    assert.equal(r.result, 'origin-changed', url);
    assert.equal(r.pwd.value, '', `password leaked into ${url}`);
  }
});

test('the script refuses a downgrade to plain http off loopback', () => {
  const r = runAt('http://example.com/login', buildFillScript('example.com', 'a', 'p'));
  assert.equal(r.result, 'origin-changed');
  assert.equal(r.pwd.value, '');
});

test('the password and username are JSON-escaped into the script', () => {
  const r = runAt('https://example.com/', buildFillScript('example.com', 'a"b', "x');alert(1);//"));
  assert.equal(r.result, 'filled');
  assert.equal(r.pwd.value, "x');alert(1);//");
  assert.equal(r.user.value, 'a"b');
});

test('host-side re-check: a redirect during the confirmation blocks the fill', () => {
  assert.equal(fillTargetOk('https://app.example.com/login', 'example.com'), true);
  assert.equal(fillTargetOk('https://evil.example/login', 'example.com'), false);
  assert.equal(fillTargetOk('http://example.com/login', 'example.com'), false);
  assert.equal(fillTargetOk(undefined, 'example.com'), false);
  assert.equal(matchDomain('EXAMPLE.com.', 'example.com'), true);
  assert.equal(matchDomain('badexample.com', 'example.com'), false);
});

test('eval results are normalised whether quoted or raw', () => {
  assert.equal(fillResult("'origin-changed'"), 'origin-changed');
  assert.equal(fillResult('"filled"'), 'filled');
  assert.equal(fillResult('no-password-field'), 'no-password-field');
});

test('the in-page origin check survives a page that poisons String built-ins (S18-307)', () => {
  const js = buildFillScript('example.com', 'ann', 's3cr3t');
  const proto = String.prototype as unknown as Record<string, unknown>;
  const saved = { slice: proto.slice, replace: proto.replace, toLowerCase: proto.toLowerCase };
  const G = globalThis as unknown as { String: unknown };
  const savedString = G.String;
  // Every override makes a naive `h === D || h.slice(...) === '.' + D` pass.
  proto.slice = () => '.example.com';
  proto.replace = () => 'example.com';
  proto.toLowerCase = () => 'example.com';
  G.String = () => 'example.com';
  try {
    for (const url of ['https://evil.example/login', 'https://example.com.evil.io/']) {
      const r = runAt(url, js);
      assert.equal(r.result, 'origin-changed', url);
      assert.equal(r.pwd.value, '', `password leaked into ${url}`);
    }
    assert.equal(runAt('https://login.example.com/', js).result, 'filled');
  } finally {
    Object.assign(proto, saved);
    G.String = savedString;
  }
  assert.equal(runAt('https://example.com./', js).result, 'filled', 'a trailing-dot host still matches');
  assert.equal(runAt('http://localhost/', buildFillScript('localhost', 'a', 'p')).result, 'filled');
});
