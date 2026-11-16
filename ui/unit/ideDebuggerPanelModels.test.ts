// The debugger panel a person looks at during a session (round 5, I2-04/05/08/
// 10/11/12/14): console lines stitched from adapter chunks, evaluations in
// time order, Clear by sequence number at the output cap, variables that keep
// their expansion by path and mark changed values, the failure card's next
// step per kind, the re-attach rule and the one breakpoint list.
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  alignOffset,
  interleaveRepl,
  lastOutputSeq,
  OutputSeqTracker,
  stitchOutput,
  type OutputChunk,
} from '../src/modules/ide/debug/ideConsoleModel.ts';
import {
  chordAria,
  debugErrorCard,
  inferErrorKind,
  pickReattachSession,
  threadOptionLabel,
  withKeyHint,
} from '../src/modules/ide/debug/ideDebugPanelModel.ts';
import {
  collectValues,
  copyExpression,
  pathLabel,
  reexpandOpenPaths,
  REEXPAND_CAP,
  VarChangeTracker,
  varPath,
  type VarLike,
} from '../src/modules/ide/debug/ideVarTree.ts';
import { groupBreakpoints } from '../src/modules/ide/debug/ideBreakpointList.ts';

const out = (category: string, text: string, seq?: number): OutputChunk => ({ category, text, ...(seq ? { seq } : {}) });
const texts = (chunks: OutputChunk[]) => stitchOutput(chunks, chunks.map((c, i) => c.seq ?? i + 1)).map((l) => l.text);

describe('debug console lines (I2-04)', () => {
  it('stitches print("tick", i) chunks into one row per line', () => {
    const chunks = [out('stdout', 'tick'), out('stdout', ' '), out('stdout', '0'), out('stdout', '\n'), out('stdout', 'tick'), out('stdout', ' '), out('stdout', '1'), out('stdout', '\n')];
    assert.deepEqual(texts(chunks), ['tick 0', 'tick 1']);
  });

  it('keeps a trailing partial line open for the next chunk', () => {
    const first = [out('stdout', 'a\nb')];
    assert.deepEqual(texts(first), ['a', 'b']);
    assert.deepEqual(texts([...first, out('stdout', 'c\n')]), ['a', 'bc']);
  });

  it('a category change ends the open line; stderr stays its own red row', () => {
    const lines = stitchOutput([out('stdout', 'tick'), out('stderr', 'warn 0\n'), out('stdout', ' 0\n')], [1, 2, 3]);
    assert.deepEqual(lines.map((l) => [l.text, l.isError]), [['tick', false], ['warn 0', true], [' 0', false]]);
  });

  it('a bare newline is a blank line, an empty chunk is nothing; keys are unique', () => {
    const lines = stitchOutput([out('stdout', 'x\n'), out('stdout', ''), out('stdout', '\n'), out('stdout', 'y\nz\n')], [1, 2, 3, 4]);
    assert.deepEqual(lines.map((l) => l.text), ['x', '', 'y', 'z']);
    assert.equal(new Set(lines.map((l) => l.key)).size, lines.length);
    assert.deepEqual(lines.map((l) => [l.seq, l.endSeq]), [[1, 1], [3, 3], [4, 4], [4, 4]]);
  });
});

describe('sequence numbers and Clear at the cap (I2-12)', () => {
  it('uses the daemon seq when every chunk has one', () => {
    const t = new OutputSeqTracker();
    assert.deepEqual(t.next('s', [out('stdout', 'a', 41), out('stdout', 'b', 42)]), [41, 42]);
    assert.equal(lastOutputSeq([out('stdout', 'a', 41), out('stdout', 'b', 42)]), 42);
    assert.equal(lastOutputSeq([out('stdout', 'a')]), undefined);
  });

  it('without seq: a client counter follows the window when the daemon trims the front', () => {
    const t = new OutputSeqTracker();
    const w1 = ['l1\n', 'l2\n', 'l3\n'].map((s) => out('stdout', s));
    assert.deepEqual(t.next('s', w1), [1, 2, 3]);
    // At a cap of 3: one line in, one line out — the length does not change.
    const w2 = ['l2\n', 'l3\n', 'l4\n'].map((s) => out('stdout', s));
    assert.deepEqual(t.next('s', w2), [2, 3, 4]);
    assert.equal(alignOffset(w1, w2), 1);
    // A new session restarts the counter.
    assert.deepEqual(t.next('s2', w1), [1, 2, 3]);
  });

  it('Clear by sequence: output after a Clear at the cap is shown', () => {
    const t = new OutputSeqTracker();
    const w1 = ['l1\n', 'l2\n', 'l3\n'].map((s) => out('stdout', s));
    const seq1 = t.next('s', w1);
    const cleared = seq1[seq1.length - 1];
    const w2 = ['l2\n', 'l3\n', 'l4\n'].map((s) => out('stdout', s));
    const lines = stitchOutput(w2, t.next('s', w2));
    // The old index-based Clear kept `slice(3)` of a 3-long array: nothing, forever.
    assert.deepEqual(lines.filter((l) => l.endSeq > cleared).map((l) => l.text), ['l4']);
  });
});

