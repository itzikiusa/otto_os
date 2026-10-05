// S17-26: pin the iteration-1/2 security fixes in the automation slice, so a
// refactor can't silently drop them (nothing else tests them below e2e).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const src = (rel: string) => readFileSync(new URL(`../src/modules/${rel}`, import.meta.url), 'utf8');

test('the insights report frame is script-only: no popups, no same-origin', () => {
  const s = src('insights/ReportDetail.svelte');
  const sandboxes = [...s.matchAll(/sandbox="([^"]*)"/g)].map((m) => m[1]);
  assert.ok(sandboxes.length > 0, 'report iframe has a sandbox');
  for (const sb of sandboxes) {
    assert.ok(!/allow-popups|allow-same-origin|allow-top-navigation/.test(sb), `sandbox too loose: ${sb}`);
  }
});

test('the insights link bridge only honours the frame, http(s), a user gesture and a rate limit', () => {
  const s = src('insights/ReportDetail.svelte');
  const fn = s.slice(s.indexOf('function onFrameMessage'), s.indexOf('let html = $state'));
  assert.match(fn, /ev\.source !== frameEl\.contentWindow/);
  assert.match(fn, /\^https\?:/);
  assert.match(fn, /userActivation/);
  assert.match(fn, /OPEN_GAP_MS/);
});

test('plugin frames: messages are source-checked and the slug is validated before framing', () => {
  const s = src('plugins/PluginFrame.svelte');
  assert.match(s, /ev\.source !== frame\.contentWindow/);
  assert.match(s, /const slugOk = \$derived\(\/\^\[a-z\]\[a-z0-9-\]/);
  assert.match(s, /if \(!slugOk\) return;/, 'otto:init (with the token) is never posted for an invalid slug');
  assert.match(s, /encodeURIComponent\(slug\)/);
});
