// The iteration-6 ui-guards ratchets. Their detectors are pure functions kept
// between the `p04-detectors` markers in scripts/ui-guards.mjs (the script
// runs on import, so the block is evaluated on its own here).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const src = readFileSync(new URL('../scripts/ui-guards.mjs', import.meta.url), 'utf8');
const block = src.slice(src.indexOf('// p04-detectors:start'), src.indexOf('// p04-detectors:end'));
const tagEndSrc = src.slice(src.indexOf('function tagEnd('), src.indexOf('\n}\n', src.indexOf('function tagEnd(')) + 3);
const d = new Function(`${tagEndSrc}\n${block}\nreturn { detectInsertedLiveRegion, detectBusyEmptyStatus, detectBareLazyImport, detectEmptyStateError, detectCouldNotCopy, detectIndexKeyedReorder };`)() as Record<string, (t: string) => { index: number; detail: string }[]>;
const n = (fn: string, text: string) => d[fn](text).length;

test('inserted-live-region: a status inserted with its text, not an always-mounted one', () => {
  assert.equal(n('detectInsertedLiveRegion', '{#if saved}<p role="status">Saved</p>{/if}'), 1);
  assert.equal(n('detectInsertedLiveRegion', '{#if err}\n  <p class="e" aria-live="polite">{err}</p>\n{/if}'), 1, 'next line counts');
  assert.equal(n('detectInsertedLiveRegion', '{#if err}<p role="alert">{err}</p>{/if}'), 1);
  assert.equal(n('detectInsertedLiveRegion', '{#if err}<EmptyState tone="error" role="alert" />{/if}'), 0, 'a toned alert is the error state');
  assert.equal(n('detectInsertedLiveRegion', '<p role="status">{msg}</p>'), 0, 'always mounted');
  assert.equal(n('detectInsertedLiveRegion', '{#if x}\n<p>a</p>\n<p role="status">b</p>{/if}'), 0, 'only the same or next line');
});

test('busy-empty-status: role=status with aria-busy=true', () => {
  assert.equal(n('detectBusyEmptyStatus', '<div class="sk" role="status" aria-busy="true"></div>'), 1);
  assert.equal(n('detectBusyEmptyStatus', '<div aria-busy={true} role="status">'), 1);
  assert.equal(n('detectBusyEmptyStatus', '<div role="status">Loading rows…</div>'), 0);
});

test('bare-lazy-import: {#await import()} needs a {:catch}', () => {
  assert.equal(n('detectBareLazyImport', "{#await import('./X.svelte') then m}<m.default />{/await}"), 1);
  assert.equal(n('detectBareLazyImport', "{#await import('./X.svelte')}<p/>{:then m}<m.default />{:catch e}<LoadState error={e} />{/await}"), 0);
});

test('empty-state-error: an EmptyState used as a load error without tone=error', () => {
  assert.equal(n('detectEmptyStateError', '<EmptyState icon="x" title="Couldn’t load object" body={err} />'), 1);
  assert.equal(n('detectEmptyStateError', '<EmptyState title={"Couldn\'t load"} />'), 1, 'straight apostrophe, braced');
  assert.equal(n('detectEmptyStateError', '<EmptyState title="Nothing yet" actionLabel="Retry" onaction={load} />'), 1);
  assert.equal(n('detectEmptyStateError', '<EmptyState tone="error" title="Couldn’t load object" actionLabel="Retry" />'), 0);
  assert.equal(n('detectEmptyStateError', '<EmptyState title="No work items yet" actionLabel="Refresh" />'), 0);
});

test('could-not-copy: “Could not” / “Cannot open” / lower-case “failed to”, not console lines', () => {
  assert.equal(n('detectCouldNotCopy', "this.fail('Could not open relay socket');"), 1);
  assert.equal(n('detectCouldNotCopy', '<p>Cannot open {path}</p>'), 1);
  assert.equal(n('detectCouldNotCopy', "toasts.error('Couldn’t load', 'the list failed to load');"), 1);
  assert.equal(n('detectCouldNotCopy', "console.warn('failed to parse', e);"), 0);
  assert.equal(n('detectCouldNotCopy', "toasts.error('Couldn’t open the relay socket');"), 0);
  assert.equal(n('detectCouldNotCopy', "'Failed to fetch'"), 0, 'capitalised browser text is not ours');
});

test('index-keyed-reorder: index keys in a file that swaps rows', () => {
  const editor = 'function moveStep(i, d) { const n = [...s]; [n[i], n[j]] = [n[j], n[i]]; }\n{#each steps as step, i (i)}<li/>{/each}';
  assert.equal(n('detectIndexKeyedReorder', editor), 1);
  assert.equal(n('detectIndexKeyedReorder', editor.replace('(i)}', '(step.key)}')), 0, 'stable keys');
  assert.equal(n('detectIndexKeyedReorder', 'function move(delta) { active += delta; }\n{#each items as it, i (i)}{/each}'), 0, 'a cursor move swaps nothing');
  assert.equal(n('detectIndexKeyedReorder', '{#each rows as r, i (i)}{/each}'), 0, 'no reorder at all');
});