describe('evaluations sit where they were made (I2-04)', () => {
  it('interleaves by afterSeq and keeps unplaced entries last', () => {
    const lines = [{ seq: 1 }, { seq: 2 }, { seq: 5 }];
    const repl = [{ id: 'a', afterSeq: 1 }, { id: 'b', afterSeq: 4 }, { id: 'c' }];
    const order = interleaveRepl(lines, repl).map((it) => (it.kind === 'line' ? `L${it.line.seq}` : it.entry.id));
    assert.deepEqual(order, ['L1', 'a', 'L2', 'b', 'L5', 'c']);
  });

  it('an evaluation made before any output comes first', () => {
    const order = interleaveRepl([{ seq: 3 }], [{ id: 'x', afterSeq: 0 }]).map((it) => it.kind);
    assert.deepEqual(order, ['repl', 'line']);
  });
});

describe('variables keep their place (I2-08)', () => {
  const vars: Record<number, VarLike[]> = {
    10: [{ name: 'order', value: "{'id': 0}", var_type: 'dict', variables_reference: 11 }, { name: 'total', value: '0', variables_reference: 0 }],
    11: [{ name: 'items', value: '[…]', var_type: 'list', variables_reference: 12 }],
    12: [{ name: '0', value: "{'sku': 'a'}", var_type: 'dict', variables_reference: 13 }],
  };

  it('paths are scope/name/name…', () => {
    assert.equal(pathLabel(varPath(varPath('Locals', 'order'), 'items')), 'Locals/order/items');
  });

  it('re-expands open paths breadth-first after a stop, loading only what is missing', async () => {
    const loaded: number[] = [];
    const cache: Record<number, VarLike[]> = {};
    const open = { [varPath('Locals', 'order')]: true, [varPath(varPath('Locals', 'order'), 'items')]: true };
    const n = await reexpandOpenPaths([{ name: 'Locals', variables_reference: 10 }], { Locals: true }, open, (r) => cache[r], async (r) => {
      loaded.push(r);
      cache[r] = vars[r];
      return vars[r];
    });
    assert.deepEqual(loaded, [10, 11, 12]);
    assert.equal(n, 3);
  });

  it('is capped', async () => {
    let calls = 0;
    const deep: Record<string, boolean> = {};
    let p = 'S';
    for (let i = 0; i < 40; i++) { p = varPath(p, 'n'); deep[p] = true; }
    await reexpandOpenPaths([{ name: 'S', variables_reference: 1 }], { S: true }, deep, () => undefined, async (r) => {
      calls++;
      return [{ name: 'n', value: String(r), variables_reference: r + 1 }];
    });
    assert.equal(calls, REEXPAND_CAP);
  });

  it('marks a value that changed since the previous stop; a new path is not "changed"', () => {
    const t = new VarChangeTracker();
    const known1 = (r: number) => vars[r];
    t.observe('stop1', collectValues([{ name: 'Locals', variables_reference: 10 }], { Locals: true }, {}, known1));
    t.observe('stop2', { [varPath('Locals', 'total')]: '2', [varPath('Locals', 'order')]: "{'id': 0}", [varPath('Locals', 'i')]: '1' });
    assert.equal(t.isChanged(varPath('Locals', 'total'), '2'), true);
    assert.equal(t.isChanged(varPath('Locals', 'order'), "{'id': 0}"), false);
    assert.equal(t.isChanged(varPath('Locals', 'i'), '1'), false);
  });

  it('copies a nested variable as an expression', () => {
    const chain = [vars[10][0], vars[11][0], vars[12][0]];
    assert.equal(copyExpression(chain), "order['items'][0]");
    assert.equal(copyExpression([{ name: 'cfg', value: '', var_type: 'Config', variables_reference: 1 }, { name: 'port', value: '80', variables_reference: 0 }]), 'cfg.port');
  });
});

