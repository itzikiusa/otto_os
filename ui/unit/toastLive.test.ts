// Toast announcements (a11y P2): the live region must EXIST before a toast is
// inserted (WebKit/VoiceOver speak changes to an existing region; a region
// born with its text is often silent). So the always-mounted .toasts container
// is the polite region, and only errors carry their own (assertive) role.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const src = readFileSync(new URL('../src/lib/components/Toasts.svelte', import.meta.url), 'utf8').replace(/<!--[\s\S]*?-->/g, '');
const markup = src.slice(src.indexOf('</script>'), src.indexOf('<style'));

test('the persistent container is the polite live region', () => {
  const container = /<div class="toasts"[^>]*>/.exec(markup)?.[0] ?? '';
  assert.match(container, /aria-live="polite"/);
  assert.ok(markup.indexOf('class="toasts"') < markup.indexOf('{#each'), 'mounted outside the #each — present while empty');
  assert.doesNotMatch(markup.slice(0, markup.indexOf('class="toasts"')), /\{#if/, 'not behind an {#if}');
});

test('only an error toast has its own live role; every toast reads atomically', () => {
  assert.match(markup, /role=\{t\.level === 'error' \? 'alert' : undefined\}/);
  assert.doesNotMatch(markup, /'status'/, 'no per-toast role="status" (a region born with its text)');
  assert.match(markup, /aria-atomic="true"/);
});
