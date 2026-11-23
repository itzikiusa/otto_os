// Game-ready models carry more than the model itself: Otto Forge (like most
// game pipelines) ships lower-detail copies (`<name>_LOD1…3`) and invisible
// collision hulls (Unreal's `UCX_ / UBX_ / UCP_ / USP_` prefixes) as sibling
// meshes in the same GLB. A game engine knows to hide them; a plain viewer
// draws them all at once — the hulls have no material, so they paint white
// shards over the real surface and the model looks broken. Every place that
// shows a model drops them after loading (raycasts and framing then see only
// the model a person sees).
import type * as THREE_NS from 'three';

const COLLISION = /^(UCX|UBX|UCP|USP)_/i;
const LOWER_LOD = /_LOD[1-9]\d*(?:\.\d+)?$/i;

/** True for a node that only a game engine should see (collision hull or lower level of detail). */
export function isGameHelper(name: string | undefined | null): boolean {
  return !!name && (COLLISION.test(name) || LOWER_LOD.test(name));
}

/** Remove collision hulls and lower levels of detail from a loaded model, in place. Returns how many went. */
export function dropGameHelpers(root: THREE_NS.Object3D): number {
  const gone: THREE_NS.Object3D[] = [];
  root.traverse((o) => {
    if (o !== root && isGameHelper(o.name)) gone.push(o);
  });
  for (const o of gone) o.parent?.remove(o);
  return gone.length;
}
