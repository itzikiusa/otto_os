import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';

const panelPath = new URL('../src/modules/canvas/ConversationPanel.svelte', import.meta.url);
const editorPath = (name: string) => new URL(`../src/modules/canvas/${name}Canvas.svelte`, import.meta.url);

for (const name of ['Excalidraw', 'Mermaid', 'D2']) {
  test(`${name} assist failure retains the exact composer prompt`, async () => {
    const canvas = { currentId: 'scene-A', assist: async () => { throw new Error('Provider unavailable'); }, pushConvo() {} };
    const editor = componentFunctions(editorPath(name), ['generate'], {
      canvas, sceneId: 'scene-A', saveContext: undefined, generating: false, userAdjusted: false,
      toastAgentEdit() {}, toastError() {}, toasts: { info() {}, success() {} },
    });
    const panel = componentFunctions(panelPath, ['send'], {
      canvas, editor, busy: false, draft: '  Draw the login flow\nwith retries  ', toastError() {},
    });
    await panel.send();
    assert.equal(panel.draft, '  Draw the login flow\nwith retries  ');
    assert.equal(panel.busy, false);
  });

  test(`${name} reports accepted content and rejects an empty generation result`, async () => {
    let response: Record<string, unknown> = {};
    let applied = 0;
    const canvas = { currentId: 'scene-A', saveContext: 'visit-A', assist: async () => response,
      pushConvo() {}, ingestDoc() { applied++; }, refreshSession: async () => {} };
    const editor = componentFunctions(editorPath(name), ['generate'], {
      canvas, sceneId: 'scene-A', saveContext: 'visit-A', generating: false, userAdjusted: false, sketch: false,
      toastAgentEdit() {}, toastError() {}, toasts: { info() {}, success() {} },
    });
    assert.equal(await editor.generate('Draw a flow'), false);
    assert.equal(applied, 0);
    response = name === 'Excalidraw' ? { excalidraw: { elements: [] } }
      : name === 'Mermaid' ? { mermaid: 'graph LR\nA-->B' } : { d2: 'A -> B' };
    assert.equal(await editor.generate('Draw a flow'), true);
    assert.equal(applied, 1);
    editor.generating = true;
    assert.equal(await editor.generate('Another flow'), false);
    assert.equal(applied, 1);
  });
}

test('Canvas keeps an unaccepted prompt while the request is pending', async () => {
  const request = deferred<boolean>();
  const panel = componentFunctions(panelPath, ['send'], {
    canvas: { currentId: 'A' }, editor: { generate: () => request.promise }, busy: false, draft: 'Draw a queue',
  });
  const pending = panel.send();
  const whilePending = panel.draft;
  request.resolve(true); await pending;
  assert.equal(whilePending, 'Draw a queue');
  assert.equal(panel.draft, '', 'an accepted request clears its own unchanged draft');
});

test('late accepted generation cannot clear another scene or a newer prompt', async () => {
  const request = deferred<boolean>();
  const canvas = { currentId: 'A' };
  const panel = componentFunctions(panelPath, ['send'], {
    canvas, editor: { generate: () => request.promise }, busy: false, draft: 'Draw A',
  });
  const pending = panel.send();
  canvas.currentId = 'B'; panel.draft = 'Draw B';
  request.resolve(true); await pending;
  assert.equal(panel.draft, 'Draw B');
});

