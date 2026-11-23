import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';
import { learningRefHref } from '../src/modules/product/learningRefs.ts';
import { shouldIgnoreVaultShortcut } from '../src/modules/vault/vaultShortcuts.ts';

test('learningRefHref allows only http(s) URLs and blocks javascript:/data: protocols', () => {
  assert.equal(learningRefHref({ type: 'url', ref: 'https://example.com/spec' }), 'https://example.com/spec');
  assert.equal(learningRefHref({ type: 'jira', ref: 'http://jira.local/browse/ABC-1' }), 'http://jira.local/browse/ABC-1');
  assert.equal(learningRefHref({ type: 'confluence', ref: 'https://wiki.local/pages/1' }), 'https://wiki.local/pages/1');
  assert.equal(learningRefHref({ type: 'url', ref: 'javascript:alert(1)' }), null);
  assert.equal(learningRefHref({ type: 'url', ref: 'java\tscript:alert(1)' }), null);
  assert.equal(learningRefHref({ type: 'url', ref: 'data:text/html,<script>alert(1)</script>' }), null);
  assert.equal(learningRefHref({ type: 'url', ref: '//evil.example/x' }), null);
  assert.equal(learningRefHref({ type: 'other', ref: 'https://example.com' }), null);
});

test('shouldIgnoreVaultShortcut respects open modals, PTY terminals, and editor inputs', () => {
  const mkTarget = (matches: string[]) => ({
    closest(sel: string) {
      return matches.some((m) => sel.includes(m)) ? {} : null;
    },
  });
  const modalDoc = { querySelector: () => ({}), activeElement: null } as unknown as Document;
  const plainDoc = { querySelector: () => null, activeElement: null } as unknown as Document;

  // Open modal blocks every shortcut (even ⌘).
  assert.equal(shouldIgnoreVaultShortcut({ metaKey: true, ctrlKey: false, target: null }, { doc: modalDoc }), true);

  // Ctrl-only inside terminal (.xterm) is ignored even with allowInEditor.
  const termDoc = { querySelector: () => null, activeElement: mkTarget(['.xterm']) } as unknown as Document;
  assert.equal(shouldIgnoreVaultShortcut({ metaKey: false, ctrlKey: true, target: null }, { doc: termDoc }), true);
  assert.equal(shouldIgnoreVaultShortcut({ metaKey: false, ctrlKey: true, target: null }, { doc: termDoc, allowInEditor: true }), true);

  // Ctrl-only inside CodeMirror (.cm-editor): blocked for Ctrl+E/O/N, allowed for Ctrl+S.
  const cmDoc = { querySelector: () => null, activeElement: mkTarget(['.cm-editor']) } as unknown as Document;
  assert.equal(shouldIgnoreVaultShortcut({ metaKey: false, ctrlKey: true, target: null }, { doc: cmDoc }), true);
  assert.equal(shouldIgnoreVaultShortcut({ metaKey: false, ctrlKey: true, target: null }, { doc: cmDoc, allowInEditor: true }), false);

  // ⌘ shortcut outside modal is allowed.
  assert.equal(shouldIgnoreVaultShortcut({ metaKey: true, ctrlKey: false, target: null }, { doc: plainDoc }), false);
});

test('ProductPage setFolder preserves folder on Cancel and clears folder on empty submit', async () => {
  const patches: Array<{ id: string; patch: unknown }> = [];
  let promptResult: string | null = null;
  let capturedOpts: Record<string, unknown> | undefined;
  const state = componentFunctions(new URL('../src/modules/product/ProductPage.svelte', import.meta.url), ['setFolder'], {
    product: {
      childrenOf: () => [],
      patchStory: async (id: string, patch: unknown) => { patches.push({ id, patch }); },
    },
    confirmer: {
      promptText: async (_msg: string, opts?: Record<string, unknown>) => {
        capturedOpts = opts;
        return promptResult;
      },
    },
    toastError() {},
  });

  // Cancel returns null → must NOT wipe existing folder.
  promptResult = null;
  await state.setFolder({ id: 's1', parent_id: 'e1', folder: 'Design' });
  assert.equal(capturedOpts?.allowEmpty, true);
  assert.equal(patches.length, 0);

  // Explicit empty submit returns '' → clears folder.
  promptResult = '';
  await state.setFolder({ id: 's1', parent_id: 'e1', folder: 'Design' });
  assert.deepEqual(JSON.parse(JSON.stringify(patches)), [{ id: 's1', patch: { folder: '' } }]);
});

