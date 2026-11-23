import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';

test('TestCasesTab publishTests ignores completion when story selection changes mid-publish', async () => {
  const req = deferred<void>();
  const toastsCalled: string[] = [];
  const product = {
    selectedId: 'story-A',
    testcaseRuns: [{ run: { id: 'run-1', confluence_url: 'https://wiki.test/1' } }],
    captureSelection() {
      const id = this.selectedId;
      return () => this.selectedId === id;
    },
    publishTests: () => req.promise,
  };
  const state = componentFunctions(new URL('../src/modules/product/TestCasesTab.svelte', import.meta.url), ['publishTests'], {
    product,
    activeRun: { id: 'run-1', confluence_url: null },
    publishingRun: false,
    publishSpaceKey: 'ENG',
    publishParentId: null,
    showPublishForm: true,
    toasts: { success(msg: string) { toastsCalled.push(msg); } },
    toastError() {},
  });
  const pending = state.publishTests();
  product.selectedId = 'story-B';
  req.resolve();
  await pending;
  assert.equal(state.showPublishForm, true);
  assert.equal(toastsCalled.length, 0);
});

test('BrandEditor refreshMeta and loadUsage drop stale responses after kit switch or unmount', async () => {
  const metaReq = deferred<{ artifact: { id: string }; head: { id: string }; approved: null }>();
  const usageReq = deferred<{ artifact_count: number }>();
  const state = componentFunctions(new URL('../src/modules/design-hall/brand/BrandEditor.svelte', import.meta.url), ['refreshMeta', 'loadUsage'], {
    id: 'kit-A', alive: true, metaSeq: 0, usageSeq: 0,
    artifact: null, head: null, approved: null,
    usage: null, usageLoading: false, usageError: null,
    api: { getArtifact: () => metaReq.promise },
    brandImpact: () => usageReq.promise,
    errText: String,
  });
  const p1 = state.refreshMeta();
  const p2 = state.loadUsage();
  state.id = 'kit-B';
  metaReq.resolve({ artifact: { id: 'kit-A' }, head: { id: 'v1' }, approved: null });
  usageReq.resolve({ artifact_count: 9 });
  await Promise.all([p1, p2]);
  assert.equal(state.artifact, null);
  assert.equal(state.usage, null);
});

test('SceneList clearFilter empties filter and restores focus to filter input', () => {
  let focused = false;
  const state = componentFunctions(new URL('../src/modules/canvas/SceneList.svelte', import.meta.url), ['clearFilter'], {
    filter: 'arch',
    filterEl: { focus() { focused = true; } },
  });
  state.clearFilter();
  assert.equal(state.filter, '');
  assert.equal(focused, true);
});

test('RightPanel jumpToHeading exits edit mode before scrolling to heading', async () => {
  let scrolled = false;
  const viewCalls: boolean[] = [];
  const state = componentFunctions(new URL('../src/modules/vault/RightPanel.svelte', import.meta.url), ['jumpToHeading'], {
    vault: {
      editing: true,
      setView(v: boolean) {
        viewCalls.push(v);
        this.editing = v;
      },
    },
    tick: async () => {},
    slugifyHeading: (s: string) => s.toLowerCase().replace(/\s+/g, '-'),
    scrollBehavior: () => 'auto',
    CSS: { escape: (s: string) => s },
    document: {
      querySelector: (sel: string) => (sel === '.read #h-overview' ? { scrollIntoView() { scrolled = true; } } : null),
    },
    requestAnimationFrame: (cb: () => void) => cb(),
  });
  await state.jumpToHeading('Overview');
  assert.deepEqual(viewCalls, [false]);
  assert.equal(scrolled, true);
});
