import { test } from 'node:test';
import assert from 'node:assert/strict';
import { OFF_THREAD_MIN_BYTES, plainPreview, rendersOffThread } from '../src/modules/vault/noteRenderPlan.ts';

test('only notes over 64 KB leave the main thread', () => {
  assert.equal(OFF_THREAD_MIN_BYTES, 64 * 1024);
  assert.equal(rendersOffThread(''), false);
  assert.equal(rendersOffThread('x'.repeat(OFF_THREAD_MIN_BYTES)), false);
  assert.equal(rendersOffThread('x'.repeat(OFF_THREAD_MIN_BYTES + 1)), true);
});

test('the placeholder is the escaped plain body, marked busy', () => {
  const html = plainPreview('# Title\n<script>alert("x")</script> & \'q\'');
  assert.match(html, /^<pre class="note-plain" aria-busy="true">/);
  assert.ok(!html.includes('<script>'));
  assert.ok(html.includes('&#60;script&#62;alert(&#34;x&#34;)&#60;/script&#62; &#38; &#39;q&#39;'));
  assert.ok(html.endsWith('</pre>'));
});
