/**
 * Kart effects that are more than particles, all pooled and built once:
 * camera-space speed lines, lightning strikes (a jagged bolt from the sky with
 * a glow sheath, rebuilt in place per strike), ground shockwave rings, and
 * the shell models (green, red and spiked blue shells; the fuse bomb is the
 * kit model). Nothing here allocates per frame.
 */
import * as THREE from 'three';
import type { AssetLibrary } from '../../common/assets';
import { approach } from '../../common/math';
import type { ShellKind } from '../sim';
import { shellTexture } from './surfaces';

/** Streaks rushing past the camera at speed and on every boost (one instanced draw). */
export class SpeedLines {
  readonly mesh: THREE.InstancedMesh;
  private material: THREE.MeshBasicMaterial;
  private z: Float32Array; private angle: Float32Array; private radius: Float32Array; private len: Float32Array;
  private m = new THREE.Matrix4(); private p = new THREE.Vector3(); private q = new THREE.Quaternion(); private s = new THREE.Vector3();
  private level = 0;
  constructor(lib: AssetLibrary, camera: THREE.Camera, private count = 64) {
    const geo = lib.own(new THREE.PlaneGeometry(0.03, 1));
    geo.rotateX(Math.PI / 2);
    this.material = lib.own(new THREE.MeshBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0, blending: THREE.AdditiveBlending, depthWrite: false, fog: false, side: THREE.DoubleSide }));
    this.mesh = new THREE.InstancedMesh(geo, this.material, count);
    this.mesh.frustumCulled = false;
    this.mesh.renderOrder = 6;
    this.mesh.visible = false;
    this.z = new Float32Array(count); this.angle = new Float32Array(count); this.radius = new Float32Array(count); this.len = new Float32Array(count);
    for (let i = 0; i < count; i++) this.respawn(i, -2 - Math.random() * 40);
    camera.add(this.mesh);
  }
  private respawn(i: number, z: number): void {
    this.z[i] = z; this.angle[i] = Math.random() * Math.PI * 2; this.radius[i] = 1.4 + Math.random() * 2.4; this.len[i] = 2 + Math.random() * 5;
  }
  /** `intensity` 0..1 (speed and boost), `speed` the kart's speed in m/s. */
  update(dt: number, intensity: number, speed: number): void {
    this.level = approach(this.level, intensity, intensity > this.level ? 10 : 4, dt);
    this.material.opacity = this.level * 0.42;
    this.mesh.visible = this.level > 0.02;
    if (!this.mesh.visible) return;
    const v = (20 + speed * 1.6) * dt;
    for (let i = 0; i < this.count; i++) {
      this.z[i] += v;
      if (this.z[i] > -1) this.respawn(i, -30 - Math.random() * 14);
      const a = this.angle[i], r = this.radius[i];
      this.p.set(Math.cos(a) * r, Math.sin(a) * r * 0.62, this.z[i]);
      this.s.set(1, 1, this.len[i] * (0.4 + this.level));
      this.mesh.setMatrixAt(i, this.m.compose(this.p, this.q, this.s));
    }
    this.mesh.instanceMatrix.needsUpdate = true;
  }
}

