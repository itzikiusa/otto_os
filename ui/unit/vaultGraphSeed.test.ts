// SD-03 step 3: a real vault change refetches the graph; surviving nodes keep
// their positions (by path), new nodes land by a placed neighbour, and a
// mostly-placed graph starts warm instead of re-exploding from the random disc.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { applySeed, carrySeed, WARM_SHARE } from '../src/modules/vault/graphSeed.ts';

test('carrySeed maps previous positions onto the new node order by path', () => {
  const prev = ['a.md', 'b.md', 'c.md'];
  const pos = Float32Array.from([1, 1, 2, 2, 3, 3]);
  const out = carrySeed(prev, pos, ['c.md', 'new.md', 'a.md'])!;
  assert.equal(out.known, 2);
  assert.deepEqual([out.seed[0], out.seed[1]], [3, 3]);
  assert.ok(Number.isNaN(out.seed[2]) && Number.isNaN(out.seed[3]), 'a new node has no carry');
  assert.deepEqual([out.seed[4], out.seed[5]], [1, 1]);
  assert.equal(carrySeed(prev, null, ['a.md']), null, 'no layout yet → fresh');
  assert.equal(carrySeed(prev, pos, ['x.md']), null, 'nothing survives → fresh');
});

test('applySeed keeps carried nodes and places a new node beside its neighbour', () => {
  const px = new Float32Array([500, 500, 500]);
  const py = new Float32Array([500, 500, 500]);
  const seed = Float32Array.from([10, 20, 30, 40, Number.NaN, Number.NaN]);
  const edges = Uint32Array.from([2, 1]); // the new node links to node 1
  let r = 0;
  const rnd = () => ((r = (r * 9301 + 49297) % 233280), r / 233280);
  const known = applySeed(px, py, seed, edges, rnd);
  assert.equal(known, 2);
  assert.deepEqual([px[0], py[0], px[1], py[1]], [10, 20, 30, 40]);
  assert.ok(Math.abs(px[2] - 30) <= 15 && Math.abs(py[2] - 40) <= 15, 'next to its neighbour, not the disc');
  assert.ok(known >= px.length * WARM_SHARE, 'mostly placed → warm start');
});
