// Canvas keyboard editing (a11y P2): Enter / F2 in CanvasFlow → requestEdit(id)
// → only the node with that id opens its editor, once per request, and a
// request made before a node mounted never replays into it.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { importRunes, svelteClient } from './runesHarness.ts';

type Mod = { requestEdit: (id: string) => void; onEditRequest: (id: () => string, start: () => void) => void };

test('an edit request opens only the matching node, once per request', async () => {
  const mod = await importRunes<Mod>(new URL('../src/modules/canvas/editRequest.svelte.ts', import.meta.url));
  const $ = await svelteClient();
  mod.requestEdit('a'); // before any node mounted: must not replay
  const opened: string[] = [];
  const stop = $.effectRoot(() => {
    mod.onEditRequest(() => 'a', () => opened.push('a'));
    mod.onEditRequest(() => 'b', () => opened.push('b'));
  });
  $.flushSync();
  assert.deepEqual(opened, [], 'a request from before mount is ignored');
  mod.requestEdit('b');
  $.flushSync();
  assert.deepEqual(opened, ['b']);
  mod.requestEdit('b');
  $.flushSync();
  assert.deepEqual(opened, ['b', 'b'], 'asking again (after Esc) re-opens');
  mod.requestEdit('a');
  $.flushSync();
  assert.deepEqual(opened, ['b', 'b', 'a']);
  stop();
  mod.requestEdit('a');
  $.flushSync();
  assert.deepEqual(opened, ['b', 'b', 'a'], 'unmounted nodes stop listening');
});
