import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import { setTraceSink, trace } from '../src/lib/composerTrace.ts';
import { composerViewState } from '../src/lib/stores/runtimeDraftCursor.ts';

// lib/composerTrace.ts + lib/composerTraceRecorder.ts: the phone composer's
// on-device trace. It is bounded, and the typed text never lands in it.

const rec = loadSource(new URL('../src/lib/composerTraceRecorder.ts', import.meta.url), {
  './composerTrace': { setTraceSink: () => {}, toggleComposerTrace: () => {} },
  './viewportFit': { onKeyboard: () => () => {} },
  './api/client': { api: {} },
  './clipboard': { copyWithToast: async () => true },
  './toast.svelte': { toasts: {} },
  './toastError': { toastError: () => 0 },
});

const SECRET = 'i also saw some issue on mobile';

test('the ring keeps only the newest entries, oldest first, and counts what it dropped', () => {
  const ring = new rec.TraceRing(3);
  for (let i = 0; i < 5; i++) ring.push(rec.entry('e', { i }, i));
  assert.deepEqual(Array.from(ring.entries(), (e: { i: number }) => e.i), [2, 3, 4]);
  assert.equal(ring.size, 3);
  assert.equal(ring.dropped, 2);
  ring.push(rec.entry('e', { i: 5 }, 5));
  assert.deepEqual(Array.from(ring.entries(), (e: { i: number }) => e.i), [3, 4, 5]);
  ring.clear();
  assert.equal(ring.size, 0);
  const json = JSON.parse(rec.traceJson(ring, { v: 1 }));
  assert.deepEqual(json, { meta: { v: 1, dropped: 0 }, entries: [] });
});

test('typing is recorded as its input type, lengths and caret, never the text', () => {
  const ring = new rec.TraceRing();
  const field = { value: SECRET, selectionStart: 1, selectionEnd: 1 };
  ring.push(rec.entry('beforeinput', rec.textFacts({ inputType: 'insertCompositionText', isComposing: true, data: SECRET.slice(1) }, field), 1));
  const [e] = ring.entries();
  assert.deepEqual({ ...e }, { t: 1, k: 'beforeinput', type: 'insertCompositionText', composing: true, len: SECRET.length - 1, s: 1, e: 1, vlen: SECRET.length });
  assert.ok(!rec.traceJson(ring, {}).includes('also saw'));
});

test('an element is described by its tag, classes and place, never its text', () => {
  const el = {
    tagName: 'TEXTAREA',
    value: SECRET,
    textContent: SECRET,
    getAttribute: () => 'rt-input svelte-x1y2 a b c',
    closest: (sel: string) => (sel.includes('.rt-composer') ? {} : null),
  };
  const e = rec.entry('focusin', { at: el, why: 'x'.repeat(200) }, 0);
  assert.equal(e.at, 'textarea.rt-input.a.b [composer,textarea]');
  assert.equal(e.why.length, 80, 'strings are capped');
});

test('the caret action reports its moves through trace() without the draft', async () => {
  const seen: unknown[] = [];
  trace('before');
  setTraceSink((k, data) => seen.push({ k, ...data }));
  try {
    class Field extends EventTarget {
      value = SECRET;
      selectionStart = 0;
      selectionEnd = 0;
      selectionDirection = 'none';
      scrollTop = 0;
      clientHeight = 44;
      isConnected = true;
      setSelectionRange(s: number, e: number): void {
        this.selectionStart = s;
        this.selectionEnd = e;
      }
    }
    const node = new Field();
    const { saveComposerCursorState } = await import('../src/lib/stores/runtimeDraftCursor.ts');
    saveComposerCursorState('trace-key', { selectionStart: 4, selectionEnd: 4 });
    const action = composerViewState(node as unknown as HTMLTextAreaElement, { key: 'trace-key', hasText: true });
    action.destroy();
  } finally {
    setTraceSink(null);
  }
  const kinds = seen.map((s) => (s as { k: string }).k);
  assert.ok(!kinds.includes('before'), 'nothing is recorded while the trace is off');
  assert.deepEqual(kinds, ['caret-mount', 'caret-restore', 'caret-destroy']);
  assert.deepEqual(seen[1], { k: 'caret-restore', why: 'mount', from: '0,0', to: '4,4', vlen: SECRET.length });
  assert.ok(!JSON.stringify(seen).includes('also saw'));
});

test('a programmatic write names its callers by function and file, past the recorder itself', () => {
  const stack = [
    'Error',
    '    at HTMLTextAreaElement.set (http://x/src/lib/composerTraceRecorder.ts?t=1:250:9)',
    '    at applyRestore (http://x/src/lib/stores/runtimeDraftCursor.ts?t=2:172:7)',
    '    at http://x/src/modules/threads/runtime/RuntimeThread.svelte:419:3',
  ].join('\n');
  assert.equal(rec.callers(stack), 'applyRestore@runtimeDraftCursor.ts:172 < ?@RuntimeThread.svelte:419');
  // WebKit's format: fn@url:line:col
  assert.equal(rec.callers('set@http://x/a/composerTraceRecorder.ts:1:1\nfocusComposer@http://x/a/RuntimeThread.svelte:88:4'), 'focusComposer@RuntimeThread.svelte:88');
});
