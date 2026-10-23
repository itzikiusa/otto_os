// P09 (quality iteration 6): Product module regressions — the analysis run
// area's inline load state (ux-design-6-03), the live Mermaid preview that
// keeps its last good diagram (ui-correctness-6-03), keyboard reorder of test
// cases (accessibility-responsive-6-04) and the inline load errors that never
// read as "nothing yet" (ux-design-6-04). The Mermaid race/debounce and the
// reorder queue + focus are driven as behaviour (test-quality-6w1-06,
// accessibility-responsive-6w1-02), not matched as source text.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { componentFunctions } from './componentFunctions.ts';
import { deferred as sourceDeferred, loadSource } from './sourceHarness.ts';

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

// ── MockupViewer: the live Mermaid preview (driven, not regex-matched) ──────

const deferred = sourceDeferred;
const flush = () => new Promise((r) => setImmediate(r));
const MERMAID = { id: 'm1', mime: 'text/vnd.mermaid', filename: 'flow.mmd' };

function mockupViewer(extra: Record<string, any> = {}, names = ['classify', 'revokeCreated', 'renderFor', 'scheduleRender']) {
  return componentFunctions(new URL('../src/modules/product/MockupViewer.svelte', import.meta.url), names, {
      Date, gen: 0, createdUrls: [], mermaidSeq: 0, renderedFor: null, LIVE_DEBOUNCE_MS: 200,
      rendering: false, renderError: null, liveError: null, imgUrl: null, htmlContent: null, mermaidSvg: null,
      toasts: { error() {} }, ...extra,
    });
}

test('a Mermaid render that resolves late never overwrites a newer one, and rendering ends', async () => {
  const pending: { text: string; d: ReturnType<typeof deferred<{ svg: string }>> }[] = [];
  const v = mockupViewer({
    renderMermaid(_id: string, text: string) {
      const d = deferred<{ svg: string }>();
      pending.push({ text, d });
      return d.promise;
    },
  });
  const first = v.renderFor(MERMAID, 'graph A');
  const second = v.renderFor(MERMAID, 'graph B');
  assert.equal(v.rendering, true);
  // The SECOND render lands first, then the stale first one.
  pending[1].d.resolve({ svg: '<svg>B</svg>' });
  await second;
  pending[0].d.resolve({ svg: '<svg>A</svg>' });
  await first;
  assert.equal(v.mermaidSvg, '<svg>B</svg>', 'the newer diagram is kept');
  assert.equal(v.rendering, false);

  // A live edit of the diagram on screen keeps the last good SVG meanwhile.
  const live = v.renderFor(MERMAID, 'graph C');
  assert.equal(v.mermaidSvg, '<svg>B</svg>', 'no blanking while the edit renders');
  assert.equal(v.rendering, false, 'no "Rendering…" for a live edit');
  pending[2].d.resolve({ svg: '<svg>C</svg>' });
  await live;
  assert.equal(v.mermaidSvg, '<svg>C</svg>');
});

/** A manual clock for the component's setTimeout/clearTimeout. */
function fakeClock() {
  let now = 0;
  let seq = 0;
  const timers = new Map<number, { at: number; fn: () => void }>();
  return {
    setTimeout(fn: () => void, ms: number) { timers.set(++seq, { at: now + ms, fn }); return seq; },
    clearTimeout(id: number) { timers.delete(id); },
    tick(ms: number) {
      now += ms;
      for (const [id, t] of [...timers]) if (t.at <= now) { timers.delete(id); t.fn(); }
    },
  };
}

test('two live edits inside the debounce window make exactly one render', () => {
  const clock = fakeClock();
  const renders: string[] = [];
  const v = mockupViewer({
    renderedFor: 'm1', setTimeout: clock.setTimeout, clearTimeout: clock.clearTimeout,
    renderFor: (_a: unknown, src: string) => { renders.push(src); },
  }, ['scheduleRender']);
  // Svelte runs the previous effect's cleanup before re-running it.
  const cleanup = v.scheduleRender(MERMAID, 'graph A');
  clock.tick(150);
  cleanup();
  v.scheduleRender(MERMAID, 'graph AB');
  clock.tick(199);
  assert.deepEqual(renders, [], 'nothing renders inside the window');
  clock.tick(1);
  assert.deepEqual(renders, ['graph AB']);
  // A first render (nothing of this attachment on screen) is immediate.
  v.renderedFor = null;
  assert.equal(v.scheduleRender(MERMAID, 'graph D'), undefined);
  assert.deepEqual(renders, ['graph AB', 'graph D']);
});

