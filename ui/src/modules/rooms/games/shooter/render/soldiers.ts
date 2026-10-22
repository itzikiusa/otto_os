/**
 * The crowd: every soldier and every infected as articulated low-poly bodies
 * drawn with five InstancedMeshes in all (capsule limbs, rounded-box torso /
 * vest / pack / boots / hands, heads, helmets and the held weapon's boxes),
 * whatever the head count. Each frame the renderer calls `begin()`, `add()`
 * once per visible body with its pose (see pose.ts) and `end()`; joints are
 * chained with preallocated matrices (no per-frame allocation), bodies
 * outside the view frustum are skipped, and per-instance colors are only
 * rewritten when a slot changes hands.
 */
import * as THREE from 'three';
import { RoundedBoxGeometry } from 'three/examples/jsm/geometries/RoundedBoxGeometry.js';
import type { Pose } from './pose';
import { GUN_PARTS, type GunSpec } from './guns';

/** A body's colors: skin, hair, top, trousers, vest, helmet (-1: bare head), boots, pack, gloves. */
export interface Skin { skin: number; hair: number; top: number; legs: number; vest: number; helmet: number; boots: number; pack: number; gloves: number }
const LIMBS = 8, BOXES = 10;
/** Hip height and limb lengths (m); the body stands 1.8 m tall. */
const HIP = 0.95, THIGH = 0.44, SHIN = 0.44, UPPER = 0.29, FORE = 0.27;

const _root = new THREE.Matrix4(), _pelvis = new THREE.Matrix4(), _chest = new THREE.Matrix4(), _head = new THREE.Matrix4();
const _thigh = new THREE.Matrix4(), _shin = new THREE.Matrix4(), _upper = new THREE.Matrix4(), _fore = new THREE.Matrix4(), _gun = new THREE.Matrix4();
const _local = new THREE.Matrix4(), _part = new THREE.Matrix4(), _out = new THREE.Matrix4(), _e = new THREE.Euler();
const _sphere = new THREE.Sphere(new THREE.Vector3(), 1.3), _col = new THREE.Color();
const ZERO = new THREE.Matrix4().makeScale(0, 0, 0);
const _gunIds = new Map<GunSpec, number>();
const gunId = (g: GunSpec): number => { let v = _gunIds.get(g); if (v === undefined) { v = _gunIds.size + 1; _gunIds.set(g, v); } return v; };

/** out = parent * T(tx, ty, tz) * R(rx, ry, rz). */
function joint(out: THREE.Matrix4, parent: THREE.Matrix4, tx: number, ty: number, tz: number, rx: number, ry: number, rz: number, order: THREE.EulerOrder = 'YXZ'): THREE.Matrix4 {
  _e.set(rx, ry, rz, order);
  _local.makeRotationFromEuler(_e).setPosition(tx, ty, tz);
  return out.multiplyMatrices(parent, _local);
}

export class SoldierCrowd {
  readonly group = new THREE.Group();
  private limbs: THREE.InstancedMesh; private boxes: THREE.InstancedMesh; private heads: THREE.InstancedMesh; private helmets: THREE.InstancedMesh; private guns: THREE.InstancedMesh;
  private resources: { dispose(): void }[] = [];
  private capacity: number;
  private n = 0;
  /** Skin + team + gun shape last written to each slot (colors are rewritten only when it changes). */
  private slotKey: Float64Array;
  private skins: Skin[] = [];
  private frustum: THREE.Frustum | null = null;
  /** World position of the last added body's muzzle (tracers and flashes start here). */
  readonly muzzle = new THREE.Vector3();

