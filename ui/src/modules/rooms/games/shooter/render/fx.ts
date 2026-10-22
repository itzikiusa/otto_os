/**
 * Combat effects for Storm Royale, all pooled and instanced (fixed draw
 * calls, no per-frame allocation): tracer streaks that fly down the shot,
 * muzzle-flash stars, decals (bullet holes, blood splats, scorch marks) laid
 * flat on the surface they hit, tumbling debris chunks that bounce on the
 * ground, explosion fireballs with a ground shockwave, and a small pool of
 * point lights for muzzle / explosion flashes (always in the scene, so the
 * light count never changes and no shader recompiles).
 */
import * as THREE from 'three';
import { decalTexture, flashTexture } from '../../common/textures';

const TRACERS = 72, FLASHES = 24, HOLES = 220, SPLATS = 72, SCORCH = 40, DEBRIS = 220, BALLS = 8, LIGHTS = 3;
const TRACER_SPEED = 520, STREAK = 7;
const _m = new THREE.Matrix4(), _m2 = new THREE.Matrix4(), _q = new THREE.Quaternion(), _p = new THREE.Vector3(), _s = new THREE.Vector3(), _d = new THREE.Vector3(), _c = new THREE.Color();
const FWD = new THREE.Vector3(0, 0, 1), ZERO = new THREE.Matrix4().makeScale(0, 0, 0);

/** A ring buffer of flat decals on one InstancedMesh. */
class Decals {
  readonly mesh: THREE.InstancedMesh;
  private next = 0;
  constructor(tex: THREE.Texture, count: number, blend: THREE.Blending = THREE.NormalBlending) {
    const mat = new THREE.MeshStandardMaterial({ map: tex, transparent: true, depthWrite: false, polygonOffset: true, polygonOffsetFactor: -4, polygonOffsetUnits: -4, roughness: 0.9, blending: blend });
    this.mesh = new THREE.InstancedMesh(new THREE.PlaneGeometry(1, 1), mat, count);
    for (let i = 0; i < count; i++) this.mesh.setMatrixAt(i, ZERO);
    this.mesh.frustumCulled = false; this.mesh.receiveShadow = true; this.mesh.renderOrder = 2;
  }
  add(x: number, y: number, z: number, nx: number, ny: number, nz: number, size: number, spin: number): void {
    _d.set(nx, ny, nz);
    if (_d.lengthSq() < 1e-6) _d.set(0, 1, 0);
    _d.normalize();
    _q.setFromUnitVectors(FWD, _d);
    _m.makeRotationZ(spin).premultiply(_m2.makeRotationFromQuaternion(_q));
    _m.scale(_s.set(size, size, size)).setPosition(x + _d.x * 0.02, y + _d.y * 0.02, z + _d.z * 0.02);
    this.mesh.setMatrixAt(this.next, _m);
    this.mesh.instanceMatrix.needsUpdate = true;
    this.next = (this.next + 1) % this.mesh.count;
  }
  dispose(): void { this.mesh.geometry.dispose(); (this.mesh.material as THREE.MeshStandardMaterial).map?.dispose(); (this.mesh.material as THREE.Material).dispose(); this.mesh.dispose(); }
}

