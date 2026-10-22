// Product Rewrite / Plan / Test cases: the "reset on story change" effect
// (product/storyReset.svelte.ts) depends on the story id ONLY. Its reset used
// to call `clearPoll()` tracked, reading the tab's `$state.raw` poller, so
// Generate's `startPolling()` write re-ran the effect: the teardown stopped
// the poller just started (re-enabling Generate) and the reset wiped the tab.
// Driven under the real Svelte client runtime (runesHarness.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { importRunes, svelteClient } from './runesHarness.ts';

type Mod = {
  onStoryChange: (id: () => string | null, reset: (id: string | null) => void, teardown: () => void) => void;
};
const mod = await importRunes<Mod>(new URL('../src/modules/product/storyReset.svelte.ts', import.meta.url));

test('generate() keeps the poller running and Generate disabled', async () => {
  const $ = await svelteClient();
  const story = $.state<string | null>('A');
  // The tab's `let pollTimer = $state.raw<Poller | null>(null)`.
  const pollTimer = $.state<{ stopped: boolean } | null>(null);
  let resets = 0;
  let loads = 0;
  // The tab's clearPoll(): reads AND writes the poller state.
  const clearPoll = () => {
    const p = $.get(pollTimer);
    if (p !== null) {
      p.stopped = true;
      $.set(pollTimer, null);
    }
  };
  const generating = false;
  const disabled = () => generating || $.get(pollTimer) !== null;
  const stop = $.effectRoot(() => {
    mod.onStoryChange(() => $.get(story), (sid) => {
      resets++;
      clearPoll();
      if (sid) loads++;
    }, clearPoll);
  });
  $.flushSync();
  assert.equal(resets, 1);
  assert.equal(loads, 1);

  // generate() → startPolling(): clearPoll() then assign the new poller.
  clearPoll();
  const poller = { stopped: false };
  $.set(pollTimer, poller);
  $.flushSync();
  assert.equal(poller.stopped, false, 'the poller Generate started is still running');
  assert.equal($.get(pollTimer), poller);
  assert.equal(disabled(), true, 'Generate stays disabled while polling');
  assert.equal(resets, 1, 'the tab was not reset by the poller write');
  assert.equal(loads, 1);

  // A real story switch still resets and stops the poller.
  $.set(story, 'B');
  $.flushSync();
  assert.equal(poller.stopped, true);
  assert.equal(resets, 2);
  assert.equal(loads, 2);
  assert.equal(disabled(), false);
  stop();
});

test('Rewrite, Plan and Test cases reset through onStoryChange, not a tracked effect', () => {
  for (const tab of ['RewriteTab', 'PlanTab', 'TestCasesTab']) {
    const src = readFileSync(new URL(`../src/modules/product/${tab}.svelte`, import.meta.url), 'utf8');
    assert.match(src, /onStoryChange\(\(\) => product\.selectedId,/, tab);
    // No effect body reads `product.selectedId;` bare and then clears the poll.
    assert.doesNotMatch(src, /\$effect\(\(\) => \{\s*product\.selectedId;/, tab);
  }
});

// ui-correctness-5-03: the Design arena's list load is owned by the story it
// started for, and a switch drops the previous story's artifacts at once.
test('DesignArena commits a list load only while its story is current', () => {
  const src = readFileSync(new URL('../src/modules/product/design/DesignArena.svelte', import.meta.url), 'utf8');
  const load = src.slice(src.indexOf('async function loadAll('), src.indexOf('let childSeq'));
  assert.match(load, /const my = \+\+loadSeq;/);
  assert.match(load, /product\.captureSelection\(\)/);
  assert.match(load, /const own = await product\.listAttachments\(\);\s*if \(!current\(\)\) return;\s*ownAtts = own;/);
  const effect = src.slice(src.indexOf('const sid = product.selectedId;'), src.indexOf('void loadAll();'));
  for (const reset of ['ownAtts = [];', 'childAtts = {};', 'selectedId = null;']) assert.ok(effect.includes(reset), reset);
});
