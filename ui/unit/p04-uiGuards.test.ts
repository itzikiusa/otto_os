// The iteration-6 ui-guards ratchets. Their detectors are pure functions kept
// between the `p04-detectors` markers in scripts/ui-guards.mjs (the script
// runs on import, so the block is evaluated on its own here).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const src = readFileSync(new URL('../scripts/ui-guards.mjs', import.meta.url), 'utf8');
const block = src.slice(src.indexOf('// p04-detectors:start'), src.indexOf('// p04-detectors:end'));
const tagEndSrc = src.slice(src.indexOf('function tagEnd('), src.indexOf('\n}\n', src.indexOf('function tagEnd(')) + 3);
const d = new Function(`${tagEndSrc}\n${block}\nreturn { detectInsertedLiveRegion, detectBusyEmptyStatus, detectBareLazyImport, detectEmptyStateError, detectCouldNotCopy, detectIndexKeyedReorder, detectIconButtonName, detectCrossModuleImport, detectGuessedChecked, detectClipboardNoNextStep, detectUnitSourceRegex };`)() as Record<string, (...a: string[]) => any>;
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

// Quality iteration 6, wave 2 (P8): the layering, clipboard, loading-checkbox
// and source-regex ratchets, and the hard icon-button rule's detector.

test('icon-button-name (hard rule 3): an <Icon/>-only button needs aria-label AND title', () => {
  assert.equal(n('detectIconButtonName', '<button onclick={x}><Icon name="trash" /></button>'), 1);
  assert.equal(d.detectIconButtonName('<button aria-label="Delete row"><Icon name="trash" /></button>')[0].detail, 'title');
  assert.equal(d.detectIconButtonName('<button title="Delete row"><Icon name="trash" /></button>')[0].detail, 'aria-label');
  assert.equal(n('detectIconButtonName', '<button aria-label="Delete row" title="Delete row"><Icon name="trash" /></button>'), 0);
  assert.equal(n('detectIconButtonName', '<button title="Copy"><Icon name="copy" /> Copy</button>'), 0, 'a text label names it');
  assert.equal(n('detectIconButtonName', '<button {...props}><Icon name="x" /></button>'), 0, 'spread props may carry the labels');
});

test('cross-module-import: modules/<a> reaching into modules/<b>, not its own module or lib/', () => {
  assert.equal(d.detectCrossModuleImport('src/modules/agents/NewSession.svelte', 'src/modules/product/story'), true);
  assert.equal(d.detectCrossModuleImport('src/modules/agents/NewSession.svelte', 'src/modules/agents/conversation/x'), false);
  assert.equal(d.detectCrossModuleImport('src/modules/agents/NewSession.svelte', 'src/lib/clipboard'), false);
  assert.equal(d.detectCrossModuleImport('src/lib/uiCommands/vault.ts', 'src/modules/vault/vault.svelte'), false, 'lib → modules is the hard rule');
});

test('guessed-checked: checked={… ?? true|false}', () => {
  assert.equal(n('detectGuessedChecked', '<input type="checkbox" checked={status?.enabled ?? false} />'), 1);
  assert.equal(n('detectGuessedChecked', '<input type="checkbox" checked={cfg.attach ?? true} disabled={!cfg} />'), 1);
  assert.equal(n('detectGuessedChecked', '<input type="checkbox" checked={!!status?.enabled} disabled={!status} />'), 0);
  assert.equal(n('detectGuessedChecked', '<input type="checkbox" bind:checked={on} />'), 0);
});

test('clipboard-no-next-step: a copy failure toast must say what to do next', () => {
  assert.equal(n('detectClipboardNoNextStep', "toasts.error('Couldn’t copy the username', 'The clipboard write was blocked.');"), 1);
  assert.equal(n('detectClipboardNoNextStep', "toasts.warn('Couldn’t copy the path');"), 1, 'no body at all');
  assert.equal(n('detectClipboardNoNextStep', "toasts.warn('Copy failed', 'The clipboard is not available here.');"), 1);
  assert.equal(n('detectClipboardNoNextStep', "toasts.error('Couldn’t copy', COPY_FAILED_BODY);"), 0);
  assert.equal(n('detectClipboardNoNextStep', "toasts.error('Couldn’t copy', `The clipboard isn’t available here. Select the path and copy it manually: ${p}`);"), 0);
  assert.equal(n('detectClipboardNoNextStep', "toasts.error('Couldn’t copy the skill', 'The clipboard is unavailable. Use Download instead.');"), 0, 'another route is a next step');
  assert.equal(n('detectClipboardNoNextStep', "toasts.success('Copied', 'to the clipboard');"), 0, 'success is not a failure path');
  assert.equal(n('detectClipboardNoNextStep', "toasts.warn('Nothing selected', 'Pick a row first.');"), 0);
});

test('unit-source-regex: a unit test reading .svelte source, directly or via a helper', () => {
  assert.equal(n('detectUnitSourceRegex', "const s = readFileSync(new URL('../src/modules/git/X.svelte', import.meta.url), 'utf8');"), 1); // ui-guards: allow (fixture)
  assert.equal(n('detectUnitSourceRegex', "const src = (p: string) => readFileSync(new URL(`../src/modules/${p}`, import.meta.url), 'utf8');\nsrc('git/PrList.svelte'); src('git/A.svelte'); src('git/b.ts');"), 2, 'helper calls on .svelte only'); // ui-guards: allow (fixture)
  assert.equal(n('detectUnitSourceRegex', "readFileSync(new URL('../src/lib/clipboard.ts', import.meta.url), 'utf8');"), 0, 'a .ts store is loaded, not regexed');
  assert.equal(n('detectUnitSourceRegex', "readFileSync(new URL('../src/modules/vault/vault.svelte.ts', import.meta.url), 'utf8');"), 0, 'a .svelte.ts rune store is not component source');
  assert.equal(n('detectUnitSourceRegex', "componentFunctions(new URL('../src/shell/Palette.svelte', import.meta.url), ['run'], {});"), 0, 'driving handlers is the fix'); // ui-guards: allow (fixture)
});
