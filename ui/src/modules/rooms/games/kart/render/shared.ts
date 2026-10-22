// Kart renderer shared bits: tint tables, scratch vectors and the visual
// records the world builder (render/world.ts) and the animator (../render.ts) share.
import * as THREE from 'three';
import { newNear, type Drawbridge, type Near } from '../tracks';
import type { RacerId } from '../roster';
import type { ShellKind } from '../sim';
import type { TerrainKind } from './surfaces';

export const KART_SCALE = 2.0;
/** Drift spark colours per mini-turbo tier (0: still charging). */
export const SPARK = [0x9a9a9a, 0x4fc3ff, 0xff9a2e, 0xd36bff];
/** Boost flame tints: dash panel / ramp, mushroom, rocket start, golden mushroom, slipstream. */
export const FLAME = { pad: 0xffa030, mushroom: 0xff5a1e, start: 0xffd040, golden: 0xffd84a, draft: 0x9fe6ff };
/** Shell kinds, by the index the `throw` / `explode` events carry. */
export const SHELL_TINT: Record<ShellKind, number> = { green: 0x3fe04a, red: 0xff4a3a, blue: 0x4f8fff, bomb: 0xffa040 };
export const FIREWORK = [0xff4f6a, 0xffd23a, 0x4fc3ff, 0x7dff6a, 0xd36bff, 0xffffff];
export const TERRAIN: Record<string, TerrainKind> = { meadow: 'grass', city: 'pave', canyon: 'sand', frost: 'snow', lagoon: 'grass', volcano: 'rock', dusk: 'pave' };
/** Underwater fog. */
export const DEEP = { color: 0x0d5f78, near: 2, far: 120 };
export const _near: Near = newNear();
export const _pt = [0, 0, 0, 0];
export const _ob = [0, 0, 0, 0];
export const _cp = [0, 0, 0, 0];
export const _v = new THREE.Vector3();
export const _look = new THREE.Vector3();
export const _m = new THREE.Matrix4();
export const _q = new THREE.Quaternion();
export const _s = new THREE.Vector3();
export const _p = new THREE.Vector3();
export const _up = new THREE.Vector3(0, 1, 0);

export interface KartVisual {
  racer: RacerId; root: THREE.Group; body: THREE.Group; wheels: THREE.Object3D[]; front: THREE.Object3D[];
  mixer: THREE.AnimationMixer | null; flames: THREE.Mesh[]; cores: THREE.Mesh[]; flameMat: THREE.MeshBasicMaterial;
  exhaust: THREE.Sprite[]; exhaustMat: THREE.SpriteMaterial; sparks: THREE.Sprite[]; sparkMat: THREE.SpriteMaterial;
  bubble: THREE.Mesh; label: THREE.Sprite | null; wing: THREE.Group;
  wheelSpin: number; tilt: number; bounce: number; visualYaw: number; spinAngle: number; pitch: number; wingOpen: number;
  /** What the current boost's flames look like, the last drift tier seen, the shrink from lightning. */
  flameTint: number; lastLevel: number; size: number;
}
export interface ObstacleVisual { mesh: THREE.Mesh; roll: number }
/** A drawbridge's two leaves, each a group pivoting about its hinge. */
export interface DrawbridgeVisual { def: Drawbridge; leaves: THREE.Group[] }

export interface RenderRacer { id: number; racer: RacerId; human: boolean; name: string }