  constructor(capacity: number) {
    this.capacity = capacity;
    this.slotKey = new Float64Array(capacity).fill(NaN);
    const limbGeo = new THREE.CapsuleGeometry(0.5, 1, 3, 8).translate(0, -1, 0);
    const boxGeo = new RoundedBoxGeometry(1, 1, 1, 2, 0.18);
    const headGeo = new THREE.SphereGeometry(0.5, 12, 10);
    const helmetGeo = new THREE.SphereGeometry(0.5, 12, 6, 0, Math.PI * 2, 0, Math.PI * 0.55);
    const gunGeo = new THREE.BoxGeometry(1, 1, 1);
    // Base colors above 1 lift the palette's dark fatigues so bodies read in dim light.
    const cloth = new THREE.MeshStandardMaterial({ roughness: 0.9, metalness: 0, color: new THREE.Color(1.35, 1.35, 1.35) });
    const skinMat = new THREE.MeshStandardMaterial({ roughness: 0.62, metalness: 0, color: new THREE.Color(1.1, 1.1, 1.1) });
    const shell = new THREE.MeshStandardMaterial({ roughness: 0.55, metalness: 0.15 });
    const metal = new THREE.MeshStandardMaterial({ roughness: 0.42, metalness: 0.55 });
    const make = (geo: THREE.BufferGeometry, mat: THREE.Material, count: number) => {
      const m = new THREE.InstancedMesh(geo, mat, count);
      m.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
      m.setColorAt(0, _col.setRGB(1, 1, 1));
      m.count = 0; m.frustumCulled = false; m.castShadow = true; m.receiveShadow = true;
      this.group.add(m);
      return m;
    };
    this.limbs = make(limbGeo, cloth, capacity * LIMBS);
    this.boxes = make(boxGeo, cloth, capacity * BOXES);
    this.heads = make(headGeo, skinMat, capacity);
    this.helmets = make(helmetGeo, shell, capacity);
    this.guns = make(gunGeo, metal, capacity * GUN_PARTS);
    this.resources.push(limbGeo, boxGeo, headGeo, helmetGeo, gunGeo, cloth, skinMat, shell, metal);
  }

  /** Registers a skin; returns its index for `add`. */
  skin(s: Skin): number { this.skins.push(s); return this.skins.length - 1; }

  begin(frustum: THREE.Frustum | null): void { this.n = 0; this.frustum = frustum; }

  /**
   * Adds one body standing at (x, y, z) facing `yaw`. `band` is the team
   * color for the armband (-1: none, the skin's top), `gun` the held item.
   * Returns false when the body was culled or the crowd is full.
   */
  add(x: number, y: number, z: number, yaw: number, pose: Pose, skin: number, band: number, gun: GunSpec | null, scale = 1): boolean {
    if (this.n >= this.capacity) return false;
    if (this.frustum) {
      _sphere.center.set(x, y + 0.9, z);
      if (!this.frustum.intersectsSphere(_sphere)) return false;
    }
    const i = this.n++, s = this.skins[skin] ?? this.skins[0];
    // Root: placed and turned, lowered, then tilted / rolled about the hips.
    _root.makeRotationY(yaw).setPosition(x, y, z);
    if (scale !== 1) _root.multiply(_local.makeScale(scale, scale, scale));
    joint(_pelvis, _root, 0, HIP - pose.drop, 0, pose.tilt, 0, pose.roll, 'XYZ');
    joint(_chest, _pelvis, 0, 0, 0, pose.spine, pose.twist, 0);
    const L = this.limbs, B = this.boxes;
    const li = i * LIMBS, bi = i * BOXES;
    this.put(B, bi, _pelvis, 0, 0, 0, 0.32, 0.17, 0.21);
    this.put(B, bi + 1, _chest, 0, 0.3, 0, 0.36, 0.5, 0.22);
    this.put(B, bi + 2, _chest, 0, 0.32, 0.01, 0.42, 0.34, 0.28);
    this.put(B, bi + 3, _chest, 0, 0.33, -0.19, 0.29, 0.34, 0.14);
    this.put(B, bi + 8, _chest, 0, 0.05, 0.02, 0.35, 0.08, 0.25);
    // Legs (left is +x: the body faces +z).
    for (let side = 0; side < 2; side++) {
      const sx = side === 0 ? 0.1 : -0.1, hip = side === 0 ? pose.hipL : pose.hipR, knee = side === 0 ? pose.kneeL : pose.kneeR;
      joint(_thigh, _pelvis, sx, -0.04, 0, -hip, 0, 0);
      this.put(L, li + side, _thigh, 0, 0, 0, 0.16, THIGH / 2, 0.16);
      joint(_shin, _thigh, 0, -THIGH, 0, knee, 0, 0);
      this.put(L, li + 2 + side, _shin, 0, 0, 0, 0.13, SHIN / 2, 0.13);
      this.put(B, bi + 4 + side, _shin, 0, -SHIN - 0.02, 0.05, 0.12, 0.1, 0.27);
    }
    // Arms.
    for (let side = 0; side < 2; side++) {
      const sx = side === 0 ? 0.23 : -0.23, sh = side === 0 ? pose.shL : pose.shR, roll = side === 0 ? pose.rollL : -pose.rollR, el = side === 0 ? pose.elL : pose.elR;
      joint(_upper, _chest, sx, 0.5, 0, -sh, 0, roll, 'XYZ');
      this.put(L, li + 4 + side, _upper, 0, 0, 0, 0.11, UPPER / 2, 0.11);
      if (side === 0) this.put(B, bi + 9, _upper, 0, -0.08, 0, 0.125, 0.06, 0.125);
      joint(_fore, _upper, 0, -UPPER, 0, -el, 0, 0);
      this.put(L, li + 6 + side, _fore, 0, 0, 0, 0.095, FORE / 2, 0.095);
      this.put(B, bi + 6 + side, _fore, 0, -FORE - 0.03, 0, 0.075, 0.1, 0.07);
    }
    // Head and helmet.
    joint(_head, _chest, 0, 0.58, 0.01, -pose.head, 0, 0);
    this.put(this.heads, i, _head, 0, 0.11, 0.01, 0.23, 0.25, 0.24);
    if (s.helmet >= 0) this.put(this.helmets, i, _head, 0, 0.13, 0, 0.29, 0.27, 0.31);
    else this.put(this.helmets, i, _head, 0, 0.15, -0.015, 0.25, 0.2, 0.26);
    // Held item, anchored to the chest (the right side is -x).
    const G = this.guns, gi = i * GUN_PARTS;
    joint(_gun, _chest, -pose.gunX, 0.42 + pose.gunY, pose.gunZ, -pose.gunPitch, pose.gunYaw, 0);
    for (let k = 0; k < GUN_PARTS; k++) {
      const part = gun?.parts[k];
      if (part) this.put(G, gi + k, _gun, part[0], part[1], part[2], part[3], part[4], part[5]);
      else G.setMatrixAt(gi + k, ZERO);
    }
    if (gun) this.muzzle.set(gun.muzzle[0], gun.muzzle[1], gun.muzzle[2]).applyMatrix4(_gun);
    else this.muzzle.set(0, 1.3, 0.3).applyMatrix4(_root);
    // Colours, only when this slot now shows someone else (or a different gun).
    const key = (skin * 16777217 + band + 1) * 32 + (gun ? gunId(gun) : 0);
    if (this.slotKey[i] !== key) {
      this.slotKey[i] = key;
      const c = (mesh: THREE.InstancedMesh, k: number, hex: number) => mesh.setColorAt(k, _col.setHex(hex));
      c(B, bi, s.legs); c(B, bi + 1, s.top); c(B, bi + 2, s.vest); c(B, bi + 3, s.pack); c(B, bi + 8, s.pack);
      c(B, bi + 4, s.boots); c(B, bi + 5, s.boots); c(B, bi + 6, s.gloves); c(B, bi + 7, s.gloves); c(B, bi + 9, band >= 0 ? band : s.top);
      c(L, li, s.legs); c(L, li + 1, s.legs); c(L, li + 2, s.legs); c(L, li + 3, s.legs);
      c(L, li + 4, s.top); c(L, li + 5, s.top); c(L, li + 6, s.top); c(L, li + 7, s.top);
      c(this.heads, i, s.skin); c(this.helmets, i, s.helmet >= 0 ? s.helmet : s.hair);
      for (let k = 0; k < GUN_PARTS; k++) { const part = gun?.parts[k]; c(G, gi + k, part && gun ? gun.colors[part[6]] : 0x000000); }
      for (const m of [L, B, this.heads, this.helmets, G]) if (m.instanceColor) m.instanceColor.needsUpdate = true;
    }
    return true;
  }

