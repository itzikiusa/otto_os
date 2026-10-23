// The iteration-6 ui-guards ratchets. Their detectors are pure functions kept
// between the `p04-detectors` markers in scripts/ui-guards.mjs (the script
// runs on import, so the block is evaluated on its own here).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const src = readFileSync(new URL('../scripts/ui-guards.mjs', import.meta.url), 'utf8');
const block = src.slice(src.indexOf('// p04-detectors:start'), src.indexOf('// p04-detectors:end'));
const tagEndSrc = src.slice(src.indexOf('function tagEnd('), src.indexOf('\n}\n', src.indexOf('function tagEnd(')) + 3);
const d = new Function(`${tagEndSrc}\n${block}\nreturn { detectInsertedLiveRegion, detectBusyEmptyStatus, detectBareLazyImport, detectEmptyStateError, detectCouldNotCopy, detectIndexKeyedReorder, detectIconButtonName, detectCrossModuleImport, detectGuessedChecked, detectClipboardNoNextStep, detectUnitSourceRegex, detectAnnounceInRegion, detectGuessedDerived };`)() as Record<string, (...a: string[]) => any>;
const n = (fn: string, text: string) => d[fn](text).length;

test('inserted-live-region: a status inserted with its text, not an always-mounted one', () => {
  assert.equal(n('detectInsertedLiveRegion', '{#if saved}<p role="status">Saved</p>{/if}'), 1);
  assert.equal(n('detectInsertedLiveRegion', '{#if err}\n  <p class="e" aria-live="polite">{err}</p>\n{/if}'), 1, 'next line counts');
  assert.equal(n('detectInsertedLiveRegion', '{#if err}<p role="alert">{err}</p>{/if}'), 1);
  assert.equal(n('detectInsertedLiveRegion', '{#if err}<EmptyState tone="error" role="alert" />{/if}'), 0, 'a toned alert is the error state');
  assert.equal(n('detectInsertedLiveRegion', '<p role="status">{msg}</p>'), 0, 'always mounted');
  assert.equal(n('detectInsertedLiveRegion', '{#if x}\n<p>a</p>\n<p>b</p>\n<p role="status">c</p>{/if}'), 0, 'only the opener line and the next two');
  assert.equal(n('detectInsertedLiveRegion', '{#if x}<p>a</p>{/if}\n<p role="status">{msg}</p>'), 0, 'a region after the branch closes is not in it');
});

// Quality iteration 6, wave 3 (P8): the widened opener set and look-ahead.
test('inserted-live-region: {:else if} / {:else} branches, comments and {@const} lines', () => {
  assert.equal(n('detectInsertedLiveRegion', '{#if a}<p/>\n{:else if busy}\n  <p class="hint" role="status">Loading…</p>\n{/if}'), 1, '{:else if} + region on the next line');
  assert.equal(n('detectInsertedLiveRegion', '{#if a}<p/>{:else}\n  <p aria-live="polite">{n} results</p>\n{/if}'), 1, '{:else} branch');
  // Comments are blanked before the detector runs (so a comment line is a blank one).
  assert.equal(n('detectInsertedLiveRegion', '{#if saved}\n  \n  <p role="status">Saved</p>\n{/if}'), 1, '{#if} + comment line + region');
  assert.equal(n('detectInsertedLiveRegion', '{#if row}\n  {@const label = row.name}\n  <p role="status">{label}</p>\n{/if}'), 1, '{#if} + {@const} + region');
  assert.equal(n('detectInsertedLiveRegion', '{#if a}\n  <div class="wrap">\n    <p role="status">Saved</p>\n  </div>\n{/if}'), 1, 'two lines past the opener');
  assert.equal(d.detectInsertedLiveRegion('{#if a}<p/>{:else if b}\n<p role="status">x</p>{/if}')[0].detail, '{:else if …} + role="status"');
  assert.equal(n('detectInsertedLiveRegion', '<p class="sr-only" role="status">{busy ? \'Loading…\' : \'\'}</p>\n{#if busy}<Spinner />{/if}'), 0, 'always mounted, text changes');
  assert.equal(n('detectInsertedLiveRegion', '{#if err}\n  <EmptyState tone="error" title="Couldn’t load" />\n{:else}\n  <p>{rows.length} rows</p>\n{/if}'), 0, 'no region in either branch');
});

