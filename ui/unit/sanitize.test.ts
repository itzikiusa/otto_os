// S18-07: the markdown sanitizer's URL policy. node:test has no DOMParser,
// so this drives the exported policy functions sanitizeHtml applies to every
// href/src: a network-path link (`//evil/login`) is EXTERNAL (gets
// target=_blank, can't swap the app's webview), obfuscated schemes are
// refused, and `data:` is only a non-SVG image.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isExternalHref, isNetworkPath, urlOk } from '../src/lib/sanitize.ts';

test('network-path references are external, not relative', () => {
  for (const v of ['//evil.example/login', ' //evil.example', '/\\evil.example', '\\\\evil.example', '/\t/evil.example']) {
    assert.equal(isNetworkPath(v), true, JSON.stringify(v));
    assert.equal(isExternalHref(v), true, JSON.stringify(v));
  }
  assert.equal(urlOk('//evil.example/login', 'a'), true); // kept, but as https + _blank
});

test('relative, fragment and query hrefs stay internal', () => {
  for (const v of ['/notes/a.md', 'a.md', '#heading', '?q=1', '']) {
    assert.equal(isExternalHref(v), false, JSON.stringify(v));
    assert.equal(urlOk(v, 'a'), true, JSON.stringify(v));
  }
});

test('http(s) is external; mailto allowed but not external', () => {
  assert.equal(isExternalHref('https://example.com'), true);
  assert.equal(isExternalHref(' HTTP://example.com'), true);
  assert.equal(urlOk('mailto:a@b.c', 'a'), true);
  assert.equal(isExternalHref('mailto:a@b.c'), false);
});

test('script-ish schemes are refused, even obfuscated', () => {
  for (const v of ['javascript:alert(1)', 'java\tscript:alert(1)', ' \u0001JaVaScRiPt:alert(1)', 'vbscript:x', 'data:text/html,<b>x</b>']) {
    assert.equal(urlOk(v, 'a'), false, JSON.stringify(v));
    assert.equal(urlOk(v, 'img'), false, JSON.stringify(v));
  }
});

test('data: only as a non-SVG image', () => {
  assert.equal(urlOk('data:image/png;base64,AAAA', 'img'), true);
  assert.equal(urlOk('data:image/svg+xml,<svg onload=alert(1)>', 'img'), false);
  assert.equal(urlOk('data:image/png;base64,AAAA', 'a'), false);
});