test('Excalidraw restore applies persisted background and grid before the next save', () => {
  let elements: unknown[] = [];
  let appState: Record<string, unknown> = { viewBackgroundColor: '#ffffff', gridSize: 20, gridModeEnabled: false, scrollX: 99 };
  const api = {
    updateScene(patch: { elements: unknown[]; appState?: Record<string, unknown> }) {
      elements = patch.elements; appState = { ...appState, ...patch.appState };
    },
    scrollToContent() {}, getAppState: () => appState, getSceneElements: () => elements, getFiles: () => ({}),
  };
  const editor = componentFunctions(editorPath('Excalidraw'), ['loadScene', 'snapshotDoc', 'persistedAppState'], {
    liveApi: api, excaliApi: api, suppressSave: false, lastApplied: '',
    normalizeScene: (raw: { elements: unknown[] } | null) => raw?.elements ?? [],
    knownFiles: new Map(), inlineOf: new WeakMap(), filesForSave: () => ({ files: {}, inline: [] }),
    setTimeout() {},
  });
  editor.loadScene(JSON.stringify({ elements: [{ id: 'restored' }], appState: { viewBackgroundColor: '#123456', gridSize: 40, gridModeEnabled: true, scrollX: 1000 } }));
  const saved = JSON.parse(editor.snapshotDoc().source);
  assert.equal(saved.appState.viewBackgroundColor, '#123456');
  assert.equal(saved.appState.gridSize, 40);
  assert.equal(saved.appState.gridModeEnabled, true, 'restoring a grid also restores its enabled state');
  assert.equal(appState.scrollX, 99, 'transient viewport state is not restored');
  assert.equal(saved.elements[0].id, 'restored');
  editor.loadScene('malformed JSON');
  const defaults = JSON.parse(editor.snapshotDoc().source).appState;
  assert.equal(defaults.viewBackgroundColor, '#ffffff');
  assert.equal(defaults.gridSize, 20);
  assert.equal(defaults.gridModeEnabled, false);
  editor.loadScene(JSON.stringify({elements: [], appState: {gridSize: null}}));
  assert.equal(appState.gridSize, 20, 'legacy null grid sizes normalize to the installed numeric API');
  assert.equal(appState.gridModeEnabled, false);
  editor.loadScene(JSON.stringify({elements: [], appState: {gridSize: 40, gridModeEnabled: false}}));
  assert.equal(JSON.parse(editor.snapshotDoc().source).appState.gridModeEnabled, false);
});

for (const change of [{gridModeEnabled: true}, {gridSize: 40}]) {
  test(`Excalidraw grid-only edit ${Object.keys(change)[0]} schedules a persisted snapshot`, () => {
    let appState = {viewBackgroundColor: '#ffffff', gridSize: 20, gridModeEnabled: false, scrollX: 0};
    const timers: (() => void)[] = [];
    const staged: any[] = [];
    const canvas = {saveContext: 'visit-A', currentId: 'A', dirty: false, stageDoc: (_id: string, doc: any) => staged.push(doc)};
    const editor = componentFunctions(editorPath('Excalidraw'), ['onSceneChange', 'commitPending', 'snapshotDoc'], {
      sceneVersionOf: () => 1, lastFingerprint: null, canvas, saveContext: 'visit-A', sceneId: 'A',
      readonly: false, suppressSave: false, changePending: false, saveTimer: null, pendingDoc: null,
      setTimeout: (fn: () => void) => {timers.push(fn); return timers.length;}, clearTimeout() {}, saveNow() {},
      excaliApi: {getAppState: () => appState, getSceneElements: () => [], getFiles: () => ({})},
      filesForSave: () => ({files: {}, inline: []}), knownFiles: new Map(), inlineOf: new WeakMap(),
    });
    editor.onSceneChange([], appState, {});
    appState = {...appState, scrollX: 100};
    editor.onSceneChange([], appState, {});
    assert.equal(canvas.dirty, false, 'viewport movement remains a non-edit');
    assert.equal(timers.length, 0);
    appState = {...appState, ...change};
    editor.onSceneChange([], appState, {});
    assert.equal(canvas.dirty, true);
    assert.equal(timers.length, 1);
    timers[0]();
    assert.equal(staged.length, 1);
    const saved = JSON.parse(staged[0].source).appState;
    assert.equal(saved.gridModeEnabled, appState.gridModeEnabled);
    assert.equal(saved.gridSize, appState.gridSize);
  });
}