  end(): void {
    const n = this.n;
    this.limbs.count = n * LIMBS; this.boxes.count = n * BOXES; this.heads.count = n; this.helmets.count = n; this.guns.count = n * GUN_PARTS;
    for (const m of [this.limbs, this.boxes, this.heads, this.helmets, this.guns]) m.instanceMatrix.needsUpdate = true;
  }

  /** Bodies drawn this frame. */
  get drawn(): number { return this.n; }

  /** frame * T(cx, cy, cz) * S(sx, sy, sz) into instance `k`. */
  private put(mesh: THREE.InstancedMesh, k: number, frame: THREE.Matrix4, cx: number, cy: number, cz: number, sx: number, sy: number, sz: number): void {
    _part.makeScale(sx, sy, sz).setPosition(cx, cy, cz);
    mesh.setMatrixAt(k, _out.multiplyMatrices(frame, _part));
  }

  dispose(): void { for (const r of this.resources) r.dispose(); for (const m of [this.limbs, this.boxes, this.heads, this.helmets, this.guns]) m.dispose(); }
}

/** Soldier looks for the twelve survivors (index = FIGHTER_IDS order): fatigues, vests, helmets or caps, skin tones. */
export const SOLDIER_SKINS: Skin[] = [
  { skin: 0xe0ac7e, hair: 0x3a2a1e, top: 0x5b6346, legs: 0x4c5238, vest: 0x3d4430, helmet: 0x4f5a3c, boots: 0x2a2520, pack: 0x5a5038, gloves: 0x2b2b2b },
  { skin: 0xf1c7a1, hair: 0xd8b25a, top: 0x8a7a5c, legs: 0x6e6248, vest: 0x5e5340, helmet: -1, boots: 0x3a2f24, pack: 0x6a5c44, gloves: 0x3a3a3a },
  { skin: 0x8d5a3b, hair: 0x1c1410, top: 0x2f3b4c, legs: 0x283240, vest: 0x1e252e, helmet: 0x232a33, boots: 0x161616, pack: 0x2a3340, gloves: 0x151515 },
  { skin: 0xc68e62, hair: 0x5a2e1e, top: 0x6b5d4a, legs: 0x4e5a3a, vest: 0x7a6a4c, helmet: 0x6c6048, boots: 0x30281e, pack: 0x4e4a36, gloves: 0x2e2a22 },
  { skin: 0xe8b48c, hair: 0x2a1c14, top: 0x4a4f55, legs: 0x3a3e44, vest: 0x2a2d31, helmet: 0x3b3f44, boots: 0x1c1c1c, pack: 0x45484c, gloves: 0x222222 },
  { skin: 0xa86e48, hair: 0x101010, top: 0x7a3e34, legs: 0x3e3a36, vest: 0x2c2a28, helmet: -1, boots: 0x201c18, pack: 0x4a3a30, gloves: 0x1e1e1e },
  { skin: 0xf0c09a, hair: 0x7a4a26, top: 0x58624a, legs: 0x8a7c5e, vest: 0x46503a, helmet: 0x56603f, boots: 0x3c3024, pack: 0x6a6046, gloves: 0x34302a },
  { skin: 0x6e4630, hair: 0x0e0b09, top: 0x3c4a3c, legs: 0x2e3a2e, vest: 0x26302a, helmet: 0x2e3a30, boots: 0x161616, pack: 0x3a4436, gloves: 0x1a1a1a },
  { skin: 0xd9a27a, hair: 0x8f8f8f, top: 0xb8b2a2, legs: 0x5a6070, vest: 0x8a8678, helmet: -1, boots: 0x2a2a2a, pack: 0x6a6658, gloves: 0x3a3a3a },
  { skin: 0xf3cfb0, hair: 0xa8442a, top: 0x4a5a6c, legs: 0x3a4652, vest: 0x2e3946, helmet: 0x3a4858, boots: 0x22252a, pack: 0x4a5664, gloves: 0x24282c },
  { skin: 0xb98058, hair: 0x2c1a10, top: 0x6c6a40, legs: 0x5a5836, vest: 0x4c4a2e, helmet: 0x5e5c38, boots: 0x2c2818, pack: 0x5c5a3a, gloves: 0x2a2818 },
  { skin: 0xe6b28e, hair: 0x101418, top: 0x2a2a2e, legs: 0x1e1e22, vest: 0x141416, helmet: -1, boots: 0x101010, pack: 0x26262a, gloves: 0x0e0e0e },
];
/** Infected: grey-green skin, torn civilian clothes, no kit. */
export const INFECTED_SKINS: Skin[] = [
  { skin: 0x8f9a7a, hair: 0x2a2620, top: 0x6a5a4a, legs: 0x3a3e48, vest: 0x5a4c3e, helmet: -1, boots: 0x2a2420, pack: 0x4a3e34, gloves: 0x7f8a6c },
  { skin: 0x7e8c72, hair: 0x14120e, top: 0x8a8a86, legs: 0x4a4a46, vest: 0x6e6c66, helmet: -1, boots: 0x1e1c1a, pack: 0x5a5852, gloves: 0x76846a },
  { skin: 0x9aa088, hair: 0x5a4430, top: 0x7a3a34, legs: 0x2e3440, vest: 0x5e2e2a, helmet: -1, boots: 0x262220, pack: 0x4a2a26, gloves: 0x8a9478 },
  { skin: 0x86947e, hair: 0x6a6a66, top: 0xa8a39a, legs: 0x5a5a62, vest: 0x8a857c, helmet: -1, boots: 0x2e2c2a, pack: 0x6a665e, gloves: 0x7c8a72 },
  { skin: 0x7a866a, hair: 0x241c14, top: 0x3e5a6a, legs: 0x3a3a3a, vest: 0x2e4652, helmet: -1, boots: 0x1a1a1a, pack: 0x32424a, gloves: 0x72806a },
  // Brutes and soldiers that turned: a helmet still on.
  { skin: 0x84907a, hair: 0x1a1a1a, top: 0x4e5640, legs: 0x3e4432, vest: 0x34392a, helmet: 0x434a36, boots: 0x1e1c18, pack: 0x404432, gloves: 0x2a2a2a },
];
