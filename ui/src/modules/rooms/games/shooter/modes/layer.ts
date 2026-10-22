/**
 * Render layer for the gear and the modes, drawn into the shooter scene (one
 * hook in render.ts calls `modeLayer(scene).frame(...)`): the infected
 * (instanced low-poly bodies, one draw call per part, walk / claw / stagger
 * animation, smoothed between snapshots), grenades in flight, smoke clouds,
 * fires and acid, airstrike warnings, attack drones, Domination's capture
 * points (flag, team-coloured ring that fills while capturing), Zombies'
 * buy-walls, landing zone and helicopter, and the effects of the new events.
 * Everything is pooled and sized once; no allocation per frame. Meshes are
 * marked `owned` so GameView.dispose() frees them with the scene.
 */
import * as THREE from 'three';
import { Particles } from '../../common/particles';
import { owned } from '../../common/view';
import { mapFor } from '../map';
import { MAX_DRONES, MAX_FLYING, MAX_AREAS, AREA_ACID, AREA_FIRE, AREA_SMOKE, AREA_STRIKE } from './gear';
import { WAVE_BOARD, WAVE_HELI } from './index';
import { buyWallsFor, MAX_INFECTED, ZOMBIE_KINDS, LZ_R } from '../zombies';
import { teamColour } from '../catalog';
import type { ShooterEvent, ShooterState } from '../sim';

const _o = new THREE.Object3D();
const _m = new THREE.Matrix4();
const _c = new THREE.Color();
/** Skin and clothes per infected kind. */
const SKIN = [0x8fa877, 0xb3c08f, 0x6d7d58, 0xc4cf55];
const CLOTH = [0x55606e, 0x7a4b3a, 0x3e3a36, 0x6a6d3a];
const NADE_COLOURS = [0x3f5a2e, 0xc8452f, 0x9aa0a6, 0xf2f2e8, 0x9bd12e];
/** Body parts: size, pivot height, and offset of the part from its pivot. */
const PARTS: { size: [number, number, number]; pivot: [number, number, number]; hang: number; skin: boolean }[] = [
  { size: [0.56, 0.7, 0.32], pivot: [0, 1.15, 0], hang: 0, skin: false },        // torso
  { size: [0.36, 0.38, 0.36], pivot: [0, 1.66, 0.04], hang: 0, skin: true },     // head
  { size: [0.2, 0.78, 0.22], pivot: [0.15, 0.8, 0], hang: -0.39, skin: false },  // left leg
  { size: [0.2, 0.78, 0.22], pivot: [-0.15, 0.8, 0], hang: -0.39, skin: false }, // right leg
  { size: [0.15, 0.66, 0.15], pivot: [0.36, 1.42, 0], hang: -0.3, skin: true },  // left arm
  { size: [0.15, 0.66, 0.15], pivot: [-0.36, 1.42, 0], hang: -0.3, skin: true }, // right arm
];

interface Shown { x: number; y: number; z: number; yaw: number; phase: number; seen: number }

export class ModeLayer {
  private scene: THREE.Scene;
  private parts: THREE.InstancedMesh[];
  private shown = new Map<number, Shown>();
  private nades: THREE.InstancedMesh;
  private drones: THREE.InstancedMesh;
  private rotors: THREE.InstancedMesh;
  private discs: THREE.Mesh[] = [];
  private discMats: THREE.MeshBasicMaterial[] = [];
  private fx = new Particles(1400, true);
  private haze = new Particles(900, false);
  private lastEvent = 0;
  private time = 0;
  private stamp = 0;
  /** Domination flags, Zombies set pieces: built on the first frame of that mode. */
  private built = '';
  private flags: { ring: THREE.Mesh; fill: THREE.Mesh; mat: THREE.MeshBasicMaterial; fillMat: THREE.MeshBasicMaterial; cloth: THREE.Mesh }[] = [];
  private lz: THREE.Group | null = null;
  private lzBeam: THREE.Mesh | null = null;
  private heli: THREE.Group | null = null;
  private heliRotor: THREE.Mesh | null = null;

