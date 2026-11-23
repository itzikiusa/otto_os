import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as THREE from 'three';
import { RigPlayer } from '../src/lib/design/scene3d/rigPlayer.ts';

/** A model with one clip that walks its root along x (one unit a second). */
function model(name: string): THREE.Group {
  const g = new THREE.Group();
  g.name = name;
  const track = new THREE.NumberKeyframeTrack('.position[x]', [0, 1], [0, 1]);
  g.userData.ottoAnimations = [new THREE.AnimationClip(`${name}-pose`, 1, [track])];
  return g;
}

test('every animated model of a scene plays its own first clip, not only the first', () => {
  const scene = new THREE.Group();
  const models = ['girl', 'boy', 'owl'].map(model);
  scene.add(...models);
  const rig = new RigPlayer(THREE as never, new THREE.Group());
  const info = rig.syncFromContent(scene);
  assert.equal(info.clips.length, 1, 'the first model still drives the clip picker');
  rig.setPlayback(0, true);
  assert.equal(rig.isAnimating, true);
  rig.step(0.5);
  for (const m of models) assert.ok(Math.abs(m.position.x - 0.5) < 1e-6, `${m.name} moves: ${m.position.x}`);
  rig.setPlayback(0, false);
  rig.step(0.25);
  for (const m of models) assert.ok(Math.abs(m.position.x - 0.5) < 1e-6, `${m.name} paused`);
  rig.dispose();
});