const BOLT_POINTS = 16, BOLT_HEIGHT = 42;
interface Bolt { group: THREE.Group; positions: Float32Array; geo: THREE.BufferGeometry; life: number; core: THREE.MeshBasicMaterial; glow: THREE.MeshBasicMaterial }
/** Lightning strikes: a pool of camera-facing jagged ribbons from high in the sky down to a racer. */
export class Bolts {
  private pool: Bolt[] = [];
  constructor(lib: AssetLibrary, scene: THREE.Scene, size = 8) {
    const idx: number[] = [];
    for (let k = 0; k < BOLT_POINTS - 1; k++) { const a = k * 2; idx.push(a, a + 1, a + 2, a + 1, a + 3, a + 2); }
    for (let n = 0; n < size; n++) {
      const positions = new Float32Array(BOLT_POINTS * 2 * 3);
      const geo = lib.own(new THREE.BufferGeometry());
      geo.setAttribute('position', new THREE.BufferAttribute(positions, 3).setUsage(THREE.DynamicDrawUsage));
      geo.setIndex(idx);
      const core = lib.own(new THREE.MeshBasicMaterial({ color: 0xf2fbff, transparent: true, blending: THREE.AdditiveBlending, depthWrite: false, side: THREE.DoubleSide, toneMapped: false, fog: false }));
      const glow = lib.own(new THREE.MeshBasicMaterial({ color: 0x6fc4ff, transparent: true, blending: THREE.AdditiveBlending, depthWrite: false, side: THREE.DoubleSide, toneMapped: false, fog: false }));
      const group = new THREE.Group();
      const a = new THREE.Mesh(geo, core), b = new THREE.Mesh(geo, glow);
      b.scale.set(5, 1, 1);
      a.frustumCulled = b.frustumCulled = false;
      a.renderOrder = b.renderOrder = 7;
      group.add(b, a);
      group.visible = false;
      scene.add(group);
      this.pool.push({ group, positions, geo, life: 0, core, glow });
    }
  }
  /** Strikes (x, y, z): a fresh random bolt shape in the oldest slot. */
  strike(x: number, y: number, z: number): void {
    let bolt = this.pool[0];
    for (const b of this.pool) if (b.life < bolt.life) bolt = b;
    const p = bolt.positions;
    let jx = 0;
    for (let k = 0; k < BOLT_POINTS; k++) {
      const t = k / (BOLT_POINTS - 1);
      jx = k === BOLT_POINTS - 1 ? 0 : jx * 0.4 + (Math.random() - 0.5) * 3.2 * (1 - t * 0.6);
      const w = 0.16 + (1 - t) * 0.14;
      const h = BOLT_HEIGHT * (1 - t);
      p[k * 6] = jx - w; p[k * 6 + 1] = h; p[k * 6 + 2] = 0;
      p[k * 6 + 3] = jx + w; p[k * 6 + 4] = h; p[k * 6 + 5] = 0;
    }
    (bolt.geo.getAttribute('position') as THREE.BufferAttribute).needsUpdate = true;
    bolt.group.position.set(x, y + 0.5, z);
    bolt.group.visible = true;
    bolt.life = 0.42;
  }
  update(dt: number, camera: THREE.Camera): void {
    for (const b of this.pool) {
      if (b.life <= 0) continue;
      b.life -= dt;
      if (b.life <= 0) { b.group.visible = false; continue; }
      // Face the camera about the vertical; flicker as it fades.
      b.group.rotation.y = Math.atan2(camera.position.x - b.group.position.x, camera.position.z - b.group.position.z);
      const flick = (Math.sin(b.life * 90) > -0.3 ? 1 : 0.35) * Math.min(1, b.life / 0.25);
      b.core.opacity = flick; b.glow.opacity = flick * 0.45;
    }
  }
}

interface Ring { mesh: THREE.Mesh; mat: THREE.MeshBasicMaterial; life: number; max: number; radius: number }
/** Flat expanding shockwave rings on the ground (blasts, boosts, landings). */
export class Rings {
  private pool: Ring[] = [];
  constructor(lib: AssetLibrary, scene: THREE.Scene, size = 10) {
    const geo = lib.own(new THREE.RingGeometry(0.82, 1, 48));
    geo.rotateX(-Math.PI / 2);
    for (let n = 0; n < size; n++) {
      const mat = lib.own(new THREE.MeshBasicMaterial({ transparent: true, blending: THREE.AdditiveBlending, depthWrite: false, toneMapped: false, side: THREE.DoubleSide }));
      const mesh = new THREE.Mesh(geo, mat);
      mesh.visible = false; mesh.renderOrder = 6; mesh.frustumCulled = false;
      scene.add(mesh);
      this.pool.push({ mesh, mat, life: 0, max: 1, radius: 1 });
    }
  }
  spawn(x: number, y: number, z: number, color: number, radius: number, life = 0.5): void {
    let r = this.pool[0];
    for (const c of this.pool) if (c.life < r.life) r = c;
    r.mesh.position.set(x, y + 0.25, z); r.mat.color.setHex(color);
    r.life = r.max = life; r.radius = radius; r.mesh.visible = true;
  }
  update(dt: number): void {
    for (const r of this.pool) {
      if (r.life <= 0) continue;
      r.life -= dt;
      if (r.life <= 0) { r.mesh.visible = false; continue; }
      const t = 1 - r.life / r.max;
      r.mesh.scale.setScalar(0.5 + r.radius * (1 - (1 - t) * (1 - t)));
      r.mat.opacity = (1 - t) * 0.9;
    }
  }
}