export class Fx {
  readonly group = new THREE.Group();
  private tracers: THREE.InstancedMesh;
  private tr = { ax: new Float32Array(TRACERS), ay: new Float32Array(TRACERS), az: new Float32Array(TRACERS), dx: new Float32Array(TRACERS), dy: new Float32Array(TRACERS), dz: new Float32Array(TRACERS), len: new Float32Array(TRACERS), t: new Float32Array(TRACERS).fill(-1) };
  private trNext = 0;
  private flashes: THREE.InstancedMesh;
  private flashLife = new Float32Array(FLASHES);
  private flashNext = 0;
  private holes: Decals; private splats: Decals; private scorches: Decals;
  private debris: THREE.InstancedMesh;
  private db = { p: new Float32Array(DEBRIS * 3), v: new Float32Array(DEBRIS * 3), r: new Float32Array(DEBRIS * 3), w: new Float32Array(DEBRIS * 3), s: new Float32Array(DEBRIS), life: new Float32Array(DEBRIS) };
  private dbNext = 0;
  private balls: THREE.InstancedMesh; private rings: THREE.InstancedMesh;
  private ball = { x: new Float32Array(BALLS), y: new Float32Array(BALLS), z: new Float32Array(BALLS), r: new Float32Array(BALLS), t: new Float32Array(BALLS).fill(-1), g: new Float32Array(BALLS) };
  private ballNext = 0;
  private lights: THREE.PointLight[] = [];
  private lightLife = new Float32Array(LIGHTS); private lightMax = new Float32Array(LIGHTS); private lightPeak = new Float32Array(LIGHTS);
  private lightNext = 0;
  private resources: { dispose(): void }[] = [];

