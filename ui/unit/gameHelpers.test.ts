import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Group, Mesh } from 'three';
import { dropGameHelpers, isGameHelper } from '../src/lib/design/scene3d/gameHelpers.ts';

test('collision hulls and lower levels of detail are game-only nodes', () => {
  for (const n of ['UCX_fixed_00', 'UBX_box', 'ucp_x', 'USP_ball', 'fixed_LOD1', 'game_LOD2.002', 'fixed_LOD3.004']) assert.ok(isGameHelper(n), n);
  for (const n of ['fixed', 'game', 'gameready', 'fixed_LOD0', 'Head', 'LOD1', 'UCXfoo', '', undefined]) assert.ok(!isGameHelper(n as string), String(n));
});

test('a Forge game-ready GLB keeps only the model a person sees', () => {
  // The node layout Otto Forge writes (out/text_to_asset/prop_check/fixed.glb).
  const root = new Group();
  for (const n of ['fixed', 'fixed_LOD1', 'fixed_LOD2', 'fixed_LOD3', 'UCX_fixed_00', 'UCX_fixed_01']) {
    const m = new Mesh();
    m.name = n;
    root.add(m);
  }
  assert.equal(dropGameHelpers(root), 5);
  assert.deepEqual(root.children.map((c) => c.name), ['fixed']);
});
