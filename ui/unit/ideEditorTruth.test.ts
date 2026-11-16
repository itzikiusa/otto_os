// What the IDE editor shows while you edit and debug: breakpoints stay on the code they were
// set on (I2-02), say honestly whether they bound (I2-07), the stop vs the selected frame
// (I2-23), quiet inline values (I2-17), hover-to-evaluate (I2-13) and the notice when the
// file under your hands changes on disk (I1-12).
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';
import { history, undo } from '@codemirror/commands';
import { EditorSelection, EditorState, Transaction, type TransactionSpec } from '@codemirror/state';
import {
  applyBreakpointMoves, breakpointAppearance, mapBreakpointLines, pendingBreakpointMoves, sameBreakpointLines,
  selectedFrameMark, shouldMapBreakpoints, type BpLines,
} from '../src/lib/components/cm-ide-bp-model.ts';
import { outsideText } from '../src/lib/components/cm-outside-text.ts';
import { debugHoverExpressionAt } from '../src/lib/components/cm-ide-debug-hover.ts';
import { loadSource } from './sourceHarness.ts';

// Values built inside the vm realm compare structurally only after a JSON round trip.
const plain = <T>(v: T): T => JSON.parse(JSON.stringify(v)) as T;

type Bp = { path: string; line: number; enabled: boolean; condition?: string | null; log_message?: string | null; verified?: boolean; message?: string | null };
const DOC = ['def main():', '    order = {}', '    total = 0', '    for i in range(3):', '        total += i', '    print(total)', ''].join('\n');

function linesOf(list: Bp[]): BpLines<Bp> {
  const map = new Map(list.map((b) => [b.line, b]));
  return { map, origin: new Map([...map.keys()].map((l) => [l, l])), dropped: [] };
}
/** Applies one edit and maps breakpoints the way the gutter field does. */
function edit(state: EditorState, bps: BpLines<Bp>, spec: TransactionSpec) {
  const tr = state.update(spec);
  const mapped = shouldMapBreakpoints(tr) ? mapBreakpointLines(bps, tr.changes, tr.startState.doc, tr.state.doc) : null;
  return { state: tr.state, bps: mapped ?? bps };
}
const bp = (line: number, extra: Partial<Bp> = {}): Bp => ({ path: '/p/review.py', line, enabled: true, ...extra });

