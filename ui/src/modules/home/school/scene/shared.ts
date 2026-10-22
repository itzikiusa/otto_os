// Otto School — the scene's internal shapes and constants, shared by
// ../scene.ts and its scene/ helpers (type-only three.js imports).

import type * as THREE_NS from 'three';
import { BENCH_SLOTS, MAX_BACK, MAX_FRONT } from '../model.ts';

export type Three = typeof THREE_NS;
export type Obj = THREE_NS.Object3D;

/** Nodes the runtime paints or moves (never instanced). */
export const LIVE_CHILDREN = new Set(['Screen', 'BoardTitle', 'NamePlate', 'DoorSign', 'DoorLeaf']);
/** Kit pieces that never cast shadows: walls / floors / doors would shut the
 *  low window sun out of the room; the ceiling is a separate plane. */
export const NO_CAST = new Set(['Floor_Tile', 'Corridor_Floor', 'Wall', 'Wall_Window', 'Wall_Door', 'Corridor_Wall', 'Corridor_Door', 'Ceiling_Light', 'Board', 'Clock', 'Corkboard']);
/** Pins / rings for every seat a room can hold. */
export const SEATS = MAX_FRONT + MAX_BACK + BENCH_SLOTS;
export const LAMPS = 4;
export const EYE = 1.62;
export const FPS_MS = 1000 / 30;
export const ease = (t: number) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);

export interface MeshDesc {
  geometry: THREE_NS.BufferGeometry;
  material: THREE_NS.Material | THREE_NS.Material[];
  rel: THREE_NS.Matrix4;
}

export interface Painted {
  mesh: THREE_NS.Mesh;
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
  tex: THREE_NS.CanvasTexture;
  key: string;
}

export interface Character {
  id: string;
  obj: Obj;
  mixer: THREE_NS.AnimationMixer;
  actions: Map<string, THREE_NS.AnimationAction>;
  clip: string;
  hit: THREE_NS.Mesh;
  heading: number;
}

export interface Door {
  roomId: string;
  pivots: { obj: Obj; base: THREE_NS.Matrix4 }[];
  angle: number;
  target: number;
  hit: THREE_NS.Mesh | null;
  plate: Painted | null;
  sign: Painted | null;
}
