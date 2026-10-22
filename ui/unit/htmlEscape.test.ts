// The one HTML escaper (lib/html.ts): all five significant characters, so its
// output is safe both as element text and inside a quoted attribute.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { escapeHtml } from '../src/lib/html.ts';

test('escapeHtml escapes & < > " and \'', () => {
  assert.equal(escapeHtml(`<a href="x" title='y'>&</a>`), '&lt;a href=&quot;x&quot; title=&#39;y&#39;&gt;&amp;&lt;/a&gt;');
  assert.equal(escapeHtml('plain'), 'plain');
  assert.equal(escapeHtml('&amp;'), '&amp;amp;', 'already-escaped input is escaped again (no double-decode)');
});