// ── TestCasesTab reorder (product/caseOrder.svelte.ts): queued saves, focus ──

function caseOrder(ids: string[]) {
  const saves: string[][] = [];
  const inflight: ReturnType<typeof deferred<void>>[] = [];
  const focused: string[] = [];
  const announced: string[] = [];
  // Each card exposes its Move buttons and checkbox; a Move button at the
  // list edge is disabled (as the re-rendered markup makes it).
  const ctl = (name: string, disabled = () => false) => ({ focus() { focused.push(name); }, get disabled() { return disabled(); } });
  const mod = loadSource(new URL('../src/modules/product/caseOrder.svelte.ts', import.meta.url), {
    svelte: { tick: async () => {} },
    '../../lib/announce.svelte': { announce(m: string) { announced.push(m); } },
    '../../lib/toastError': { toastError() {} },
  }, {
    CSS: { escape: (s: string) => s },
    document: {
      activeElement: { dataset: {} },
      querySelector(sel: string) {
        const id = /data-case-id="([^"]+)"/.exec(sel)![1];
        const order = () => state.ids as string[];
        return { querySelector(q: string) {
          if (q === '[data-move="up"]') return ctl(`${id}:up`, () => order()[0] === id);
          if (q === '[data-move="down"]') return ctl(`${id}:down`, () => order()[order().length - 1] === id);
          return ctl(`${id}:checkbox`);
        } };
      },
    },
  });
  const state = new mod.CaseOrder({ run: () => ({ id: 'run1' }), save(_run: string, order: string[]) {
    saves.push([...order]);
    const d = deferred<void>();
    inflight.push(d);
    return d.promise;
  } });
  state.ids = [...ids];
  return { state, saves, inflight, focused, announced };
}

test('a Move persists the swapped order and keeps focus on the pressed button mid-list', async () => {
  const { state, saves, inflight, focused, announced } = caseOrder(['a', 'b', 'c', 'd']);
  await state.move('b', 1, ['a', 'b', 'c', 'd'], 'down');
  assert.deepEqual(saves, [['a', 'c', 'b', 'd']]);
  assert.deepEqual(focused, ['b:down'], 'WebKit does not focus a clicked button; the pressed one is kept');
  assert.deepEqual(announced, ['Moved to position 3 of 4']);
  inflight[0].resolve();
  await flush();
  assert.equal(state.saving, false);
});

test('Move buttons stay live during a save: every move persists, the last save has the final order', async () => {
  const { state, saves, inflight, focused } = caseOrder(['a', 'b', 'c', 'd']);
  const group = ['a', 'b', 'c', 'd'];
  await state.move('b', 1, group, 'down'); // save 1 in flight
  assert.equal(state.saving, true);
  await state.move('b', 1, ['a', 'c', 'b', 'd'], 'down'); // queued
  // ⌥↑ from the card's checkbox is queued the same way.
  let prevented = false;
  state.onKey({ altKey: true, key: 'ArrowUp', preventDefault() { prevented = true; }, currentTarget: { dataset: {} } },
    'c', ['a', 'c', 'd', 'b']);
  await flush();
  assert.equal(prevented, true);
  assert.equal(saves.length, 1, 'one save at a time');
  assert.deepEqual([...state.ids], ['c', 'a', 'd', 'b']);
  inflight[0].resolve();
  await flush();
  assert.deepEqual(saves, [['a', 'c', 'b', 'd'], ['c', 'a', 'd', 'b']], 'the queued moves are saved with the final order');
  inflight[1].resolve();
  await flush();
  assert.equal(saves.length, 2, 'nothing left queued');
  assert.equal(state.saving, false);
  // b reached the bottom: its Down button went disabled, so focus fell back
  // to the checkbox; the ⌥↑ from c's checkbox stays on that checkbox.
  assert.deepEqual(focused, ['b:down', 'b:checkbox', 'c:checkbox']);
});

test('test cases have labelled Move buttons; the drag glyph is hidden from screen readers', () => {
  const s = src('TestCasesTab.svelte');
  assert.match(s, /class="drag-handle" aria-hidden="true"/);
  assert.match(s, /aria-label="Move \{tc\.title\} up"/);
  assert.match(s, /aria-label="Move \{tc\.title\} down"/);
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
