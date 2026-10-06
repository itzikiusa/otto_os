// S14-302: weights are stored per grid shape, but one shape covers several
// tile counts (3×2 = 5 or 6 tiles). Edits must start from the RENDERED view.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { liveTracks, normalize } from '../src/modules/agents/tileTracks.ts';

/** colPair(r, k).apply as TiledView builds it: the pair's total is read from
 *  the rendered weights, the split written into the working copy. */
function dragCol(t: { cols: number[][] }, rendered: number[][], r: number, k: number, share: number): void {
  const total = rendered[r][k - 1] + rendered[r][k];
  t.cols[r][k - 1] = share * total;
  t.cols[r][k] = total - share * total;
}

test('a divider in a last row that shrank from 3 tiles to 2 still moves', () => {
  // Saved with 6 tiles (3 + 3); now 5 tiles (3 + 2), same 3×2 shape.
  const stored = { rows: [1, 1], cols: [[1, 1, 1], [2, 1, 1]] };
  const lens = [3, 2];
  const before = liveTracks(stored, lens);
  assert.deepEqual(before.cols[1], [1, 1], 'renders equal-width: the stored row has the wrong length');
  const t = liveTracks(stored, lens);
  dragCol(t, before.cols, 1, 1, 0.7);
  const after = liveTracks(t, lens);
  assert.equal(after.cols[1].length, 2);
  assert.ok(Math.abs(after.cols[1][0] - 1.4) < 1e-9, `the drag took effect: ${after.cols[1]}`);
  assert.deepEqual(after.cols[0], [1, 1, 1], 'the full row is untouched');
});

test('a stored row SHORTER than the live one never leaks an old weight into a drag', () => {
  const stored = { rows: [1, 1], cols: [[1, 1, 1], [3, 1]] };
  const lens = [3, 3];
  const t = liveTracks(stored, lens);
  assert.deepEqual(t.cols[1], [1, 1, 1], 'starts from the rendered equal shares');
  dragCol(t, liveTracks(stored, lens).cols, 1, 2, 0.5);
  assert.deepEqual(liveTracks(t, lens).cols[1], [1, 1, 1], 'the untouched first column did not jump to 3');
});

test('liveTracks returns copies (editing never mutates the stored weights)', () => {
  const stored = { rows: [2, 1], cols: [[1, 2], [1]] };
  const t = liveTracks(stored, [2, 1]);
  t.rows[0] = 9;
  t.cols[0][0] = 9;
  assert.deepEqual(stored, { rows: [2, 1], cols: [[1, 2], [1]] });
  assert.deepEqual(normalize([1, -1], 2), [1, 1]);
});

test('TiledView edits start from the rendered weights, not the raw stored arrays', () => {
  const src = readFileSync(new URL('../src/modules/agents/TiledView.svelte', import.meta.url), 'utf8');
  assert.doesNotMatch(src, /tracks\[shape(Key)?\]\.cols\.map/, 'no verbatim copy of the stored per-shape arrays');
  assert.equal((src.match(/const t = currentTracks\(\);/g) ?? []).length, 2, 'drag + keyboard nudge');
});