test('SceneList move un-files scene on empty submit and aborts on Cancel', async () => {
  const updates: Array<{ id: string; patch: unknown }> = [];
  let promptResult: string | null = null;
  let capturedOpts: Record<string, unknown> | undefined;
  const state = componentFunctions(new URL('../src/modules/canvas/SceneList.svelte', import.meta.url), ['move'], {
    canvas: {
      updateMeta: async (id: string, patch: unknown) => { updates.push({ id, patch }); },
    },
    confirmer: {
      promptText: async (_msg: string, opts?: Record<string, unknown>) => {
        capturedOpts = opts;
        return promptResult;
      },
    },
    toastError() {},
  });

  promptResult = null;
  await state.move(null, { id: 'c1', section: 'Platform/API' });
  assert.equal(capturedOpts?.allowEmpty, true);
  assert.equal(updates.length, 0);

  promptResult = '';
  await state.move(null, { id: 'c1', section: 'Platform/API' });
  assert.deepEqual(JSON.parse(JSON.stringify(updates)), [{ id: 'c1', patch: { section: null } }]);
});

test('Vault Switcher clamps ArrowDown selection to the rendered 30-item slice', async () => {
  const opened: string[] = [];
  const hits = Array.from({ length: 40 }, (_, i) => ({ path: `note-${i}.md`, title: `Note ${i}` }));
  const state = componentFunctions(new URL('../src/modules/vault/Switcher.svelte', import.meta.url), ['close', 'pick', 'createFromQuery', 'onKey'], {
    vault: { switcherOpen: true, open(p: string) { opened.push(p); }, createNote() {} },
    query: 'note', hits, sel: 28, loading: false, lookupError: '', resolvedQuery: 'note',
  });
  const ev = (key: string) => ({ key, shiftKey: false, preventDefault() {} });
  state.onKey(ev('ArrowDown'));
  assert.equal(state.sel, 29);
  state.onKey(ev('ArrowDown'));
  assert.equal(state.sel, 29);
  state.onKey(ev('Enter'));
  assert.deepEqual(opened, ['note-29.md']);
});

test('PlanTab loadPlanBody drops stale version responses after a story switch', async () => {
  const req = deferred<{ id: string; body_md: string }>();
  const product = {
    selectedId: 'story-A',
    captureSelection() {
      const id = this.selectedId;
      return () => this.selectedId === id;
    },
    getVersion: () => req.promise,
  };
  const state = componentFunctions(new URL('../src/modules/product/PlanTab.svelte', import.meta.url), ['loadPlanBody'], {
    product, loadSeq: 0, loading: false, planVersion: null, body: '', pollFailures: 0, lastPollError: null, loadError: null,
    loadErrorText: String, toastError() {},
  });
  const pending = state.loadPlanBody({ id: 'vA' });
  product.selectedId = 'story-B';
  req.resolve({ id: 'vA', body_md: '# Story A plan' });
  const ok = await pending;
  assert.equal(ok, false);
  assert.equal(state.planVersion, null);
  assert.equal(state.body, '');
});

test('MockupViewer renderFor drops stale HTML fetches after switching attachments', async () => {
  const firstFetch = deferred<string>();
  const state = componentFunctions(new URL('../src/modules/product/MockupViewer.svelte', import.meta.url), ['classify', 'revokeCreated', 'renderFor'], {
    gen: 0, createdUrls: [], renderedFor: null, rendering: false, renderError: null, liveError: null,
    imgUrl: null, htmlContent: null, mermaidSvg: null, mermaidSeq: 0,
    fetchText: () => firstFetch.promise,
    toasts: { error() {} },
  });
  const p = state.renderFor({ id: 'att-A', mime: 'text/html', filename: 'a.html' }, null);
  state.revokeCreated(); // simulates switching away before fetch resolves
  firstFetch.resolve('<h1>Stale A</h1>');
  await p;
  assert.equal(state.htmlContent, null);
});
