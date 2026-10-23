// P09 (quality iteration 6): Product module regressions — the analysis run
// area's inline load state (ux-design-6-03), the live Mermaid preview that
// keeps its last good diagram (ui-correctness-6-03), keyboard reorder of test
// cases (accessibility-responsive-6-04) and the inline load errors that never
// read as "nothing yet" (ux-design-6-04).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { componentFunctions } from './componentFunctions.ts';

const src = (file: string) => readFileSync(new URL(`../src/modules/product/${file}`, import.meta.url), 'utf8');

function analysisTab(getAnalysis: (id: string) => Promise<any>) {
  const product = { selectedId: 'story', getAnalysis, captureSelection() {
    const id = this.selectedId; return () => this.selectedId === id;
  } };
  const toasts: string[] = [];
  const state = componentFunctions(new URL('../src/modules/product/AnalysisTab.svelte', import.meta.url),
    ['selectHistory', 'selectRun', 'retrySelect', 'clearPoll', 'isTerminal', 'startPolling'], {
      activeId: null, activeDetail: null, detailLoading: false, detailError: null,
      viewGeneration: 0, readSequence: 0, pollTimer: null, polling: false, pollStartedAt: 0,
      pollFailures: 0, lastPollError: null, pollStopped: null,
      loadErrorText: (e: Error) => e.message, product,
      toastError: (title: string) => { toasts.push(title); },
      liveQuery() { return { stop() {} }; },
    });
  return { state, toasts };
}

test('a past run that fails to load shows inline (no toast) and re-picking it retries', async () => {
  let calls = 0;
  const { state, toasts } = analysisTab(async (id) => {
    calls++;
    if (calls === 1) throw new Error('daemon unreachable');
    return { analysis: { id, status: 'done' } };
  });
  await state.selectHistory({ id: 'R1' });
  assert.equal(state.detailError, 'daemon unreachable');
  assert.equal(state.detailLoading, false);
  assert.equal(state.activeDetail, null);
  assert.deepEqual(toasts, [], 'a failed load is inline, not a toast');

  // Re-picking the same run (the <select> can't fire a change for it, so the
  // inline Retry calls retrySelect) loads it again.
  state.retrySelect();
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(calls, 2);
  assert.equal(state.detailError, null);
  assert.equal((state.activeDetail as { analysis: { id: string } } | null)?.analysis.id, 'R1');

  // Once loaded, re-picking the same run is a no-op.
  await state.selectHistory({ id: 'R1' });
  assert.equal(calls, 2);
});

test('the run area shows the detail load state before the "Run analysis" invitation', () => {
  const s = src('AnalysisTab.svelte');
  const loadState = s.indexOf('<LoadState what="this analysis run"');
  const invite = s.indexOf('Configure lenses above and click Run analysis.');
  assert.ok(loadState > 0 && loadState < invite);
});

test('live Mermaid edits keep the last good SVG and guard stale renders', () => {
  const s = src('MockupViewer.svelte');
  assert.match(s, /renderMermaid\(id, text, \{ isStale: \(\) => myGen !== gen \}\)/);
  assert.match(s, /finally \{\s*if \(myGen === gen\) rendering = false;/);
  assert.match(s, /catch \(e\) \{\s*if \(myGen !== gen\) return;/);
  // Live path: no blanking ("Rendering…") while the new source renders, and debounced.
  assert.match(s, /const live = src !== null && renderedFor === a\.id/);
  assert.match(s, /if \(!live\) \{\s*rendering = true;/);
  assert.match(s, /setTimeout\(\(\) => void renderFor\(a, src\), LIVE_DEBOUNCE_MS\)/);
});

test('test cases reorder without a mouse; the drag glyph is hidden from screen readers', () => {
  const s = src('TestCasesTab.svelte');
  assert.match(s, /class="drag-handle" aria-hidden="true"/);
  assert.match(s, /aria-label="Move \{tc\.title\} up"/);
  assert.match(s, /aria-label="Move \{tc\.title\} down"/);
  assert.match(s, /e\.key === 'ArrowUp' \? -1 : 1/);
  assert.match(s, /async function moveCase[\s\S]*?void persistOrder\(\);[\s\S]*?\.focus\(\)/);
});

test('annotations and the canvas picker show failed loads inline, never as "nothing yet"', () => {
  const notes = src('MockupAnnotations.svelte');
  assert.doesNotMatch(notes, /toastError\('Couldn’t load annotations'/);
  assert.match(notes, /<LoadState what="annotations" variant="compact" error=\{loadError\}/);
  const canvases = src('LinkedCanvases.svelte');
  assert.doesNotMatch(canvases, /toastError\('Couldn’t load your canvases'/);
  assert.match(canvases, /<LoadState what="your canvases"/);
  assert.match(canvases, /from '\.\.\/\.\.\/lib\/canvas\/types'/);
});

test('the Jira multi-select is a named group', () => {
  const s = src('jira/IssueEditor.svelte');
  assert.match(s, /class="field-multiselect" role="group" aria-label=\{ef\.name\}/);
});

test('lib stores no longer import from modules/', () => {
  for (const f of ['product.svelte.ts', 'mockup-assist.svelte.ts']) {
    const s = readFileSync(new URL(`../src/lib/stores/${f}`, import.meta.url), 'utf8');
    assert.doesNotMatch(s, /from '\.\.\/\.\.\/modules\//, f);
  }
});