describe('breakpoints follow the code they were set on (I2-02)', () => {
  it('inserting two lines above moves both breakpoints down; the store rewrites them once', () => {
    const st = EditorState.create({ doc: DOC });
    const { bps } = edit(st, linesOf([bp(3), bp(5, { condition: 'i > 1' })]), { changes: { from: st.doc.line(2).from, insert: '    # a\n    # b\n' }, userEvent: 'input' });
    assert.deepEqual([...bps.map.keys()], [5, 7]);
    const moves = pendingBreakpointMoves(bps);
    assert.deepEqual(moves, [{ from: 3, to: 5 }, { from: 5, to: 7 }]);
    const stored = applyBreakpointMoves([bp(3, { verified: true }), bp(5, { condition: 'i > 1' })], moves);
    assert.deepEqual(stored.map((b) => [b.line, b.condition ?? null, 'verified' in b]), [[5, null, false], [7, 'i > 1', false]]);
  });

  it('deleting the line (text and its line break) drops its breakpoint; emptying it keeps it', () => {
    const st = EditorState.create({ doc: DOC });
    const l3 = st.doc.line(3);
    const gone = edit(st, linesOf([bp(3), bp(5)]), { changes: { from: l3.from, to: l3.to + 1 }, userEvent: 'delete.line' }).bps;
    assert.deepEqual([...gone.map.keys()], [4]);
    assert.deepEqual(pendingBreakpointMoves(gone), [{ from: 3, to: null, reason: 'deleted' }, { from: 5, to: 4 }]);
    assert.deepEqual(applyBreakpointMoves([bp(3), bp(5)], pendingBreakpointMoves(gone)).map((b) => b.line), [4]);
    const emptied = edit(st, linesOf([bp(3)]), { changes: { from: l3.from, to: l3.to }, userEvent: 'delete' }).bps;
    assert.deepEqual([...emptied.map.keys()], [3]);
  });

  it('two breakpoints never collapse onto one line silently: one is kept and the drop says "merged"', () => {
    const st = EditorState.create({ doc: DOC });
    // Backspace at the start of line 4 joins it into line 3.
    const joined = edit(st, linesOf([bp(3), bp(4, { log_message: 'i={i}' })]), { changes: { from: st.doc.line(3).to, to: st.doc.line(4).from }, userEvent: 'delete.backward' }).bps;
    assert.equal(joined.map.size, 1);
    assert.equal(joined.map.get(3)?.log_message, 'i={i}', 'the breakpoint carrying a log message wins over a plain one');
    const moves = pendingBreakpointMoves(joined);
    assert.ok(moves.some((m) => m.reason === 'merged' && m.from === 3 && m.into === 3));
    assert.deepEqual(applyBreakpointMoves([bp(3), bp(4, { log_message: 'i={i}' })], moves).map((b) => [b.line, b.log_message]), [[3, 'i={i}']]);
  });

  it('undo carries the breakpoint back with the text', () => {
    let st = EditorState.create({ doc: DOC, extensions: history() });
    let bps = linesOf([bp(5)]);
    ({ state: st, bps } = edit(st, bps, { changes: { from: 0, insert: 'import os\n\n' }, userEvent: 'input' }));
    assert.deepEqual([...bps.map.keys()], [7]);
    let undone: Transaction | null = null;
    undo({ state: st, dispatch: (tr) => { undone = tr; } });
    assert.ok(undone);
    const tr = undone as Transaction;
    assert.equal(shouldMapBreakpoints(tr), true);
    const back = mapBreakpointLines(bps, tr.changes, tr.startState.doc, tr.state.doc);
    assert.deepEqual([...(back?.map.keys() ?? [])], [5]);
    assert.deepEqual(pendingBreakpointMoves(back!), [], 'back on the line the store knows: nothing to report');
  });

  it('text from outside (disk reload, collab, the store) and whole-document replacement keep line numbers', () => {
    const st = EditorState.create({ doc: DOC });
    const remote = st.update({ changes: { from: 0, insert: 'x\n' }, annotations: Transaction.remote.of(true) });
    assert.equal(shouldMapBreakpoints(remote), false);
    // The store's own value sync uses `outsideText`, which (unlike `remote`) stays undoable.
    const outside = st.update({ changes: { from: 0, insert: 'x\n' }, annotations: outsideText.of(true) });
    assert.equal(shouldMapBreakpoints(outside), false);
    assert.equal(outside.annotation(Transaction.remote), undefined);
    const whole = st.update({ changes: { from: 0, to: st.doc.length, insert: 'new\n' + DOC }, userEvent: 'input.complete' });
    assert.equal(shouldMapBreakpoints(whole), false);
    assert.equal(shouldMapBreakpoints(st.update({ selection: EditorSelection.cursor(3) })), false);
  });

  it('no feedback loop: the store echoing the moved list is recognised as already shown', () => {
    const shown = new Map([[5, bp(5, { condition: 'x' })], [7, bp(7)]]);
    assert.equal(sameBreakpointLines(shown, [bp(5, { condition: 'x' }), bp(7)]), true);
    assert.equal(sameBreakpointLines(shown, [bp(5, { condition: 'y' }), bp(7)]), false);
    assert.equal(sameBreakpointLines(shown, [bp(5, { condition: 'x' }), bp(7, { verified: false })]), false, 'a binding answer is news');
  });

  it('the gutter field re-applies unreported moves, rebases after reporting, and flushes on clicks and teardown', () => {
    const src = readFileSync(new URL('../src/lib/components/cm-ide-extensions.ts', import.meta.url), 'utf8');
    assert.match(src, /if \(!pending\.length && sameBreakpointLines\(map, list\)\) continue;/);
    assert.match(src, /cb\(opts\.filePath, moves\);\s*view\.dispatch\(\{ effects: rebaseBreakpointsEffect\.of\(null\) \}\);/);
    assert.match(src, /setTimeout\(\(\) => \{ this\.moveTimer = null; this\.unreported = \[\]; reportMoves\(this\.view\); \}, 300\)/);
    assert.doesNotMatch(src, /closest\('\.cm-lineNumbers'\) && !bp/, 'the line-number column opens the breakpoint editor too (I2-21)');
  });
});

describe('breakpoints tell the truth about binding (I2-07)', () => {
  it('verified only from the breakpoint itself; unbound is hollow and says why; no answer is the plain dot', () => {
    assert.deepEqual(breakpointAppearance(bp(3), 3, false), { kind: 'enabled', tooltip: 'Breakpoint at line 3', verified: false });
    assert.deepEqual(breakpointAppearance(bp(3, { verified: true }), 3, false), { kind: 'enabled', tooltip: 'Breakpoint at line 3 (bound)', verified: true });
    const unbound = breakpointAppearance(bp(6, { verified: false, message: 'No code on this line' }), 6, false);
    assert.deepEqual(unbound, { kind: 'unbound', tooltip: 'Not bound: No code on this line', verified: false });
    assert.equal(breakpointAppearance(bp(6, { verified: false }), 6, true).kind, 'muted');
    assert.equal(breakpointAppearance(bp(6, { verified: true, condition: 'x' }), 6, false).kind, 'conditional');
    const src = readFileSync(new URL('../src/lib/components/cm-ide-extensions.ts', import.meta.url), 'utf8');
    assert.doesNotMatch(src, /pausedLine != null\) && !muted/, 'a pause no longer marks every breakpoint verified');
  });
});