  constructor(scene: THREE.Scene) {
    this.scene = scene;
    const skin = new THREE.MeshLambertMaterial({ color: 0xffffff });
    this.parts = PARTS.map(part => {
      const geo = new THREE.BoxGeometry(...part.size).translate(0, part.hang, 0);
      const mesh = new THREE.InstancedMesh(geo, skin, MAX_INFECTED);
      mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
      mesh.count = 0; mesh.frustumCulled = false; mesh.castShadow = true;
      for (let i = 0; i < MAX_INFECTED; i++) mesh.setColorAt(i, _c.setHex(0xffffff));
      scene.add(owned(mesh));
      return mesh;
    });
    this.nades = new THREE.InstancedMesh(new THREE.SphereGeometry(0.14, 8, 6), new THREE.MeshLambertMaterial({ color: 0xffffff }), MAX_FLYING);
    this.nades.count = 0; this.nades.frustumCulled = false;
    for (let i = 0; i < MAX_FLYING; i++) this.nades.setColorAt(i, _c.setHex(0xffffff));
    this.drones = new THREE.InstancedMesh(new THREE.BoxGeometry(0.7, 0.18, 0.7), new THREE.MeshLambertMaterial({ color: 0x2b3240 }), MAX_DRONES);
    this.rotors = new THREE.InstancedMesh(new THREE.CylinderGeometry(0.62, 0.62, 0.03, 16), new THREE.MeshBasicMaterial({ color: 0x9fb4c8, transparent: true, opacity: 0.45 }), MAX_DRONES);
    this.drones.count = 0; this.rotors.count = 0; this.drones.frustumCulled = false; this.rotors.frustumCulled = false;
    scene.add(owned(this.nades), owned(this.drones), owned(this.rotors));
    // Ground discs for clouds / fires / acid / airstrike warnings (pooled).
    const disc = new THREE.RingGeometry(0.82, 1, 40).rotateX(-Math.PI / 2);
    for (let i = 0; i < MAX_AREAS; i++) {
      const mat = new THREE.MeshBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.5, depthWrite: false, side: THREE.DoubleSide });
      const mesh = new THREE.Mesh(disc, mat);
      mesh.visible = false; mesh.renderOrder = 4;
      this.discs.push(mesh); this.discMats.push(mat);
      scene.add(owned(mesh));
    }
    this.fx.points.userData.owned = true; this.haze.points.userData.owned = true;
    scene.add(this.fx.points, this.haze.points);
  }

  frame(state: ShooterState, dt: number): void {
    this.time += dt;
    this.stamp++;
    this.events(state);
    if (this.built !== state.mode + state.map) this.build(state);
    this.infected(state, dt);
    this.gear(state);
    this.objectives(state, dt);
    this.fx.update(dt); this.haze.update(dt);
  }

  private events(state: ShooterState): void {
    for (const e of state.events) {
      if (e.id <= this.lastEvent) continue;
      this.lastEvent = e.id;
      this.effect(e);
    }
    if (state.events.length && state.events[state.events.length - 1].id < this.lastEvent - 1000) this.lastEvent = 0;
  }

  private effect(e: ShooterEvent): void {
    switch (e.type) {
      case 'zhit': this.fx.burst(e.x, e.y, e.z, e.flag & 2 ? 10 : 6, 0x7fbf3a, 3.5, 0.3, 0.35); break;
      case 'zkill':
        this.fx.burst(e.x, e.y + 1, e.z, 34, 0x6fae2a, 5, 0.45, 0.8, -9);
        for (let i = 0; i < 6; i++) this.haze.spawn(e.x, e.y + 0.6, e.z, (Math.random() - 0.5) * 2, 0.6, (Math.random() - 0.5) * 2, 0x55663f, 1.4, 0.9, 0, 1.6);
        break;
      case 'flash': if (e.target < 0) this.fx.burst(e.x, e.y + 0.3, e.z, 70, 0xffffff, 10, 0.9, 0.25, 0); break;
      case 'fire': this.fx.burst(e.x, e.y + 0.3, e.z, 40, e.flag === 1 ? 0x9bd12e : 0xff8a2a, 5, 0.5, 0.6); break;
      case 'smoke': for (let i = 0; i < 24; i++) this.haze.spawn(e.x, e.y + 0.4, e.z, (Math.random() - 0.5) * 6, 1 + Math.random(), (Math.random() - 0.5) * 6, 0xd5d8dc, 3, 2.5, 0, 1.8); break;
      case 'mantle': case 'slide': for (let i = 0; i < 6; i++) this.haze.spawn(e.x, e.y + 0.1, e.z, (Math.random() - 0.5) * 3, 0.5, (Math.random() - 0.5) * 3, 0xcbb894, 0.9, 0.6, 0, 2); break;
      case 'capture': this.fx.burst(e.x, e.y + 2, e.z, 60, teamColour(e.flag), 7, 0.6, 1); break;
      case 'repair': this.fx.burst(e.x, e.y, e.z, 12, 0x8fd0ff, 3, 0.35, 0.45); break;
      case 'buy': if (e.flag === 1) this.fx.burst(e.x, e.y + 1.2, e.z, 40, 0xffd34d, 5, 0.5, 0.8); break;
      case 'spawn': this.fx.burst(e.x, e.y + 1, e.z, 30, 0x8fd0ff, 4, 0.5, 0.7, -2); break;
      case 'streak': if (e.flag === 1) this.fx.burst(e.x, e.y + 2, e.z, 30, 0xffd34d, 6, 0.45, 0.6); break;
      default: break;
    }
  }

  /** Infected bodies: smoothed toward the latest positions, animated per kind. */
  private infected(state: ShooterState, dt: number): void {
    const list = state.infected, n = Math.min(list.length, MAX_INFECTED);
    const k = 1 - Math.exp(-14 * dt);
    for (let i = 0; i < n; i++) {
      const z = list[i];
      let s = this.shown.get(z.id);
      if (!s) { s = { x: z.x, y: z.y, z: z.z, yaw: z.yaw, phase: (z.id % 17) * 0.4, seen: 0 }; this.shown.set(z.id, s); }
      const moved = Math.hypot(z.x - s.x, z.z - s.z);
      if (moved > 8) { s.x = z.x; s.y = z.y; s.z = z.z; } else { s.x += (z.x - s.x) * k; s.y += (z.y - s.y) * k; s.z += (z.z - s.z) * k; }
      let dy = z.yaw - s.yaw; dy = Math.atan2(Math.sin(dy), Math.cos(dy)); s.yaw += dy * k;
      s.seen = this.stamp;
      const kind = ZOMBIE_KINDS[z.kind], scale = kind.scale;
      // Walk cycle speed from the kind (runners sprint); a stagger when stunned; a two-handed claw when swinging.
      s.phase += dt * (z.stun > 0 ? 2 : kind.speed * 1.6);
      const swing = Math.sin(s.phase), claw = z.swing > 0 ? Math.sin((1 - z.swing / 0.45) * Math.PI) : 0;
      const lean = z.kind === 1 ? 0.35 : z.kind === 2 ? 0.15 : 0.1;
      const hurt = z.hp < z.max * 0.35 ? 0.15 : 0;
      for (let p = 0; p < PARTS.length; p++) {
        const part = PARTS[p];
        _o.position.set(part.pivot[0] * scale, part.pivot[1] * scale, part.pivot[2] * scale);
        _o.rotation.set(0, 0, 0);
        if (p === 0) _o.rotation.x = lean + hurt;
        else if (p === 1) { _o.rotation.x = lean * 1.4; _o.rotation.z = Math.sin(s.phase * 0.5) * 0.25; }
        else if (p === 2 || p === 3) _o.rotation.x = swing * (p === 2 ? 0.55 : -0.55);
        else _o.rotation.x = -1.35 + claw * 1.2 + (p === 4 ? swing : -swing) * 0.15;
        _o.scale.setScalar(scale * (z.kind === 3 && p === 0 ? 1.18 : 1));
        _o.updateMatrix();
        _m.makeRotationY(s.yaw).setPosition(s.x, s.y, s.z).multiply(_o.matrix);
        this.parts[p].setMatrixAt(i, _m);
        if (this.parts[p].instanceColor) {
          // Stunned by a flash: blanched.
          this.parts[p].setColorAt(i, _c.setHex(z.stun > 0 ? 0xe8ecd8 : part.skin ? SKIN[z.kind] : CLOTH[z.kind]));
        }
      }
      // Spitters drip; brutes stomp dust.
      if (z.kind === 3 && Math.random() < dt * 3) this.fx.spawn(s.x, s.y + 1.4 * scale, s.z, 0, -1, 0, 0x9bd12e, 0.18, 0.5, -6);
    }
    for (const mesh of this.parts) { mesh.count = n; mesh.instanceMatrix.needsUpdate = true; if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true; }
    if (this.stamp % 60 === 0) for (const [id, s] of this.shown) if (s.seen !== this.stamp && this.stamp - s.seen > 120) this.shown.delete(id);
  }

  /** Grenades, clouds / fires / acid / airstrike marks, drones. */
  private gear(state: ShooterState): void {
    const nn = Math.min(state.nades.length, MAX_FLYING);
    for (let i = 0; i < nn; i++) {
      const g = state.nades[i];
      _m.makeScale(g.kind === 1 ? 1.2 : 1, g.kind === 1 ? 1.6 : 1, g.kind === 1 ? 1.2 : 1).setPosition(g.x, g.y, g.z);
      this.nades.setMatrixAt(i, _m);
      this.nades.setColorAt(i, _c.setHex(NADE_COLOURS[g.kind] ?? 0xffffff));
      if (g.kind === 1 || g.kind === 4) this.fx.spawn(g.x, g.y, g.z, 0, 0.3, 0, g.kind === 1 ? 0xff9a3a : 0x9bd12e, 0.25, 0.25);
    }
    this.nades.count = nn; this.nades.instanceMatrix.needsUpdate = true;
    if (this.nades.instanceColor) this.nades.instanceColor.needsUpdate = true;
    for (let i = 0; i < this.discs.length; i++) {
      const a = state.areas[i], d = this.discs[i];
      if (!a) { d.visible = false; continue; }
      d.visible = true;
      d.position.set(a.x, a.y + 0.08, a.z);
      const pulse = 1 + Math.sin(this.time * 6) * 0.04;
      d.scale.setScalar(a.r * (a.kind === AREA_STRIKE ? pulse : 1));
      const m = this.discMats[i];
      m.color.setHex(a.kind === AREA_FIRE ? 0xff6a1a : a.kind === AREA_ACID ? 0x9bd12e : a.kind === AREA_STRIKE ? 0xff3b30 : 0xd5d8dc);
      m.opacity = a.kind === AREA_STRIKE ? 0.45 + Math.sin(this.time * 12) * 0.25 : a.kind === AREA_SMOKE ? 0.15 : 0.55;
      // Clouds billow, fires flicker, acid bubbles: a few particles per frame inside the radius.
      const rr = a.r * Math.sqrt(Math.random()), aa = Math.random() * Math.PI * 2, px = a.x + Math.cos(aa) * rr, pz = a.z + Math.sin(aa) * rr;
      if (a.kind === AREA_SMOKE) this.haze.spawn(px, a.y + 0.5 + Math.random() * 2.5, pz, (Math.random() - 0.5) * 0.6, 0.25, (Math.random() - 0.5) * 0.6, 0xd8dbe0, 4.2, 2.6, 0, 0.6);
      else if (a.kind === AREA_FIRE) { this.fx.spawn(px, a.y + 0.2, pz, 0, 2 + Math.random() * 1.5, 0, Math.random() < 0.5 ? 0xff7a1a : 0xffc23a, 0.5, 0.5, 1); if (Math.random() < 0.3) this.haze.spawn(px, a.y + 1.2, pz, 0, 1.2, 0, 0x3a3a3a, 1.6, 1.4, 0, 0.8); }
      else if (a.kind === AREA_ACID) this.fx.spawn(px, a.y + 0.1, pz, 0, 0.8, 0, 0x9bd12e, 0.3, 0.4, 0);
    }
    const dn = Math.min(state.drones.length, MAX_DRONES);
    for (let i = 0; i < dn; i++) {
      const d = state.drones[i];
      _m.makeRotationY(d.yaw).setPosition(d.x, d.y, d.z);
      this.drones.setMatrixAt(i, _m);
      _m.makeRotationY(this.time * 30 + i).setPosition(d.x, d.y + 0.14, d.z);
      this.rotors.setMatrixAt(i, _m);
      if (Math.random() < 0.3) this.fx.spawn(d.x, d.y - 0.15, d.z, 0, 0, 0, 0xff3b30, 0.18, 0.12);
    }
    this.drones.count = dn; this.rotors.count = dn;
    this.drones.instanceMatrix.needsUpdate = true; this.rotors.instanceMatrix.needsUpdate = true;
  }

  /** Builds the static pieces of a mode once: Domination flags, Zombies buy-walls and landing zone. */
  private build(state: ShooterState): void {
    this.built = state.mode + state.map;
    if (state.mode === 'dom' && state.obj.points.length && !this.flags.length) {
      const pole = new THREE.CylinderGeometry(0.06, 0.06, 5, 8).translate(0, 2.5, 0);
      const cloth = new THREE.PlaneGeometry(1.4, 0.9).translate(0.72, 4.4, 0);
      const ring = new THREE.RingGeometry(0.92, 1, 48).rotateX(-Math.PI / 2);
      const fill = new THREE.CircleGeometry(1, 48).rotateX(-Math.PI / 2);
      for (const pt of state.obj.points) {
        const g = new THREE.Group();
        const mat = new THREE.MeshBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.8, depthWrite: false, side: THREE.DoubleSide });
        const fillMat = new THREE.MeshBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.22, depthWrite: false, side: THREE.DoubleSide });
        const r = new THREE.Mesh(ring, mat), f = new THREE.Mesh(fill, fillMat);
        r.scale.setScalar(pt.r); r.position.y = 0.1; f.position.y = 0.09;
        const p = new THREE.Mesh(pole, new THREE.MeshLambertMaterial({ color: 0xd8dde3 }));
        const c = new THREE.Mesh(cloth, new THREE.MeshLambertMaterial({ color: 0xffffff, side: THREE.DoubleSide }));
        g.add(r, f, p, c);
        g.position.set(pt.x, pt.y, pt.z);
        this.scene.add(owned(g));
        this.flags.push({ ring: r, fill: f, mat, fillMat, cloth: c });
      }
    }
    if (state.mode === 'zombies' && !this.lz) {
      // Buy-walls: a lit panel with a gold frame where the gun hangs.
      const panel = new THREE.BoxGeometry(1.6, 1, 0.12), frame = new THREE.BoxGeometry(1.8, 1.2, 0.08);
      const panelMat = new THREE.MeshLambertMaterial({ color: 0x2b2f36, emissive: 0x1b2a1b }), frameMat = new THREE.MeshBasicMaterial({ color: 0xffc23a });
      for (const w of buyWallsFor(mapFor(state.map))) {
        const g = new THREE.Group();
        const f = new THREE.Mesh(frame, frameMat), p = new THREE.Mesh(panel, panelMat);
        f.position.y = 1.5; p.position.set(0, 1.5, 0.03);
        const post = new THREE.Mesh(new THREE.BoxGeometry(0.12, 1.5, 0.12).translate(0, 0.75, -0.08), panelMat);
        g.add(post, f, p);
        g.position.set(w.x, w.y, w.z);
        g.rotation.y = w.yaw;
        this.scene.add(owned(g));
      }
      // Landing zone: a ring and a beacon (lit once the helicopter is called), and the helicopter itself.
      const lz = new THREE.Group();
      const ring = new THREE.Mesh(new THREE.RingGeometry(LZ_R - 0.6, LZ_R, 64).rotateX(-Math.PI / 2), new THREE.MeshBasicMaterial({ color: 0x62e0ff, transparent: true, opacity: 0.7, depthWrite: false, side: THREE.DoubleSide }));
      ring.position.y = 0.12;
      const h = new THREE.Mesh(new THREE.RingGeometry(2.4, 3, 4, 1, Math.PI / 4).rotateX(-Math.PI / 2), new THREE.MeshBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.8, depthWrite: false }));
      h.position.y = 0.13;
      const beam = new THREE.Mesh(new THREE.CylinderGeometry(0.6, 1.4, 120, 16, 1, true).translate(0, 60, 0), new THREE.MeshBasicMaterial({ color: 0x62e0ff, transparent: true, opacity: 0.22, blending: THREE.AdditiveBlending, depthWrite: false, side: THREE.DoubleSide }));
      lz.add(ring, h, beam);
      lz.position.set(state.obj.lzX, state.obj.lzY, state.obj.lzZ);
      lz.visible = false;
      this.scene.add(owned(lz));
      this.lz = lz; this.lzBeam = beam;
      const heli = new THREE.Group();
      const hull = new THREE.MeshLambertMaterial({ color: 0x3d4a3a });
      const body = new THREE.Mesh(new THREE.BoxGeometry(2.6, 2.2, 5.2), hull);
      const tail = new THREE.Mesh(new THREE.BoxGeometry(0.5, 0.6, 5).translate(0, 0.5, -4.8), hull);
      const glass = new THREE.Mesh(new THREE.BoxGeometry(2.2, 1.2, 1.2).translate(0, 0.3, 2.6), new THREE.MeshLambertMaterial({ color: 0x9fd2ff, emissive: 0x16304a }));
      const skid = new THREE.Mesh(new THREE.BoxGeometry(2.8, 0.12, 4.6).translate(0, -1.4, 0), new THREE.MeshLambertMaterial({ color: 0x222222 }));
      const rotor = new THREE.Mesh(new THREE.BoxGeometry(11, 0.08, 0.4), new THREE.MeshLambertMaterial({ color: 0x1e1e1e }));
      rotor.position.y = 1.35;
      heli.add(body, tail, glass, skid, rotor);
      heli.visible = false;
      this.scene.add(owned(heli));
      this.heli = heli; this.heliRotor = rotor;
    }
  }

  /** Domination rings fill toward the capturing team's colour; the helicopter flies in, lands and leaves. */
  private objectives(state: ShooterState, dt: number): void {
    state.obj.points.forEach((pt, i) => {
      const f = this.flags[i];
      if (!f) return;
      const col = pt.own >= 0 ? teamColour(pt.own) : pt.by >= 0 ? teamColour(pt.by) : 0xffffff;
      f.mat.color.setHex(pt.own >= 0 ? teamColour(pt.own) : 0xffffff);
      f.fillMat.color.setHex(col);
      f.fill.scale.setScalar(Math.max(0.01, pt.r * (pt.own >= 0 ? 1 : pt.cap)));
      (f.cloth.material as THREE.MeshLambertMaterial).color.setHex(pt.own >= 0 ? teamColour(pt.own) : 0xf2f2f2);
      f.cloth.rotation.y = Math.sin(this.time * 2 + i) * 0.25;
    });
    if (!this.lz || !this.heli || state.mode !== 'zombies') return;
    const o = state.obj, inbound = o.wave === WAVE_HELI, board = o.wave === WAVE_BOARD;
    this.lz.visible = inbound || board || (o.round >= o.lastRound - 1 && o.round > 0);
    if (this.lzBeam) (this.lzBeam.material as THREE.MeshBasicMaterial).opacity = inbound || board ? 0.2 + Math.sin(this.time * 3) * 0.06 : 0.06;
    // Inbound: approaching from 300 m out and 60 m up over the last 40 s; boarding: on the pad; gone after.
    const h = this.heli;
    if (inbound || board || (o.over >= 0 && o.aboard > 0)) {
      h.visible = true;
      const t = inbound ? Math.min(1, o.timer / 40) : 0, leave = o.over >= 0 ? Math.min(1, o.over / 3) : 0;
      h.position.set(o.lzX + t * 300 + leave * 40, o.lzY + 1.5 + t * 60 + leave * 50, o.lzZ - t * 80);
      h.rotation.y = -Math.PI / 2 + Math.sin(this.time * 0.8) * 0.05;
      if (this.heliRotor) this.heliRotor.rotation.y += dt * 40;
      if (board && Math.random() < 0.6) this.haze.spawn(o.lzX + (Math.random() - 0.5) * 10, o.lzY + 0.3, o.lzZ + (Math.random() - 0.5) * 10, (Math.random() - 0.5) * 8, 0.5, (Math.random() - 0.5) * 8, 0xcbb894, 1.8, 0.8, 0, 2);
    } else h.visible = false;
  }
}

const layers = new WeakMap<THREE.Scene, ModeLayer>();
/** The layer for a scene (created on first use; freed with the scene's owned meshes). */
export function modeLayer(scene: THREE.Scene): ModeLayer {
  let l = layers.get(scene);
  if (!l) { l = new ModeLayer(scene); layers.set(scene, l); }
  return l;
}