describe('failure card (I2-10)', () => {
  it('adapter_missing → Install <adapter>', () => {
    const card = debugErrorCard({ message: "not found: Go debugger 'dlv' (Delve) not found", kind: 'adapter_missing', ecosystem: 'go' }, null);
    assert.equal(card?.primary?.id, 'install');
    assert.equal(card?.primary?.label, 'Install Delve');
    assert.equal(card?.ecosystem, 'go');
    assert.equal(card?.message.startsWith('Go debugger'), true, 'no raw error prefix');
  });

  it('toolchain_missing → Open SDKs…; build_failed → Show build output + Retry', () => {
    assert.equal(debugErrorCard(null, { state: 'error', error: 'no go SDK', error_kind: 'toolchain_missing' })?.primary?.id, 'sdks');
    const b = debugErrorCard(null, { state: 'error', error: 'exit 2', error_kind: 'build_failed' });
    assert.equal(b?.primary?.id, 'build-output');
    assert.equal(b?.canRetry, true);
  });

  it('an unknown failure still shows, with Retry and no invented action', () => {
    const c = debugErrorCard({ message: 'forbidden: nope', kind: null, ecosystem: null }, null);
    assert.equal(c?.primary, null);
    assert.equal(c?.canRetry, true);
    assert.equal(c?.message, 'nope');
    assert.equal(debugErrorCard(null, { state: 'running' }), null);
  });

  it('a daemon without error_kind: the missing-debugger text still offers the install', () => {
    const card = debugErrorCard(null, { state: 'error', language: 'go', error: "not found: Go debugger 'dlv' (Delve) not found; install it from IDE → Toolchains & SDKs" });
    assert.equal(card?.primary?.label, 'Install Delve');
    assert.deepEqual(inferErrorKind('go build failed: exit status 2', 'go'), { kind: 'build_failed', ecosystem: 'go' });
    assert.equal(inferErrorKind('connection refused', 'go'), null);
  });
});

describe('re-attach, threads, tooltips (I2-14, I2-05, I2-06)', () => {
  it('re-attaches only to a live session of this project', () => {
    const sessions = [
      { id: 'other', state: 'paused', target_root: '/ws/other' },
      { id: 'done', state: 'terminated', target_root: '/ws/me' },
      { id: 'mine', state: 'running', target_root: '/ws/me/' },
    ];
    assert.equal(pickReattachSession(sessions, '/ws/me')?.id, 'mine');
    assert.equal(pickReattachSession(sessions.slice(0, 2), '/ws/me'), null);
    assert.equal(pickReattachSession([{ id: 'old', state: 'paused' }], '/ws/me')?.id, 'old', 'no target_root: old daemon');
  });

  it('thread option text carries the active thread stop reason', () => {
    assert.equal(threadOptionLabel({ id: 1, name: 'main' }, { id: 1, reason: 'breakpoint' }), '#1 main · breakpoint');
    assert.equal(threadOptionLabel({ id: 2, name: 'worker' }, { id: 1, reason: 'breakpoint' }), '#2 worker');
  });

  it('tooltips and aria-keyshortcuts come from the chord table', () => {
    assert.equal(chordAria({ key: 'r', mod: true, alt: true }), 'Alt+Meta+R');
    assert.equal(chordAria({ key: 'F8', shift: true }), 'Shift+F8');
    assert.equal(withKeyHint('Step over', ['F8', '⌃⌥↓']), 'Step over (F8 / ⌃⌥↓)');
    assert.equal(withKeyHint('Run to cursor', []), 'Run to cursor');
  });
});

describe('one breakpoint list (I2-11)', () => {
  it('groups by file, sorts by line, previews loaded code and marks unbound ones', () => {
    const groups = groupBreakpoints(
      [
        { path: '/p/b.py', line: 11, enabled: true, condition: 'i > 1' },
        { path: '/p/a.py', line: 3, enabled: true },
        { path: '/p/b.py', line: 6, enabled: true, verified: false, message: 'No code on this line' },
      ],
      (p) => (p === '/p/b.py' ? Array.from({ length: 12 }, (_, i) => `    line_${i + 1} = ${i}`).join('\n') : null),
    );
    assert.deepEqual(groups.map((g) => g.path), ['/p/a.py', '/p/b.py']);
    assert.deepEqual(groups[1].rows.map((r) => r.bp.line), [6, 11]);
    assert.equal(groups[1].rows[1].preview, 'line_11 = 10');
    assert.deepEqual(groups[1].rows[1].chip, { kind: 'condition', label: 'if i > 1' });
    assert.equal(groups[1].rows[0].unbound, true);
    assert.equal(groups[0].rows[0].preview, '', 'file not loaded: just the line');
  });
});
