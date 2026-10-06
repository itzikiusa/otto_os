import { test } from 'node:test';
import assert from 'node:assert/strict';
import { OFF_THREAD_MIN_BYTES, plainPreview, renderFailurePlan, rendersOffThread } from '../src/modules/vault/noteRenderPlan.ts';

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

test('a timed-out worker render never re-runs on the main thread (S18-305)', () => {
  // Only a worker that failed to LOAD falls back to the main-thread parse.
  assert.equal(renderFailurePlan('load', false), 'main-thread');
  // The note that hung the worker would hang the UI the same way.
  assert.equal(renderFailurePlan('timeout', false), 'timed-out');
  assert.equal(renderFailurePlan('timeout', true), 'timed-out');
  // Renders queued behind it get one retry on a fresh worker, then give up.
  assert.equal(renderFailurePlan('recycled', false), 'retry');
  assert.equal(renderFailurePlan('recycled', true), 'timed-out');
});
