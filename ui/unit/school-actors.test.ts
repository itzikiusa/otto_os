/// <reference lib="dom" />
/// <reference types="vite/client" />
import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as T from 'three';
import { clone } from 'three/examples/jsm/utils/SkeletonUtils.js';
import * as assets from '../src/modules/home/school/assets.ts';

test('actor disposal releases unique cloned skeletons and mixer bindings while preserving shared templates', () => {
  const dispose = (assets as unknown as { disposeActor?: (actor: { obj: T.Object3D; mixer: T.AnimationMixer }) => void }).disposeActor;
  assert.equal(typeof dispose, 'function', 'actor removal needs a shared owned-resource disposer');
  const template = new T.Group();
  const geometry = new T.BufferGeometry();
  const material = new T.MeshBasicMaterial();
  const mesh = new T.SkinnedMesh(geometry, material);
  const bone = new T.Bone();
  mesh.add(bone);
  mesh.bind(new T.Skeleton([bone]));
  mesh.skeleton.computeBoneTexture();
  template.add(mesh);
  let sharedDisposals = 0;
  geometry.addEventListener('dispose', () => sharedDisposals++);
  material.addEventListener('dispose', () => sharedDisposals++);
  const room = new T.Group();
  // Repeat visits, including a separate headmaster actor, without releasing templates.
  for (let visit = 0; visit < 10; visit++) {
    for (const role of ['kid', 'headmaster']) {
      const obj = clone(template);
      obj.name = role;
      room.add(obj);
      const skinned = obj.children[0] as T.SkinnedMesh;
      assert.notEqual(skinned.skeleton, mesh.skeleton);
      skinned.skeleton.computeBoneTexture();
      let boneDisposals = 0;
      skinned.skeleton.boneTexture!.addEventListener('dispose', () => boneDisposals++);
      // Multiple meshes may refer to one skeleton: dispose it once.
      const accessory = new T.SkinnedMesh(geometry, material);
      accessory.skeleton = skinned.skeleton;
      obj.add(accessory);
      const mixer = new T.AnimationMixer(obj);
      const clip = new T.AnimationClip('Move', 1, [new T.NumberKeyframeTrack('.position[x]', [0, 1], [0, 1])]);
      const action = mixer.clipAction(clip).play();
      mixer.update(0.5);
      dispose!({ obj, mixer });
      assert.equal(boneDisposals, 1);
      assert.equal(skinned.skeleton.boneTexture, null);
      assert.equal(action.isRunning(), false);
      assert.equal(mixer.existingAction(clip, obj), null);
      assert.equal(obj.parent, null);
      assert.equal(sharedDisposals, 0);
      assert.notEqual(mesh.skeleton.boneTexture, null, 'template skeleton remains usable');
    }
    assert.equal(room.children.length, 0);
  }
  mesh.skeleton.dispose();
  geometry.dispose();
  material.dispose();
});