describe('stop vs selected frame, inline values (I2-23, I2-17)', () => {
  const mod = loadSource(new URL('../src/modules/ide/editor/ideDebugEditorProps.ts', import.meta.url), {}, {}, { relative: true });
  const snap = (extra: Record<string, unknown> = {}) => ({
    state: 'paused', paused_file: '/p/review.py', paused_line: 11,
    inline_values: [
      { line: 11, name: 'total', value: '2' },
      { line: 5, name: 'build', value: '<function build at 0x10e2c>' },
      { line: 4, name: 'os', value: "<module 'os' from '/usr/lib/python3/os.py'>" },
      { line: 6, name: 'order', value: "{'id': 7, 'items': [1, 2, 3], 'meta': {'source': 'web', 'tags': ['a', 'b']}}" },
    ],
    ...extra,
  });
  it('older daemons (no selected_frame_*): the stop gets the arrow and the values', () => {
    const p = mod.ideDebugEditorProps(snap(), '/p/review.py', ['/p']);
    assert.equal(p.pausedLine, 11);
    assert.equal(p.selectedFrameLine, null);
    assert.deepEqual(plain(p.inlineValues.map((v: { name: string }) => v.name)), ['total', 'order']);
    assert.equal(mod.ideDebugEditorProps(snap(), '/p/other.py', ['/p']).pausedLine, null);
    assert.equal(mod.ideDebugEditorProps({ ...snap(), state: 'running' }, '/p/review.py', ['/p']).pausedLine, null);
  });
  it('a selected caller frame: the arrow stays on the stop, a muted mark on the frame, values follow the frame', () => {
    const same = mod.ideDebugEditorProps(snap({ selected_frame_file: '/p/review.py', selected_frame_line: 18 }), '/p/review.py', ['/p']);
    assert.deepEqual([same.pausedLine, same.selectedFrameLine], [11, 18]);
    const atStop = mod.ideDebugEditorProps(snap({ selected_frame_file: '/p/review.py', selected_frame_line: 11 }), '/p/review.py', ['/p']);
    assert.equal(atStop.selectedFrameLine, null, 'no second mark on the stop itself');
    const other = snap({ selected_frame_file: '/p/main.py', selected_frame_line: 3 });
    assert.deepEqual(plain(mod.ideDebugEditorProps(other, '/p/review.py', ['/p']).inlineValues), [], 'the caller file owns the values');
    const caller = mod.ideDebugEditorProps(other, '/p/main.py', ['/p']);
    assert.deepEqual([caller.pausedLine, caller.selectedFrameLine, caller.inlineValues.length], [null, 3, 2]);
    assert.equal(selectedFrameMark(11, 18), 18);
    assert.equal(selectedFrameMark(11, 11), null);
    assert.equal(selectedFrameMark(null, undefined), null);
  });
  it('inline values are capped at 40 characters and skip function/module/class reprs', () => {
    const out = mod.quietInlineValues([
      { line: 1, name: 'a', value: 'x'.repeat(60) },
      { line: 1, name: 'f', value: '<bound method A.run of <A>>' },
      { line: 1, name: 'C', value: "<class 'shop.Order'>" },
      { line: 1, name: 'fn', value: '[Function: handler]' },
      { line: 1, name: 'n', value: '42' },
    ]);
    assert.deepEqual(plain(out.map((v: { name: string; value: string }) => [v.name, v.value.length])), [['a', 40], ['n', 2]]);
    assert.ok(out[0].value.endsWith('…'));
  });
  it('hover evaluates only while paused in this file', async () => {
    const ev = async (e: string) => `v:${e}`;
    assert.equal(mod.ideDebugEditorProps(snap(), '/p/review.py', ['/p'], ev).onDebugHover, ev);
    assert.equal(mod.ideDebugEditorProps(snap(), '/p/other.py', ['/p'], ev).onDebugHover, null);
    assert.equal(mod.ideDebugEditorProps({ ...snap(), state: 'running' }, '/p/review.py', ['/p'], ev).onDebugHover, null);
  });
});