  constructor() {
    const trGeo = new THREE.BoxGeometry(1, 1, 1).translate(0, 0, -0.5);
    const trMat = new THREE.MeshBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.95, blending: THREE.AdditiveBlending, depthWrite: false, fog: false });
    this.tracers = this.instanced(trGeo, trMat, TRACERS, false);
    // Three crossed quads along the barrel axis, plus one facing it.
    const quads: THREE.BufferGeometry[] = [];
    for (let k = 0; k < 3; k++) quads.push(new THREE.PlaneGeometry(1, 1.8).translate(0, 0.9, 0).rotateX(Math.PI / 2).rotateZ((k * Math.PI) / 3));
    quads.push(new THREE.PlaneGeometry(1.1, 1.1));
    const flashGeo = mergeQuads(quads);
    const flashMat = new THREE.MeshBasicMaterial({ map: flashTexture(), transparent: true, blending: THREE.AdditiveBlending, depthWrite: false, side: THREE.DoubleSide, fog: false });
    this.flashes = this.instanced(flashGeo, flashMat, FLASHES, false);
    this.holes = new Decals(decalTexture('hole'), HOLES);
    this.splats = new Decals(decalTexture('blood'), SPLATS);
    this.scorches = new Decals(decalTexture('scorch'), SCORCH);
    this.group.add(this.holes.mesh, this.splats.mesh, this.scorches.mesh);
    const dbMat = new THREE.MeshStandardMaterial({ roughness: 0.9 });
    this.debris = this.instanced(new THREE.BoxGeometry(1, 1, 1), dbMat, DEBRIS, true);
    this.debris.setColorAt(0, _c.setRGB(1, 1, 1));
    const ballMat = new THREE.MeshBasicMaterial({ color: 0xffffff, transparent: true, blending: THREE.AdditiveBlending, depthWrite: false, fog: false });
    this.balls = this.instanced(new THREE.IcosahedronGeometry(1, 2), ballMat, BALLS, false);
    this.balls.setColorAt(0, _c.setRGB(1, 1, 1));
    const ringMat = new THREE.MeshBasicMaterial({ color: 0xffffff, transparent: true, blending: THREE.AdditiveBlending, depthWrite: false, side: THREE.DoubleSide });
    this.rings = this.instanced(new THREE.RingGeometry(0.8, 1, 40).rotateX(-Math.PI / 2), ringMat, BALLS, false);
    this.rings.setColorAt(0, _c.setRGB(1, 1, 1));
    for (let i = 0; i < LIGHTS; i++) {
      const l = new THREE.PointLight(0xffb060, 0, 30, 1.6);
      this.lights.push(l);
      this.group.add(l);
    }
    this.tracers.setColorAt(0, _c.setRGB(1, 1, 1));
    this.resources.push(trGeo, trMat, flashGeo, flashMat, flashMat.map!, dbMat, ballMat, ringMat, this.holes, this.splats, this.scorches);
  }

  private instanced(geo: THREE.BufferGeometry, mat: THREE.Material, n: number, shadows: boolean): THREE.InstancedMesh {
    const m = new THREE.InstancedMesh(geo, mat, n);
    m.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    for (let i = 0; i < n; i++) m.setMatrixAt(i, ZERO);
    m.frustumCulled = false; m.castShadow = shadows; m.renderOrder = 6;
    this.group.add(m);
    this.resources.push(m.geometry);
    return m;
  }

  /** A tracer streak from a to b in `color`, flying at bullet speed. */
  tracer(ax: number, ay: number, az: number, bx: number, by: number, bz: number, color = 0xffd890): void {
    const i = this.trNext, t = this.tr;
    this.trNext = (i + 1) % TRACERS;
    const dx = bx - ax, dy = by - ay, dz = bz - az, len = Math.hypot(dx, dy, dz);
    if (len < 0.5) return;
    t.ax[i] = ax; t.ay[i] = ay; t.az[i] = az; t.dx[i] = dx / len; t.dy[i] = dy / len; t.dz[i] = dz / len; t.len[i] = len; t.t[i] = 0;
    this.tracers.setColorAt(i, _c.setHex(color));
    this.tracers.instanceColor!.needsUpdate = true;
  }

  /** A muzzle-flash star at (x, y, z) pointing along (dx, dy, dz). */
  muzzle(x: number, y: number, z: number, dx: number, dy: number, dz: number, size = 0.35): void {
    const i = this.flashNext;
    this.flashNext = (i + 1) % FLASHES;
    _d.set(dx, dy, dz).normalize();
    _q.setFromUnitVectors(FWD, _d);
    _m.compose(_p.set(x, y, z), _q, _s.set(size * (0.8 + Math.random() * 0.5), size * (0.8 + Math.random() * 0.5), size * (0.9 + Math.random() * 0.6)));
    this.flashes.setMatrixAt(i, _m);
    this.flashes.instanceMatrix.needsUpdate = true;
    this.flashLife[i] = 0.05;
  }

  /** A bullet hole (or blood / scorch) on the surface with normal (nx, ny, nz). */
  decal(kind: 'hole' | 'blood' | 'scorch', x: number, y: number, z: number, nx: number, ny: number, nz: number, size: number): void {
    (kind === 'hole' ? this.holes : kind === 'blood' ? this.splats : this.scorches).add(x, y, z, nx, ny, nz, size, Math.random() * Math.PI * 2);
  }

  /** `count` chunks thrown from (x, y, z) at up to `speed` m/s, `size` m across. */
  burst(x: number, y: number, z: number, count: number, color: number, speed: number, size: number): void {
    const d = this.db;
    for (let k = 0; k < count; k++) {
      const i = this.dbNext;
      this.dbNext = (i + 1) % DEBRIS;
      const a = Math.random() * Math.PI * 2, up = 0.35 + Math.random() * 0.65, s = speed * (0.35 + Math.random() * 0.65);
      d.p[i * 3] = x; d.p[i * 3 + 1] = y; d.p[i * 3 + 2] = z;
      d.v[i * 3] = Math.cos(a) * s * (1 - up * 0.5); d.v[i * 3 + 1] = up * s; d.v[i * 3 + 2] = Math.sin(a) * s * (1 - up * 0.5);
      d.r[i * 3] = Math.random() * 6; d.r[i * 3 + 1] = Math.random() * 6; d.r[i * 3 + 2] = Math.random() * 6;
      d.w[i * 3] = (Math.random() - 0.5) * 14; d.w[i * 3 + 1] = (Math.random() - 0.5) * 14; d.w[i * 3 + 2] = (Math.random() - 0.5) * 14;
      d.s[i] = size * (0.4 + Math.random() * 0.8); d.life[i] = 2.5 + Math.random() * 2.5;
      this.debris.setColorAt(i, _c.setHex(color).multiplyScalar(0.7 + Math.random() * 0.5));
    }
    this.debris.instanceColor!.needsUpdate = true;
  }

  /** A fireball of radius `r` with a ground shockwave at height `ground`. */
  fireball(x: number, y: number, z: number, r: number, ground: number): void {
    const i = this.ballNext, b = this.ball;
    this.ballNext = (i + 1) % BALLS;
    b.x[i] = x; b.y[i] = y; b.z[i] = z; b.r[i] = r; b.t[i] = 0; b.g[i] = ground;
  }

  /** A short point-light flash (muzzle, explosion, lightning). */
  flash(x: number, y: number, z: number, intensity: number, color: number, life: number, range = 30): void {
    // Reuse the weakest light.
    let i = this.lightNext, weakest = Infinity;
    for (let k = 0; k < LIGHTS; k++) { const v = this.lightLife[k] > 0 ? this.lights[k].intensity : 0; if (v < weakest) { weakest = v; i = k; } }
    this.lightNext = (i + 1) % LIGHTS;
    const l = this.lights[i];
    l.position.set(x, y, z); l.color.setHex(color); l.distance = range;
    this.lightLife[i] = life; this.lightMax[i] = life; this.lightPeak[i] = intensity;
    l.intensity = intensity;
  }

  update(dt: number, ground: (x: number, z: number) => number): void {
    // Tracers: the head flies at bullet speed, the tail trails by up to STREAK metres.
    const t = this.tr;
    let live = false;
    for (let i = 0; i < TRACERS; i++) {
      if (t.t[i] < 0) continue;
      t.t[i] += dt;
      const head = Math.min(t.len[i], t.t[i] * TRACER_SPEED), tail = Math.max(0, head - STREAK);
      if (tail >= t.len[i] - 0.01 || t.t[i] > 0.6) { t.t[i] = -1; this.tracers.setMatrixAt(i, ZERO); live = true; continue; }
      _d.set(t.dx[i], t.dy[i], t.dz[i]);
      _q.setFromUnitVectors(FWD, _d);
      const w = 0.03 + Math.min(0.03, head * 0.0004);
      _m.compose(_p.set(t.ax[i] + t.dx[i] * head, t.ay[i] + t.dy[i] * head, t.az[i] + t.dz[i] * head), _q, _s.set(w, w, Math.max(0.05, head - tail)));
      this.tracers.setMatrixAt(i, _m);
      live = true;
    }
    if (live) this.tracers.instanceMatrix.needsUpdate = true;
    let flashing = false;
    for (let i = 0; i < FLASHES; i++) {
      if (this.flashLife[i] <= 0) continue;
      this.flashLife[i] -= dt;
      if (this.flashLife[i] <= 0) this.flashes.setMatrixAt(i, ZERO);
      flashing = true;
    }
    if (flashing) this.flashes.instanceMatrix.needsUpdate = true;
    // Debris: gravity, drag, bounce on the ground, shrink away at the end.
    const d = this.db;
    let tumbling = false;
    for (let i = 0; i < DEBRIS; i++) {
      if (d.life[i] <= 0) continue;
      d.life[i] -= dt;
      const o = i * 3;
      if (d.life[i] <= 0) { this.debris.setMatrixAt(i, ZERO); tumbling = true; continue; }
      d.v[o + 1] -= 18 * dt;
      d.p[o] += d.v[o] * dt; d.p[o + 1] += d.v[o + 1] * dt; d.p[o + 2] += d.v[o + 2] * dt;
      const gy = ground(d.p[o], d.p[o + 2]) + d.s[i] * 0.4;
      if (d.p[o + 1] < gy) {
        d.p[o + 1] = gy;
        if (d.v[o + 1] < -1.5) { d.v[o + 1] *= -0.35; d.v[o] *= 0.6; d.v[o + 2] *= 0.6; d.w[o] *= 0.6; d.w[o + 1] *= 0.6; d.w[o + 2] *= 0.6; }
        else { d.v[o] = d.v[o + 1] = d.v[o + 2] = 0; d.w[o] = d.w[o + 1] = d.w[o + 2] = 0; }
      }
      d.r[o] += d.w[o] * dt; d.r[o + 1] += d.w[o + 1] * dt; d.r[o + 2] += d.w[o + 2] * dt;
      const k = d.s[i] * Math.min(1, d.life[i] / 0.6);
      _q.setFromEuler(_e.set(d.r[o], d.r[o + 1], d.r[o + 2]));
      _m.compose(_p.set(d.p[o], d.p[o + 1], d.p[o + 2]), _q, _s.set(k, k * 0.7, k * 0.9));
      this.debris.setMatrixAt(i, _m);
      tumbling = true;
    }
    if (tumbling) this.debris.instanceMatrix.needsUpdate = true;
    // Fireballs: swell fast, cool from white-yellow to dark red, fade; the shockwave races out along the ground.
    const b = this.ball;
    let burning = false;
    for (let i = 0; i < BALLS; i++) {
      if (b.t[i] < 0) continue;
      b.t[i] += dt;
      const k = b.t[i] / 0.9;
      if (k >= 1) { b.t[i] = -1; this.balls.setMatrixAt(i, ZERO); this.rings.setMatrixAt(i, ZERO); burning = true; continue; }
      const r = b.r[i] * (0.35 + Math.min(1, k * 4) * 0.75), heat = 1 - k;
      _m.compose(_p.set(b.x[i], b.y[i] + k * b.r[i] * 0.8, b.z[i]), _q.identity(), _s.set(r, r * 0.9, r));
      this.balls.setMatrixAt(i, _m);
      this.balls.setColorAt(i, _c.setRGB(heat * 1.2, heat * heat * 0.75, heat * heat * heat * 0.3));
      const rr = b.r[i] * (0.5 + k * 3.2);
      _m.compose(_p.set(b.x[i], b.g[i] + 0.15, b.z[i]), _q.identity(), _s.set(rr, 1, rr));
      this.rings.setMatrixAt(i, _m);
      this.rings.setColorAt(i, _c.setRGB(0.55 * heat, 0.45 * heat, 0.35 * heat));
      burning = true;
    }
    if (burning) {
      this.balls.instanceMatrix.needsUpdate = true; this.rings.instanceMatrix.needsUpdate = true;
      this.balls.instanceColor!.needsUpdate = true; this.rings.instanceColor!.needsUpdate = true;
    }
    for (let i = 0; i < LIGHTS; i++) {
      if (this.lightLife[i] <= 0) { this.lights[i].intensity = 0; continue; }
      this.lightLife[i] -= dt;
      const k = Math.max(0, this.lightLife[i] / this.lightMax[i]);
      this.lights[i].intensity = this.lightPeak[i] * k * k;
    }
  }

  dispose(): void {
    for (const r of this.resources) r.dispose();
    for (const m of [this.tracers, this.flashes, this.debris, this.balls, this.rings]) m.dispose();
  }
}
const _e = new THREE.Euler();

/** Merges same-layout quads (position / normal / uv, indexed) into one geometry. */
function mergeQuads(list: THREE.BufferGeometry[]): THREE.BufferGeometry {
  const pos: number[] = [], nor: number[] = [], uv: number[] = [], idx: number[] = [];
  for (const g of list) {
    const base = pos.length / 3;
    pos.push(...(g.getAttribute('position').array as Float32Array));
    nor.push(...(g.getAttribute('normal').array as Float32Array));
    uv.push(...(g.getAttribute('uv').array as Float32Array));
    for (const i of g.getIndex()!.array as Uint16Array) idx.push(base + i);
    g.dispose();
  }
  const out = new THREE.BufferGeometry();
  out.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
  out.setAttribute('normal', new THREE.Float32BufferAttribute(nor, 3));
  out.setAttribute('uv', new THREE.Float32BufferAttribute(uv, 2));
  out.setIndex(idx);
  return out;
}
