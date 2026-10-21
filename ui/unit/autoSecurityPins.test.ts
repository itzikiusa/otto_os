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

// The link bridge and the plugin frame's message/init rules are tested by
// behaviour in frameMessages.test.ts; these pins only check the components
// still route through those helpers (markup/wiring, not logic).
test('the insights report frame routes messages through reportLinkBridge', () => {
  const s = src('insights/ReportDetail.svelte');
  assert.match(s, /from '\.\/reportFrame'/);
  assert.match(s, /const onFrameMessage = reportLinkBridge\(/);
  assert.match(s, /<svelte:window onmessage=\{onFrameMessage\} \/>/);
});

test('plugin frames: messages and otto:init go through pluginFrame.ts; the URL encodes the slug', () => {
  const s = src('plugins/PluginFrame.svelte');
  assert.match(s, /const slugOk = \$derived\(pluginSlugOk\(slug\)\)/);
  assert.match(s, /pluginFrameAction\(ev, frame\?\.contentWindow\)/);
  assert.match(s, /postPluginInit\(slug,/);
  assert.match(s, /postPluginInit\(slug, \(extra\) => postTheme\('otto:init', extra\)/);
  assert.equal(s.split("postTheme('otto:init'").length - 1, 1, 'otto:init is posted only through postPluginInit');
  assert.match(s, /encodeURIComponent\(slug\)/);
});
