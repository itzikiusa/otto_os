// accessibility-responsive-6-07: while a link preview shows, its anchor points
// aria-describedby at the popover; hiding it (Escape) clears that and never
// touches focus.
import {test} from 'node:test';
import assert from 'node:assert/strict';
import {loadSource} from './sourceHarness.ts';

test('the shown preview describes its anchor and hide() clears it', async () => {
  const vault = {wsId: 'ws', current: {id: 1}, notePath: 'open.md', status: {generation: '1'}};
  const timers: Array<() => void> = [];
  const {linkPreview, LINK_PREVIEW_ID} = loadSource(new URL('../src/modules/vault/previewStore.svelte.ts', import.meta.url), {
    '../../lib/api/vault': {vaultNote: async () => ({meta: {title: 'T', description: 'd', okf_type: null}, raw: ''})},
    './structuredNote': {previewText: (t: string) => t}, './vault.svelte': {vault},
  }, {setTimeout: (run: () => void) => {timers.push(run); return timers.length;}, clearTimeout() {}});
  const attrs = new Map<string, string>();
  let focused = 0;
  const anchor = {
    getBoundingClientRect: () => ({left: 0, top: 0, bottom: 10}),
    setAttribute: (k: string, v: string) => attrs.set(k, v),
    removeAttribute: (k: string) => attrs.delete(k),
    focus: () => focused++,
  };
  linkPreview.show(anchor, 'other.md');
  assert.equal(attrs.get('aria-describedby'), undefined, 'not before the popover shows');
  timers.shift()!();
  assert.equal(attrs.get('aria-describedby'), LINK_PREVIEW_ID);
  linkPreview.hide();
  assert.equal(linkPreview.current, null);
  assert.equal(attrs.has('aria-describedby'), false);
  assert.equal(focused, 0, 'hiding never moves focus');
});