describe('hover-to-evaluate picks the expression under the pointer (I2-13)', () => {
  const st = EditorState.create({ doc: 'total += len(order.items)  # count order.items\nif x:' });
  it('identifiers extend over member access; keywords, numbers and comments are skipped', () => {
    assert.equal(debugHoverExpressionAt(st, 2)?.expr, 'total');
    assert.equal(debugHoverExpressionAt(st, st.doc.toString().indexOf('items'))?.expr, 'order.items');
    assert.equal(debugHoverExpressionAt(st, st.doc.toString().indexOf('order'))?.expr, 'order');
    assert.equal(debugHoverExpressionAt(st, st.doc.toString().lastIndexOf('items')), null, 'inside a comment');
    assert.equal(debugHoverExpressionAt(st, st.doc.line(2).from), null, '`if` is a keyword');
    const sel = st.update({ selection: EditorSelection.range(9, 25) }).state;
    assert.equal(debugHoverExpressionAt(sel, 12)?.expr, 'len(order.items)');
  });
});

describe('a file you are editing changes on disk (I1-12)', () => {
  const imports = {
    '../../../lib/api/client': { api: { bg: { get: async () => { throw new Error('unused'); } } } },
    '../shell/ideProjectHelpers': { findContainingRoot: () => null },
    '../../../lib/poll': { pollWhileVisible: () => ({ stop() {}, now() {} }) },
    './ideTypes': {
      normalizeEditorLf: (t: string) => t.replace(/\r\n?/g, '\n'),
      detectTextEol: (t: string) => (t.includes('\r\n') ? '\r\n' : '\n'),
      diskEolsOf: () => undefined,
    },
  };
  const mod = loadSource(new URL('../src/modules/ide/store/ideDiskChange.svelte.ts', import.meta.url), imports);
  const tab = () => ({ path: '/p/a.py', name: 'a.py', content: 'base\n', savedContent: 'base\n', sha256: 'S0', language: 'python', readOnly: false, truncated: false, loading: false, error: null, preview: false });
  function host(t = tab(), buffer = 'base\nmine\n') {
    return { tabs: [t], target: { kind: 'local', root: '/p' }, projectRoots: [], collabIsGuest: false, getBuffer: () => buffer, targetQuery: (x: Record<string, string>) => new URLSearchParams(x).toString() };
  }

  it('dirty tab + disk change -> notice (autosave waits); disk back to base -> notice gone', async () => {
    const w = new mod.IdeDiskChangeWatch();
    const h = host(), conflicts = new Set<string>();
    let disk = { content: 'theirs\n', sha256: 'S1' };
    const read = async () => ({ ...disk, path: '/p/a.py', size: 1, read_only: false, truncated: false, line_count: 1 });
    await w.check(h, conflicts, read);
    assert.equal(w.has('/p/a.py'), true);
    assert.equal(w.notices['/p/a.py'].diskContent, 'theirs\n');
    assert.equal(conflicts.has('/p/a.py'), true, 'autosave pauses while the banner is up');
    disk = { content: 'base\n', sha256: 'S0' };
    await w.check(h, conflicts, read);
    assert.equal(w.has('/p/a.py'), false);
    assert.equal(conflicts.has('/p/a.py'), false);
  });

  it('clean tabs are not watched (the git refresh reloads them silently)', async () => {
    const w = new mod.IdeDiskChangeWatch();
    let reads = 0;
    await w.check(host(tab(), 'base\n'), new Set(), async () => { reads++; return { content: 'x', sha256: 'S9' }; });
    assert.equal(reads, 0);
  });

  it('Keep mine: the disk copy becomes the base so the next save overwrites knowingly (no 409)', async () => {
    const w = new mod.IdeDiskChangeWatch();
    const t = tab(), h = host(t), conflicts = new Set<string>();
    await w.check(h, conflicts, async () => ({ content: 'theirs\r\n', sha256: 'S1' }));
    w.keepMine(t, conflicts);
    assert.deepEqual([t.sha256, t.savedContent, w.has(t.path), conflicts.has(t.path)], ['S1', 'theirs\n', false, false]);
    // Unchanged disk on the next check: no banner again.
    await w.check(h, conflicts, async () => ({ content: 'theirs\r\n', sha256: 'S1' }));
    assert.equal(w.has(t.path), false);
  });

  it('disk now holds exactly the unsaved text: no banner, and the sha is adopted', async () => {
    const w = new mod.IdeDiskChangeWatch();
    const t = tab();
    await w.check(host(t), new Set(), async () => ({ content: 'base\nmine\n', sha256: 'S2' }));
    assert.equal(w.has(t.path), false);
    assert.equal(t.sha256, 'S2');
  });
});