const SHELL_COLOR: Record<Exclude<ShellKind, 'bomb'>, number> = { green: 0x2fbf3a, red: 0xe23a2a, blue: 0x2f6fe8 };
/** Shell models: a plated dome over a white rim and a cream belly; the blue shell adds spikes and wings. */
export class ShellKit {
  private dome: THREE.BufferGeometry; private rim: THREE.BufferGeometry; private belly: THREE.BufferGeometry; private spike: THREE.BufferGeometry; private wing: THREE.BufferGeometry;
  private mats: Record<string, THREE.Material> = {};
  constructor(lib: AssetLibrary) {
    this.dome = lib.own(new THREE.SphereGeometry(0.78, 18, 9, 0, Math.PI * 2, 0, Math.PI / 2));
    this.dome.scale(1, 0.85, 1);
    this.rim = lib.own(new THREE.TorusGeometry(0.78, 0.14, 8, 22));
    this.rim.rotateX(Math.PI / 2);
    this.belly = lib.own(new THREE.CylinderGeometry(0.78, 0.6, 0.28, 18));
    this.belly.translate(0, -0.14, 0);
    this.spike = lib.own(new THREE.ConeGeometry(0.13, 0.42, 6));
    this.wing = lib.own(new THREE.BoxGeometry(0.9, 0.05, 0.38));
    const plates = lib.own(shellTexture());
    plates.repeat.set(2, 1);
    for (const [k, c] of Object.entries(SHELL_COLOR)) this.mats[k] = lib.own(new THREE.MeshStandardMaterial({ color: c, map: plates, roughness: 0.35, metalness: 0.05 }));
    this.mats.white = lib.own(new THREE.MeshStandardMaterial({ color: 0xf6f6f2, roughness: 0.5 }));
    this.mats.belly = lib.own(new THREE.MeshStandardMaterial({ color: 0xf2dfa8, roughness: 0.6 }));
  }
  /** A new shell of `kind` (green, red or blue), about 1.6 m across. */
  build(kind: Exclude<ShellKind, 'bomb'>): THREE.Group {
    const g = new THREE.Group();
    const add = (geo: THREE.BufferGeometry, mat: THREE.Material, x = 0, y = 0, z = 0) => { const m = new THREE.Mesh(geo, mat); m.position.set(x, y, z); m.castShadow = true; g.add(m); return m; };
    add(this.dome, this.mats[kind], 0, 0.1, 0);
    add(this.rim, this.mats.white, 0, 0.08, 0);
    add(this.belly, this.mats.belly, 0, 0.06, 0);
    if (kind === 'blue') {
      for (let k = 0; k < 6; k++) { const a = (k / 6) * Math.PI * 2; const s = add(this.spike, this.mats.white, Math.cos(a) * 0.42, 0.72, Math.sin(a) * 0.42); s.rotation.set(Math.sin(a) * 0.5, 0, -Math.cos(a) * 0.5); }
      add(this.spike, this.mats.white, 0, 0.88, 0);
      for (const side of [-1, 1]) { const w = add(this.wing, this.mats.white, side * 1.0, 0.35, -0.1); w.rotation.z = side * 0.35; w.name = 'wing'; }
    }
    return g;
  }
}
