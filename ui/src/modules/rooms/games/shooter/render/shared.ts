// Storm Royale renderer shared bits: per-map themes, scratch math objects,
// lookup tables, the fighter visual record and the renderer's public option
// and view types (re-exported by ../render.ts).
import * as THREE from 'three';
import type { Theme } from '../../common/view';
import type { ShooterMap } from '../map';
import type { FighterId } from '../catalog';
import type { GroundColours } from './world';
import type { WeatherId } from './weather';

export const THEMES: Record<ShooterMap['theme'], Theme & GroundColours & { floor: number }> = {
  isle: { skyTop: 0x3d84d6, skyHorizon: 0xcbe7ff, fog: 0xcfe6fb, fogNear: 140, fogFar: 620, sun: 0xfff2da, sunIntensity: 2.7, ambientSky: 0xd4e8ff, ambientGround: 0x5e7a3c, ambient: 1.1, low: 0x7cc35a, mid: 0x5da746, high: 0x8fbf63, rock: 0x8d8f8c, sand: 0xe8d39a, floor: 0x2e8fd0 },
  mesa: { skyTop: 0x3c7ccb, skyHorizon: 0xf4d6ab, fog: 0xf0d3a8, fogNear: 140, fogFar: 620, sun: 0xffe3bb, sunIntensity: 2.9, ambientSky: 0xffe6c6, ambientGround: 0x9a5b36, ambient: 1.05, low: 0xd99a5c, mid: 0xc97a45, high: 0xe3b276, rock: 0xa65a34, sand: 0xe7c08a, floor: 0xc97a45 },
  city: { skyTop: 0x5d6a78, skyHorizon: 0xc9b9a2, fog: 0xb5a690, fogNear: 60, fogFar: 420, sun: 0xffe2c0, sunIntensity: 2.3, ambientSky: 0xd7d0c6, ambientGround: 0x4a4038, ambient: 1.0, low: 0x6a645b, mid: 0x746c5e, high: 0x7d7262, rock: 0x77716a, sand: 0x857a68, floor: 0x5a544c },
};
export const LOOT_RANGE = 70;
export const _v = new THREE.Vector3(), _w = new THREE.Vector3(), _d = new THREE.Vector3(), _eye = new THREE.Vector3();
export const _q = new THREE.Quaternion();
export const _m4 = new THREE.Matrix4(), _pm = new THREE.Matrix4();
export const _frustum = new THREE.Frustum();
export const ZERO = new THREE.Matrix4().makeScale(0, 0, 0);
export const UP = new THREE.Vector3(0, 1, 0);
export const PROBE_HEIGHTS = [1.65, 1.1];
export const MAT_KEYS = ['surv-box', 'surv-rock-a', 'surv-metal-panel-screws'];
/** Shot event `value`: the weapon by WEAPON order (rockets draw their own trail). */
export const SHOT_GUNS = ['pistol', 'smg', 'rifle', 'shotgun', 'sniper', 'rocket'];
/** Viewmodel kick per weapon (0..1). */
export const KICK: Record<string, number> = { pistol: 0.45, smg: 0.2, rifle: 0.35, shotgun: 1, sniper: 1, rocket: 0.8 };
/** Stance fields newer sims carry, read defensively (an older snapshot renders standing). */
export type Stance = { ads?: number; crouch?: boolean; slideT?: number };

export interface FighterVisual {
  root: THREE.Group; glider: THREE.Mesh; label: THREE.Sprite | null;
  /** Animation state: stride phase, death progress, eased stance blends, recoil, last muzzle position and when it was drawn. */
  phase: number; dead: number; crouch: number; slide: number; aim: number; kick: number;
  muzzle: THREE.Vector3; seen: number;
}
/** A seat as the renderer sees it: model, label, and team (for teammate markers). */
export interface SeatView { fighter: FighterId; human: boolean; name: string; team: number }
/**
 * The local survivor's build intent, applied to the preview ghost before the
 * host has echoed it (guests would otherwise see the ghost trail the camera by
 * a round trip). Position is the freshest known one.
 */
export interface LocalBuild { building: boolean; piece: number; mat: number; yaw: number; pitch: number; x: number; y: number; z: number }

/** `firstPerson`: stay in first person (the viewmodel) whenever on foot, not only while aiming. */
export interface CameraState { yaw: number; pitch: number; aiming: boolean; zoom: number; firstPerson?: boolean }
/** Weather / time of day and gore; unset picks the map's default (or a saved preference). */
export interface RenderOptions { weather?: WeatherId; blood?: boolean }

export function stored(key: string): string | null { try { return localStorage.getItem(key); } catch { return null; } }
