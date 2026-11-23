// The bounds the camera frames: the scene's subject, not its ground. A 20 m floor
// plane (the default model stage) or any other plane would otherwise make the
// fit frame the floor and leave a character a few pixels tall.
import type * as THREE_NS from 'three';
import type { Three } from './build';
import type { Scene3dDoc } from './types';
import type { V3 } from './cameraFit';

/** Stand a loaded model placed on the ground (its object at y = 0) on the floor: its posed
 *  bounds' bottom at y = 0 (a character modelled around its hips otherwise sinks half into the
 *  model stage's opaque floor). A model the scene raised (already lifted onto the ground, or on
 *  top of another piece) stays where the scene put it. The shift is in the object's own units. */
export function standOnFloor(T: Three, model: THREE_NS.Object3D): void {
  const holder = model.parent;
  if (!holder || Math.abs(holder.getWorldPosition(new T.Vector3()).y) > 1e-6) return;
  const b = new T.Box3().setFromObject(updateSkins(model));
  if (Number.isFinite(b.min.y)) model.position.y -= b.min.y / holder.getWorldScale(new T.Vector3()).y;
}

function updateSkins(root: THREE_NS.Object3D): THREE_NS.Object3D {
  root.updateMatrixWorld(true);
  root.traverse((o) => {
    const s = o as THREE_NS.SkinnedMesh;
    if (s.isSkinnedMesh) {
      s.skeleton.update();
      s.computeBoundingBox();
    }
  });
  return root;
}

export function subjectBox(T: Three, doc: Scene3dDoc, nodes: Map<string, THREE_NS.Object3D>, content: THREE_NS.Object3D): THREE_NS.Box3 {
  // Skinned meshes measure their posed vertices, so bring the bones up to date
  // first (before the first frame their matrices are still zero).
  updateSkins(content);
  const box = new T.Box3();
  for (const o of doc.objects) {
    const n = o.type === 'plane' || o.backdrop ? undefined : nodes.get(o.id);
    if (n) box.expandByObject(n);
    if (o.frame) box.union(new T.Box3(new T.Vector3(...o.frame[0]), new T.Vector3(...o.frame[1])));
  }
  return box.isEmpty() ? box.setFromObject(content) : box;
}

/** Each piece's own box (planes and backdrop pieces left out), for {@link fitCamera}'s piece-by-piece framing. */
export function subjectBoxes(T: Three, doc: Scene3dDoc, nodes: Map<string, THREE_NS.Object3D>): [V3, V3][] {
  return doc.objects.flatMap((o): [V3, V3][] => {
    const n = o.type === 'plane' || o.backdrop ? undefined : nodes.get(o.id);
    // Precise: the posed vertices themselves, so a turned or skinned model adds no empty corners.
    const b = n ? new T.Box3().setFromObject(n, true) : null;
    return [...(b && !b.isEmpty() ? [[b.min.toArray(), b.max.toArray()] as [V3, V3]] : []), ...(o.frame ? [o.frame as [V3, V3]] : [])];
  });
}
