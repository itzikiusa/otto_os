import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function store() {
  return loadSource(new URL('../src/modules/threads/threadsStore.svelte.ts', import.meta.url), {
    '../../lib/api/client': { api: {} }, '../../lib/live': { onLive: () => () => {} },
    '../../lib/plural': { plural: () => '' }, '../../lib/router.svelte': { router: {} },
    '../../lib/stores/openfile.svelte': { openFile: {} }, '../../lib/stores/workspace.svelte': { ws: {} },
    '../../lib/toast.svelte': { toasts: {} }, '../../lib/toastError': { toastError: () => {} },
    './threadsChrome.svelte': { threadsChrome: { addSurface: () => {} } },
  }).threads;
}

test('compact inspector starts closed while preserving the desktop preference', () => {
  const threads = store();
  threads.inspectorOpen = true;
  threads.setInspectorCompact(true);
  assert.equal(threads.inspectorVisible, false);
  assert.equal(threads.inspectorOpen, true);
  // Explicit commands still open it when desktop has the same tab open.
  threads.setInspectorTab('files');
  assert.equal(threads.inspectorVisible, true);
  threads.toggleInspector('files');
  assert.equal(threads.inspectorVisible, false);
  assert.equal(threads.inspectorOpen, true);
  threads.setInspectorCompact(false);
  assert.equal(threads.inspectorVisible, true);
});

test('file links and side-chat requests open the mobile overlay without persisting it', () => {
  const threads = store();
  threads.setInspectorCompact(true);
  threads.openFileTab('ws', '/tmp/file.ts', { line: 7 });
  assert.equal(threads.inspectorVisible, true);
  assert.equal(threads.inspectorOpen, false);
  assert.equal(threads.inspectorTab, 'files');
  threads.setInspectorOpen(false);
  threads.openSideChat('A', 'what changed?');
  assert.equal(threads.inspectorVisible, true);
  assert.equal(threads.inspectorTab, 'side');
  assert.equal(threads.sideAsk.mainId, 'A');
  threads.setInspectorCompact(false);
  assert.equal(threads.inspectorVisible, false);
});
