// Otto School — the cast: clones a kid / headmaster model with its clips
// (one-shot clips clamp on their last frame) and a pick box, cross-fades
// between clips, and places an actor where the life director says.

import type * as THREE_NS from 'three';
import type { GLTF } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { NOD_MS, ONCE_CLIPS, SCOLD_MS, SIT_MS, STAND_MS, type ActorView } from '../life.ts';
import { toWorld, type Room } from '../model.ts';
import type { SchoolPick } from './api.ts';
import type { Character, Obj, Three } from './shared.ts';

export const KID_CLIP_NAMES = ['Sit_Type', 'Sit_Idle', 'Sit_RaiseHand', 'Sit_Slump', 'Stand_Up', 'Sit_Down', 'Walk', 'Idle_Stand', 'Talk', 'Look_Around', 'Wave'];

export function createCast(
  T: Three,
  cloneSkinned: (o: Obj) => Obj,
  hitGeo: THREE_NS.BoxGeometry,
  hitMat: THREE_NS.MeshBasicMaterial,
  roomGroup: THREE_NS.Group,
  reduced: boolean,
) {
  function makeCharacter(id: string, g: GLTF, clips: readonly string[]): Character {
    const obj = cloneSkinned(g.scene);
    obj.traverse((o) => {
      if ((o as THREE_NS.Mesh).isMesh) {
        o.castShadow = true;
        o.receiveShadow = true;
        o.frustumCulled = false;
      }
    });
    const mixer = new T.AnimationMixer(obj);
    const actions = new Map<string, THREE_NS.AnimationAction>();
    for (const c of g.animations) {
      if (!clips.includes(c.name)) continue;
      const a = mixer.clipAction(c);
      if (ONCE_CLIPS.has(c.name)) {
        a.setLoop(T.LoopOnce, 1);
        a.clampWhenFinished = true;
      }
      actions.set(c.name, a);
    }
    const hit = new T.Mesh(hitGeo, hitMat);
    hit.userData.pick = id === 'headmaster' ? ({ kind: 'head', id: 'headmaster' } satisfies SchoolPick) : ({ kind: 'kid', id } satisfies SchoolPick);
    if (id === 'headmaster') {
      hit.scale.set(0.8, 1.9, 0.8);
      hit.position.y = 0.95;
    } else {
      hit.scale.set(0.6, 1.2, 0.6);
      hit.position.y = 0.6;
    }
    obj.add(hit);
    roomGroup.add(obj);
    return { id, obj, mixer, actions, clip: '', hit, heading: Math.PI };
  }

  function play(c: Character, clip: string, phaseMs?: number): void {
    if (c.clip === clip) return;
    const next = c.actions.get(clip) ?? c.actions.get('Idle_Stand') ?? c.actions.get('Idle') ?? null;
    const prev = c.actions.get(c.clip);
    c.clip = clip;
    if (!next) return;
    next.reset();
    if (phaseMs && ONCE_CLIPS.has(clip)) next.timeScale = next.getClip().duration / (phaseMs / 1000);
    else next.timeScale = 1;
    next.enabled = true;
    next.setEffectiveWeight(1);
    next.play();
    if (prev && prev !== next) prev.crossFadeTo(next, reduced ? 0 : 0.28, false);
  }

  function placeCharacter(c: Character, v: ActorView, r: Room, dt: number): void {
    c.obj.visible = v.visible;
    const w = toWorld(r, v);
    c.obj.position.set(w.x, 0, w.z);
    // Turn smoothly (shortest way round).
    let d = v.heading - c.heading;
    d = Math.atan2(Math.sin(d), Math.cos(d));
    c.heading += reduced ? d : d * Math.min(1, dt / 120);
    c.obj.rotation.y = c.heading;
    const phase = v.clip === 'Stand_Up' ? STAND_MS : v.clip === 'Sit_Down' ? SIT_MS : v.clip === 'Nod' ? NOD_MS : v.clip === 'Scold' ? SCOLD_MS : undefined;
    play(c, v.clip, phase);
  }

  return { makeCharacter, placeCharacter };
}