test('inserted-live-region: one hit per region, aria-live="off" and snippet bodies are not regions', () => {
  assert.equal(n('detectInsertedLiveRegion', '{#if a}\n  {#if b}\n    <p role="status">x</p>\n  {/if}\n{/if}'), 1, 'nested openers report the region once');
  assert.equal(n('detectInsertedLiveRegion', '{#if hud}\n  <div class="lat-hud" aria-live="off" role="group">…</div>\n{/if}'), 0, 'aria-live="off" is no live region');
  assert.equal(n('detectInsertedLiveRegion', '{:else}\n{#snippet body()}\n  <span class="sr-only" role="status">{msg}</span>\n{/snippet}'), 0, 'a snippet body mounts where it is rendered');
  // role="alert" is reported with its kind: the script files it under the
  // separate `inserted-alert` ratchet (a real field error may keep it).
  assert.equal(d.detectInsertedLiveRegion('{#if err}<p role="alert">{err}</p>{/if}')[0].kind, 'role="alert"');
  assert.equal(d.detectInsertedLiveRegion('{#if ok}<p role="status">Saved</p>{/if}')[0].kind, 'role="status"');
});

test('announce-in-region: use:announceShown on or inside a mounted status region (6w2-02)', () => {
  const bubble = (inner: string) => `{#if sending}\n<div class="bubble" role="status">\n  <div class="row"><Dot /></div>\n${inner}\n</div>\n{/if}`;
  assert.equal(n('detectAnnounceInRegion', bubble('  {#if note}<p use:announceShown={note}>{note}</p>{/if}')), 1, 'inside the bubble, after a closed sibling div');
  assert.equal(n('detectAnnounceInRegion', '<p role="status" use:announceShown={msg}>{msg}</p>'), 1, 'on the region itself');
  assert.equal(n('detectAnnounceInRegion', '<span aria-live="polite">\n  <b use:announceShown={t}>{t}</b>\n</span>'), 1, 'aria-live counts too');
  assert.equal(n('detectAnnounceInRegion', '<div role="status">{count}</div>\n<p use:announceShown={msg}>{msg}</p>'), 0, 'the region closed before the line');
  assert.equal(n('detectAnnounceInRegion', '<div class="bubble">\n  <p use:announceShown={note}>{note}</p>\n</div>'), 0, 'no region at all');
  assert.equal(n('detectAnnounceInRegion', '<div aria-live="off">\n  <p use:announceShown={note}>{note}</p>\n</div>'), 0, 'aria-live="off"');
});

test('guessed-derived: $derived(… ?? true|false) rendered as a state word', () => {
  const code = (decl: string, markup: string) => [decl, markup] as const;
  const hit = (decl: string, markup: string) => d.detectGuessedDerived(...code(decl, markup)).length;
  assert.equal(hit('const attached = $derived(attach?.attached ?? true);', '<p>Built-in tools: {attached ? \'attached\' : \'not attached\'}</p>'), 1);
  assert.equal(hit('const on = $derived(cfg?.enabled ?? false);', '<b>{!on ? `Off` : `On`}</b>'), 1, 'negated, template literal');
  assert.equal(hit('const on = $derived(cfg?.enabled ?? false);', '<button disabled={on ? true : false} title={on ? \'Stop\' : \'Start\'}>x</button>'), 0, 'attributes are not the rendered state line');
  assert.equal(hit('const on = $derived(cfg?.enabled);', '<b>{on ? \'On\' : \'Off\'}</b>'), 0, 'no guessed fallback');
  assert.equal(hit('const n = $derived(rows?.length ?? 0);', '<b>{n ? \'some\' : \'none\'}</b>'), 0, 'only boolean guesses');
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
