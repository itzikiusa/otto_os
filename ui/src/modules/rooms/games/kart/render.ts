/**
 * Kart Grand Prix renderer: builds the track world (terrain, every route's road,
 * curbs, walls and rails, banking, bridges with pillars and abutments,
 * embankments where the land falls away, tunnels, water or lava, ramps with
 * side faces, launch pads, half-pipes, drawbridges, dash panels, item boxes,
 * coins, obstacles, fork signage, themed Kenney props) and animates karts,
 * drivers, gliders, items, coins, drawbridge leaves and effects from
 * simulation/snapshot state. Under the water surface the camera fogs over blue.
 */
import * as THREE from 'three';
import { AssetLibrary, PropBatch } from '../common/assets';
import { GameView, owned, type Theme } from '../common/view';
import { Particles } from '../common/particles';
import { approach, clamp, fbm, hashString, lerp, makeNoise, Rng, smoothstep, wrapAngle } from '../common/math';
import { asphaltTexture, checkerTexture, chevronTexture, curbTexture, groundDetailTexture, labelTexture, stripeTexture } from '../common/textures';
import {
  CHASM, circ, coinSpots, CRUSHER_HALF, DRAW_ANGLE, drawLift, F_BRIDGE, F_DRAW, FORK_ZONE, F_GAP, F_GLASS, F_PIPE, F_RAILS, F_TUNNEL, LAUNCH_HEIGHT, LAUNCH_LENGTH, nearest, PIPE_ENTRY, PIPE_HEIGHT,
  pipeOn, RAIL_SHOULDER, newNear, obstacleAt, pointAt, rampOn, RAMP_HEIGHT, RAMP_LENGTH, ROLLER_RADIUS, sampleAt, TUNNEL_HEIGHT, type Drawbridge, type Near, type Path, type ThemeId, type Track,
} from './tracks';
import { racerDef, RACERS, type RacerId } from './roster';
import type { KartEvent, KartState } from './sim';

interface KartTheme extends Theme {
  ground: number[]; far: number; water: number | null; road: string; edge: number;
  /** Colour of the track's liquid (lagoon water, lava) and of the land just below it. */
  liquid: number; bed: number;
  wall: [string, string]; obstacle: number;
}
const THEMES: Record<ThemeId, KartTheme> = {
  meadow: { skyTop: 0x3f86d8, skyHorizon: 0xcde9ff, fog: 0xd6ecff, fogNear: 180, fogFar: 720, sun: 0xfff3dc, sunIntensity: 2.6, ambientSky: 0xcfe6ff, ambientGround: 0x5d7a3a, ambient: 1.1, ground: [0x6cbf4a, 0x58a93c, 0x8fcd5c, 0x7bb84f], far: -10, water: 0x3d9bd6, road: '#5a5e66', edge: 0x9bd36a, liquid: 0x3d9bd6, bed: 0xd8c48a, wall: ['#d8322f', '#f4f4f4'], obstacle: 0x8a7a66 },
  city: { skyTop: 0x4d8fd6, skyHorizon: 0xd7e7f5, fog: 0xd9e4ef, fogNear: 160, fogFar: 640, sun: 0xfff1d6, sunIntensity: 2.5, ambientSky: 0xdde8f5, ambientGround: 0x666a70, ambient: 1.15, ground: [0x9a9ea6, 0x8e939b, 0xa4a8ae, 0x7fae5a], far: 0, water: null, road: '#4d5158', edge: 0xb9bdc4, liquid: 0x3d8fc0, bed: 0x6f7378, wall: ['#d8322f', '#f4f4f4'], obstacle: 0xd8322f },
  canyon: { skyTop: 0x3a78c4, skyHorizon: 0xf5d9b0, fog: 0xf0d2a8, fogNear: 170, fogFar: 700, sun: 0xffe2b8, sunIntensity: 2.8, ambientSky: 0xffe7c9, ambientGround: 0x9a5a35, ambient: 1.05, ground: [0xd9955a, 0xc8763f, 0xe0ac70, 0xb86534], far: 34, water: null, road: '#6a5e58', edge: 0xe7b47a, liquid: 0x3d8fc0, bed: 0xb86534, wall: ['#b5562c', '#e8c79a'], obstacle: 0x9a5a35 },
  frost: { skyTop: 0x6d9fd6, skyHorizon: 0xe8f2fc, fog: 0xe6eff9, fogNear: 150, fogFar: 640, sun: 0xffffff, sunIntensity: 2.2, ambientSky: 0xeaf3ff, ambientGround: 0x9fb3c8, ambient: 1.25, ground: [0xf3f7fc, 0xe2ebf5, 0xffffff, 0xd2deeb], far: 30, water: null, road: '#5d6670', edge: 0xffffff, liquid: 0x7fb6d9, bed: 0xd2deeb, wall: ['#2f6fd8', '#f4f4f4'], obstacle: 0xf4f8ff },
  lagoon: { skyTop: 0x2f8fe0, skyHorizon: 0xc9f1ff, fog: 0xc9eefc, fogNear: 200, fogFar: 820, sun: 0xfff1d0, sunIntensity: 2.8, ambientSky: 0xd2f3ff, ambientGround: 0x5f8f5a, ambient: 1.15, ground: [0x5fbf4f, 0x4aa844, 0x86d06a, 0xe9d7a0], far: -14, water: null, road: '#6b6a68', edge: 0xf1e3b0, liquid: 0x1fa8c9, bed: 0xe8d49a, wall: ['#2aa7a1', '#f6efe0'], obstacle: 0x8b6a4a },
  volcano: { skyTop: 0x3b2c3f, skyHorizon: 0xd98a5a, fog: 0x9a5a48, fogNear: 150, fogFar: 640, sun: 0xffc48a, sunIntensity: 2.4, ambientSky: 0xffb08a, ambientGround: 0x3a2622, ambient: 1.0, ground: [0x4a3c3a, 0x3a2f2e, 0x5c4a44, 0x2c2423], far: -16, water: null, road: '#3f3a3c', edge: 0x6b5a52, liquid: 0xff5a1a, bed: 0x8a2a10, wall: ['#e0582a', '#2a2222'], obstacle: 0x4a3a34 },
  dusk: { skyTop: 0x2b2f6b, skyHorizon: 0xf2a079, fog: 0xc98a86, fogNear: 160, fogFar: 660, sun: 0xffb27a, sunIntensity: 2.2, ambientSky: 0xc8a8e8, ambientGround: 0x4a4458, ambient: 1.2, ground: [0x8e8a96, 0x7f7b88, 0x9c98a4, 0x6f9e5a], far: 0, water: null, road: '#3e4049', edge: 0xb0acb8, liquid: 0x2c6f9e, bed: 0x4a5a6a, wall: ['#ff4fa3', '#2f2f3a'], obstacle: 0xffb000 },
};
const KART_SCALE = 2.0;
const SPARK = [0x9a9a9a, 0x4fc3ff, 0xff9a2e, 0xd36bff];
/** Underwater fog. */
const DEEP = { color: 0x0d5f78, near: 2, far: 120 };
const _near: Near = newNear();
const _tn: Near = newNear();
const _pt = [0, 0, 0, 0];
const _ob = [0, 0, 0, 0];
const _v = new THREE.Vector3();
const _look = new THREE.Vector3();
const _m = new THREE.Matrix4();
const _q = new THREE.Quaternion();
const _s = new THREE.Vector3();
const _p = new THREE.Vector3();
const _up = new THREE.Vector3(0, 1, 0);

interface KartVisual {
  racer: RacerId; root: THREE.Group; body: THREE.Group; wheels: THREE.Object3D[]; front: THREE.Object3D[];
  mixer: THREE.AnimationMixer | null; flames: THREE.Mesh[]; bubble: THREE.Mesh; label: THREE.Sprite | null; wing: THREE.Group;
  wheelSpin: number; tilt: number; bounce: number; visualYaw: number; spinAngle: number; pitch: number; wingOpen: number;
}
interface ObstacleVisual { mesh: THREE.Mesh; roll: number }
/** A drawbridge's two leaves, each a group pivoting about its hinge. */
interface DrawbridgeVisual { def: Drawbridge; leaves: THREE.Group[] }

export interface RenderRacer { id: number; racer: RacerId; human: boolean; name: string }

export class KartRenderer {
  readonly view: GameView;
  private lib: AssetLibrary;
  private track: Track;
  private theme: KartTheme;
  private karts: KartVisual[] = [];
  private boxes: THREE.Object3D[] = [];
  private bananas = new Map<number, THREE.Object3D>();
  private bombs = new Map<number, THREE.Object3D>();
  private obstacles: ObstacleVisual[] = [];
  private drawbridges: DrawbridgeVisual[] = [];
  private coins: THREE.InstancedMesh | null = null;
  private particles = new Particles(2400, true);
  private smoke = new Particles(700, false);
  private camPos = new THREE.Vector3();
  private camLook = new THREE.Vector3();
  private camReady = false;
  private underwater = false;
  private time = 0;
  private lava: THREE.Texture | null = null;
  private flameGeo = new THREE.ConeGeometry(0.22, 1.2, 10, 1, true);
  private flameMat = new THREE.MeshBasicMaterial({ color: 0xff8a2a, transparent: true, opacity: 0.85, blending: THREE.AdditiveBlending, depthWrite: false });
  private bubbleGeo = new THREE.SphereGeometry(2.0, 20, 14);
  private bubbleMat = new THREE.MeshBasicMaterial({ color: 0xffe066, transparent: true, opacity: 0.22, blending: THREE.AdditiveBlending, depthWrite: false });
  private wingGeo = new THREE.BoxGeometry(4.2, 0.08, 1.5);
  private strutGeo = new THREE.CylinderGeometry(0.05, 0.05, 1.3, 6);
  private strutMat = new THREE.MeshLambertMaterial({ color: 0x333333 });
  private racers: RenderRacer[];
  private self: number;

  constructor(host: HTMLElement, track: Track, racers: RenderRacer[], self: number, signal: AbortSignal) {
    this.track = track;
    this.theme = THEMES[track.def.theme];
    this.view = new GameView(host, this.theme);
    this.view.camera.far = 1500;
    this.lib = new AssetLibrary(signal);
    this.racers = racers;
    this.self = self;
  }

  async load(): Promise<void> {
    // Every seated driver is loaded up front: guests may switch racer during the intro.
    const drivers = RACERS.map(r => r.driver).filter((d): d is string => !!d);
    await Promise.all([this.lib.loadKit('kart-kit.glb'), drivers.length ? this.lib.loadCharacters(drivers) : Promise.resolve()]);
    this.buildWorld();
    for (const r of this.racers) this.karts.push(this.buildKart(r));
    this.view.scene.add(this.particles.points, this.smoke.points);
    for (const m of [this.flameGeo, this.flameMat, this.bubbleGeo, this.bubbleMat, this.wingGeo, this.strutGeo, this.strutMat]) this.lib.own(m);
  }

  // ── World ────────────────────────────────────────────────────────────────
  /**
   * Landscape height at (x, z) over every route: the lowest road bed when the
   * point is on a road (under an overpass the land meets the lower road),
   * otherwise each nearby road's own off-road ground blended by closeness, so
   * the land near a road is what the simulation drives on and slopes between
   * roads at different heights.
   */
  private terrainY(x: number, z: number): number {
    const { track } = this;
    let road = Infinity, sum = 0, weight = 0, far = 0, farD = Infinity;
    const levels = this.levels, dists = this.levelDist;
    for (const p of track.paths) {
      // Nearest sample per stretch within reach: stacked or neighbouring stretches of one
      // path (spirals, a road passing under itself, hairpins) are separate candidates.
      let bd = Infinity, bi = 0, nl = 0;
      const reach = (p.halfWidth + 40) ** 2;
      for (let i = 0; i < p.count; i += 5) {
        const d = (p.px[i] - x) ** 2 + (p.pz[i] - z) ** 2;
        if (d < bd) { bd = d; bi = i; }
        if (d > reach) continue;
        let slot = -1;
        for (let k = 0; k < nl; k++) {
          const gap = Math.abs(i - levels[k]);
          if (Math.abs(p.py[i] - p.py[levels[k]]) < 4 && Math.min(gap, p.closed ? p.count - gap : gap) < 40) { slot = k; break; }
        }
        if (slot < 0) { if (nl < levels.length) { levels[nl] = i; dists[nl] = d; nl++; } }
        else if (d < dists[slot]) { levels[slot] = i; dists[slot] = d; }
      }
      if (nl === 0) { levels[0] = bi; nl = 1; }
      for (let k = 0; k < nl; k++) {
        const n = nearest(p, x, z, levels[k], _tn, 8);
        const g = track.terrain(x, z, n);
        const off = Math.abs(n.lateral) - p.halfWidth;
        if (off <= 1 && !(p.flags[n.index] & (F_BRIDGE | F_GAP))) road = Math.min(road, g);
        if (n.distance < p.halfWidth + 90) { const w = 1 / Math.max(1, off) ** 3; sum += g * w; weight += w; }
        if (n.distance < farD) { farD = n.distance; far = g; }
      }
    }
    return road < Infinity ? road : weight > 0 ? sum / weight : far;
  }
  private levels = new Int32Array(6);
  private levelDist = new Float64Array(6);
  /** Distance in plan from (x, z) to the nearest centre line of any route, minus its half width. */
  private roadGap(x: number, z: number): number {
    let best = Infinity;
    for (const p of this.track.paths) for (let i = 0; i < p.count; i += 3) {
      const d = Math.sqrt((p.px[i] - x) ** 2 + (p.pz[i] - z) ** 2) - p.halfWidth;
      if (d < best) best = d;
    }
    return best;
  }

  private buildWorld(): void {
    const { track, theme } = this;
    const scene = this.view.scene;
    const noise = makeNoise(hashString(track.def.id) ^ 0x51ed);
    let minX = Infinity, maxX = -Infinity, minZ = Infinity, maxZ = -Infinity;
    for (const p of track.paths) for (let i = 0; i < p.count; i++) {
      minX = Math.min(minX, p.px[i]); maxX = Math.max(maxX, p.px[i]);
      minZ = Math.min(minZ, p.pz[i]); maxZ = Math.max(maxZ, p.pz[i]);
    }
    const margin = 260, step = 4;
    const x0 = minX - margin, z0 = minZ - margin;
    const nx = Math.ceil((maxX - minX + margin * 2) / step) + 1, nz = Math.ceil((maxZ - minZ + margin * 2) / step) + 1;
    const positions = new Float32Array(nx * nz * 3), colors = new Float32Array(nx * nz * 3), uvs = new Float32Array(nx * nz * 2);
    const palette = theme.ground.map(c => new THREE.Color(c));
    const edge = new THREE.Color(theme.edge), bed = new THREE.Color(theme.bed);
    const tmp = new THREE.Color();
    const liquid = track.liquid;
    for (let iz = 0; iz < nz; iz++) for (let ix = 0; ix < nx; ix++) {
      const k = iz * nx + ix, x = x0 + ix * step, z = z0 + iz * step;
      let y = this.terrainY(x, z);
      // Main-loop distance drives the far-field shaping (mountains, sea).
      let bd = Infinity, bi = 0;
      for (let i = 0; i < track.count; i += 6) { const d = (track.px[i] - x) ** 2 + (track.pz[i] - z) ** 2; if (d < bd) { bd = d; bi = i; } }
      const near = nearest(track, x, z, bi, _near, 8);
      const off = Math.min(Math.abs(near.lateral) - track.halfWidth, Math.sqrt(bd) - track.halfWidth);
      const far = smoothstep(60, 240, off);
      y += far * theme.far * (0.55 + 0.45 * fbm(noise, x * 0.006, z * 0.006, 3));
      positions[k * 3] = x; positions[k * 3 + 1] = y; positions[k * 3 + 2] = z;
      uvs[k * 2] = x / 6; uvs[k * 2 + 1] = z / 6;
      const n = fbm(noise, x * 0.03, z * 0.03, 3) * 0.5 + 0.5;
      tmp.copy(palette[0]).lerp(palette[1], n);
      tmp.lerp(palette[2], smoothstep(0.55, 0.9, fbm(noise, x * 0.011 + 40, z * 0.011, 2) * 0.5 + 0.5) * 0.6);
      const theme_ = track.def.theme;
      if (theme_ === 'city' || theme_ === 'dusk') tmp.lerp(palette[3], smoothstep(30, 60, off) * smoothstep(0.4, 0.7, n));
      if (theme_ === 'canyon') tmp.lerp(palette[3], smoothstep(4, 30, y - near.y) * 0.8);
      if (theme_ === 'volcano') tmp.lerp(palette[3], smoothstep(4, 24, y - near.y) * 0.7);
      if (off < 3.5 && off > -1) tmp.lerp(edge, 0.55 * (1 - smoothstep(0, 3.5, off)));
      // Beaches and lakebeds (or glowing lava shores) around the liquid line.
      if (liquid !== null) tmp.lerp(bed, smoothstep(liquid + 2.5, liquid - 0.5, y) * (track.lava ? 0.9 : 0.85));
      if (theme_ === 'lagoon' && liquid !== null) tmp.lerp(palette[3], smoothstep(liquid + 6, liquid + 2, y) * smoothstep(liquid - 1, liquid + 1, y) * 0.8);
      colors[k * 3] = tmp.r; colors[k * 3 + 1] = tmp.g; colors[k * 3 + 2] = tmp.b;
    }
    const index: number[] = [];
    for (let iz = 0; iz < nz - 1; iz++) for (let ix = 0; ix < nx - 1; ix++) {
      const a = iz * nx + ix, b = a + 1, c = a + nx, d = c + 1;
      index.push(a, c, b, b, c, d);
    }
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
    geo.setAttribute('color', new THREE.BufferAttribute(colors, 3));
    geo.setAttribute('uv', new THREE.BufferAttribute(uvs, 2));
    geo.setIndex(index);
    geo.computeVertexNormals();
    const detail = this.lib.own(groundDetailTexture());
    const terrain = owned(new THREE.Mesh(geo, new THREE.MeshLambertMaterial({ vertexColors: true, map: detail })));
    terrain.receiveShadow = true;
    scene.add(terrain);
    if (theme.water !== null && liquid === null) {
      const water = owned(new THREE.Mesh(new THREE.PlaneGeometry(4000, 4000), new THREE.MeshLambertMaterial({ color: theme.water, transparent: true, opacity: 0.92 })));
      water.rotation.x = -Math.PI / 2; water.position.y = -3.2;
      scene.add(water);
    }
    if (liquid !== null) {
      let mat: THREE.Material;
      if (track.lava) {
        this.lava = this.lib.own(groundDetailTexture());
        this.lava.repeat.set(160, 160);
        mat = new THREE.MeshBasicMaterial({ color: theme.liquid, map: this.lava });
      } else mat = new THREE.MeshLambertMaterial({ color: theme.liquid, transparent: true, opacity: 0.74, side: THREE.DoubleSide, depthWrite: false });
      const surface = owned(new THREE.Mesh(new THREE.PlaneGeometry(6000, 6000), mat));
      surface.rotation.x = -Math.PI / 2; surface.position.y = liquid;
      surface.renderOrder = 2;
      scene.add(surface);
    }
    for (const p of track.paths) this.buildRoad(p);
    this.buildCoins();
    this.buildStart();
    this.buildObstacles();
    this.buildProps();
    for (let i = 0; i < this.itemBoxCount(); i++) {
      const box = this.lib.instance('toy-item-box');
      box.scale.setScalar(4);
      scene.add(box);
      this.boxes.push(box);
    }
  }
  private itemBoxCount(): number {
    let n = 0;
    for (const p of this.track.paths) n += p.features.itemRows.length * (p.halfWidth < 6 ? 3 : 5);
    return n;
  }

  /** Strip mesh following a path between two lateral offsets (banked like the road). */
  private strip(path: Path, fromLat: number, toLat: number, lift: number, vScale: number, material: THREE.Material, filter?: (i: number) => boolean, withRamp = true): THREE.Mesh {
    const pos: number[] = [], uv: number[] = [], idx: number[] = [];
    let open = false;
    const n = path.count, last = path.closed ? n : n - 1;
    for (let i = 0; i <= last; i++) {
      const k = i % n;
      if (filter && !filter(k)) { open = false; continue; }
      const ramp = withRamp ? rampOn(path, path.dist[k]) : 0;
      const lx = path.tz[k], lz = -path.tx[k], b = path.bank[k];
      const y = path.py[k] + lift + ramp;
      pos.push(path.px[k] + lx * fromLat, y - fromLat * b, path.pz[k] + lz * fromLat, path.px[k] + lx * toLat, y - toLat * b, path.pz[k] + lz * toLat);
      const v = (i === n && path.closed ? path.length : path.dist[k]) / vScale;
      uv.push(0, v, 1, v);
      const m = pos.length / 3;
      if (open) idx.push(m - 4, m - 2, m - 3, m - 3, m - 2, m - 1);
      open = true;
    }
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
    geo.setAttribute('uv', new THREE.Float32BufferAttribute(uv, 2));
    geo.setIndex(idx);
    geo.computeVertexNormals();
    const mesh = owned(new THREE.Mesh(geo, material));
    mesh.receiveShadow = true;
    return mesh;
  }

  /** Barrier along a path at `lateral` (side = which way its thickness goes), only where `filter` allows. */
  private wall(path: Path, lateral: number, side: number, material: THREE.Material, filter?: (i: number) => boolean, top = 1.3, bottom = -0.4, thick = 0.6): THREE.Mesh {
    const pos: number[] = [], uv: number[] = [], idx: number[] = [];
    const n = path.count, last = path.closed ? n : n - 1;
    let open = false;
    for (let i = 0; i <= last; i++) {
      const k = i % n;
      if (filter && !filter(k)) { open = false; continue; }
      const lx = path.tz[k], lz = -path.tx[k];
      const bx = path.px[k] + lx * lateral, bz = path.pz[k] + lz * lateral, y = path.py[k] - lateral * path.bank[k];
      const ox = lx * side * thick, oz = lz * side * thick;
      // inner bottom, inner top, outer top, outer bottom
      pos.push(bx, y + bottom, bz, bx, y + top, bz, bx + ox, y + top, bz + oz, bx + ox, y + bottom, bz + oz);
      const v = (i === n && path.closed ? path.length : path.dist[k]) / 4;
      uv.push(0, v, 0.5, v, 0.75, v, 1, v);
      if (open) {
        const m = pos.length / 3, a = m - 8, b = m - 4;
        for (let f = 0; f < 3; f++) {
          const p = a + f, q = a + f + 1, r = b + f, s = b + f + 1;
          if (side > 0) idx.push(p, r, q, q, r, s); else idx.push(p, q, r, q, s, r);
        }
      }
      open = true;
    }
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
    geo.setAttribute('uv', new THREE.Float32BufferAttribute(uv, 2));
    geo.setIndex(idx);
    geo.computeVertexNormals();
    const mesh = owned(new THREE.Mesh(geo, material));
    mesh.castShadow = true; mesh.receiveShadow = true;
    return mesh;
  }

  /** Whether the point `lat` metres beside sample `i` of `path` lies on another route's road (forks and merges). */
  private onOtherRoad(path: Path, i: number, lat: number): boolean {
    const { track } = this;
    if (track.paths.length < 2) return false;
    const zone = FORK_ZONE * 2.5;
    pointAt(path, path.dist[i], lat, _pt);
    for (const q of track.paths) {
      if (q === path) continue;
      let hint: number;
      if (path.index === 0) {
        // Main loop: only near this branch's fork or merge.
        const s = path.dist[i];
        if (Math.abs(circ(s - q.from, track.length)) < zone) hint = sampleAt(q, s - q.from);
        else if (Math.abs(circ(s - q.to, track.length)) < zone) hint = sampleAt(q, q.length - (q.to - s));
        else continue;
      } else if (q.index === 0) {
        const s = path.dist[i];
        if (s < zone) hint = sampleAt(track, path.from + s);
        else if (s > path.length - zone) hint = sampleAt(track, path.to - (path.length - s));
        else continue;
      } else continue;
      const n = nearest(q, _pt[0], _pt[2], hint, _tn, 30, _pt[1]);
      if (Math.abs(n.lateral) < q.halfWidth + 0.6 && Math.abs(n.y - _pt[1]) < 3) return true;
    }
    return false;
  }

  private buildRoad(path: Path): void {
    const { track, theme, view } = this;
    const scene = view.scene;
    const hw = path.halfWidth;
    const flag = (i: number, f: number) => (path.flags[i] & f) !== 0;
    // Plain tarmac: not over a gap, and not where a drawbridge leaf or a half-pipe carries the surface.
    const paved = (i: number) => !flag(i, F_GAP | F_DRAW | F_PIPE);
    // Edge furniture stops where the edge runs over the other road at a fork or merge.
    const clearL = new Uint8Array(path.count), clearR = new Uint8Array(path.count);
    for (let i = 0; i < path.count; i++) { clearL[i] = this.onOtherRoad(path, i, hw + 0.8) ? 0 : 1; clearR[i] = this.onOtherRoad(path, i, -hw - 0.8) ? 0 : 1; }
    const clear = (side: number) => (i: number) => (side > 0 ? clearL[i] : clearR[i]) === 1;
    const frac = (i: number) => path.dist[i] / path.length;
    // Branches sit a hair above the main road where they overlap at the fork and merge.
    const lift = path.index === 0 ? 0.06 : 0.075;
    const asphalt = this.lib.own(asphaltTexture(theme.road));
    const asphaltMat = new THREE.MeshLambertMaterial({ map: asphalt });
    scene.add(this.strip(path, -hw, hw, lift, 14, asphaltMat, paved));
    const ice = path.features.ice;
    if (ice.length) {
      const iceMat = new THREE.MeshLambertMaterial({ color: 0xbfe6ff, transparent: true, opacity: 0.75 });
      scene.add(this.strip(path, -hw, hw, lift + 0.03, 10, iceMat, i => paved(i) && ice.some(([a, b]) => frac(i) >= a && frac(i) <= b)));
    }
    // Off-road patches: sand, gravel or cinders laid over the tarmac.
    for (const o of path.features.offroad) {
      const [l0, l1] = o.lanes ?? [-1, 1];
      const mat = new THREE.MeshLambertMaterial({ color: theme.bed, map: this.lib.own(groundDetailTexture()) });
      scene.add(this.strip(path, l0 * hw, l1 * hw, lift + 0.04, 6, mat, i => frac(i) >= o.from && frac(i) <= o.to && paved(i)));
    }
    // Edges: curbs on open grass road, barriers on walled roads, rails on bridges and in tunnels.
    const [wa, wb] = theme.wall;
    const wallMat = new THREE.MeshLambertMaterial({ map: this.lib.own(curbTexture(wa, wb)) });
    if (path.edge === 'grass') {
      const curb = new THREE.MeshLambertMaterial({ map: this.lib.own(curbTexture()) });
      const curbed = (i: number) => paved(i) && !flag(i, F_BRIDGE) && !flag(i, F_TUNNEL);
      scene.add(this.strip(path, hw, hw + 1.1, lift + 0.02, 3, curb, i => curbed(i) && clearL[i] === 1), this.strip(path, -hw - 1.1, -hw, lift + 0.02, 3, curb, i => curbed(i) && clearR[i] === 1));
      const railed = (i: number) => flag(i, F_RAILS) && !flag(i, F_TUNNEL) && paved(i);
      let rails = false;
      for (let i = 0; i < path.count && !rails; i++) rails = railed(i);
      if (rails) for (const side of [-1, 1]) scene.add(this.wall(path, side * (hw + 1.4), side, wallMat, i => railed(i) && clear(side)(i), 1.0));
    } else {
      // Continuous barrier walls matching the collision limit (none over gaps, open at forks).
      const h = track.def.theme === 'city' || track.def.theme === 'dusk' ? 1.0 : 1.3;
      for (const side of [-1, 1]) scene.add(this.wall(path, side * (hw + 1.4), side, wallMat, i => paved(i) && !flag(i, F_TUNNEL) && clear(side)(i), h));
    }
    this.buildBridges(path);
    this.buildEmbankments(path);
    this.buildTunnels(path);
    this.buildHalfpipes(path, asphaltMat, wallMat);
    this.buildDrawbridges(path, asphaltMat, wallMat);
    // Dash panels.
    const chevron = new THREE.MeshBasicMaterial({ map: this.lib.own(chevronTexture()), toneMapped: false });
    for (const pad of path.features.boostPads) {
      const mesh = owned(new THREE.Mesh(new THREE.PlaneGeometry(4.4, 4.4), chevron));
      pointAt(path, pad.at * path.length, pad.lane * hw, _pt);
      const b = path.bank[sampleAt(path, pad.at * path.length)];
      mesh.position.set(_pt[0], _pt[1] + lift + 0.06 - pad.lane * hw * b, _pt[2]);
      mesh.rotation.set(-Math.PI / 2, _pt[3] + Math.PI, 0, 'YXZ');
      scene.add(mesh);
    }
    // Kicker ramps: striped deck over the rising road + lip face.
    const stripes = new THREE.MeshLambertMaterial({ map: this.lib.own(stripeTexture()) });
    for (const at of path.features.ramps) {
      const s0 = at * path.length - RAMP_LENGTH;
      scene.add(this.strip(path, -hw, hw, lift + 0.04, 2, stripes, i => { const d = path.dist[i] - s0; return d >= -2 && d <= RAMP_LENGTH + 2; }));
      this.lip(path, at, RAMP_HEIGHT, 0x333333);
      this.rampSides(path, s0, at * path.length, 0x333333);
    }
    // Launch ramps (and glider pads): long striped ramp, chevrons, a gantry with a banner.
    const glideStripes = new THREE.MeshLambertMaterial({ map: this.lib.own(stripeTexture('#38b6ff', '#ffffff')) });
    for (const l of path.features.launches) {
      const s0 = l.at * path.length - LAUNCH_LENGTH;
      scene.add(this.strip(path, -hw, hw, lift + 0.04, 3, l.glide ? glideStripes : stripes, i => { const d = path.dist[i] - s0; return d >= -2 && d <= LAUNCH_LENGTH + 1; }));
      this.lip(path, l.at, LAUNCH_HEIGHT, l.glide ? 0x1f6fb2 : 0x333333);
      this.rampSides(path, s0, l.at * path.length, l.glide ? 0x1f6fb2 : 0x333333);
      this.gantry(path, s0 - 6, l.glide ? 'GLIDER' : 'JUMP', l.glide ? '#38b6ff' : '#ffcc00');
    }
    for (const d of path.features.drawbridges) this.gantry(path, d.from * path.length - 40, 'DRAWBRIDGE', '#ff9a2e');
    for (const [a] of path.features.halfpipes) this.gantry(path, a * path.length - 12, 'HALF-PIPE', '#9ff6ff');
    // A sign in the gore where each branch splits from the main road, facing the oncoming racers.
    if (path.index > 0) {
      this.forkGantry(path);
      const tex = this.lib.own(labelTexture(routeLabel(path), path.shortcut ? '#ffcc00' : '#ffffff'));
      let at = -1;
      for (let s = 24; s < Math.min(120, path.length / 2) && at < 0; s += 4) {
        pointAt(path, s, 0, _pt);
        const n = nearest(track, _pt[0], _pt[2], sampleAt(track, path.from + s), _tn, 30);
        if (n.distance > path.halfWidth + track.halfWidth + 5) at = s;
      }
      if (at > 0) {
        pointAt(path, at, 0, _pt);
        const bx = _pt[0], bz = _pt[2], yaw = _pt[3];
        const n = nearest(track, bx, bz, sampleAt(track, path.from + at), _tn, 30);
        const mx = bx - n.lateral * n.tz, mz = bz + n.lateral * n.tx; // closest main-road point
        const x = (bx + mx) / 2, z = (bz + mz) / 2, y = this.terrainY(x, z);
        const sign = owned(new THREE.Mesh(new THREE.PlaneGeometry(9, 2.2), new THREE.MeshBasicMaterial({ map: tex, transparent: true, side: THREE.DoubleSide })));
        sign.position.set(x, y + 3.6, z);
        sign.rotation.y = yaw + Math.PI;
        const post = owned(new THREE.Mesh(new THREE.BoxGeometry(0.4, 3.6, 0.4), new THREE.MeshLambertMaterial({ color: 0x333333 })));
        post.position.set(x, y + 1.8, z);
        scene.add(sign, post);
      }
    }
  }

  /** Dark lip face at a ramp's top edge. */
  private lip(path: Path, at: number, height: number, color: number): void {
    const hw = path.halfWidth;
    pointAt(path, at * path.length, 0, _pt);
    const face = owned(new THREE.Mesh(new THREE.BoxGeometry(hw * 2, height, 0.5), new THREE.MeshLambertMaterial({ color })));
    face.position.set(_pt[0], _pt[1] + height / 2 + 0.05, _pt[2]);
    face.rotation.y = _pt[3];
    this.view.scene.add(face);
  }

  /** Solid side faces under a ramp, from below the road up to the ramp deck, between `s0` and the lip `s1`. */
  private rampSides(path: Path, s0: number, s1: number, color: number): void {
    const hw = path.halfWidth;
    const mat = new THREE.MeshLambertMaterial({ color, side: THREE.DoubleSide });
    const inRamp = (i: number) => path.dist[i] >= s0 - 2 && path.dist[i] <= s1 + 0.5;
    for (const side of [-1, 1]) {
      this.view.scene.add(this.skirt(path, side * (hw + 0.02), inRamp, i => 0.08 + rampOn(path, Math.min(path.dist[i], s1 - 1e-6)), () => -0.6, mat));
    }
  }

  /**
   * Vertical ribbon along `path` at `lateral`, from `bottom(i)` to `top(i)`
   * (heights relative to the centre line, banking applied), where `filter` allows.
   */
  private skirt(path: Path, lateral: number, filter: (i: number) => boolean, top: (i: number) => number, bottom: (i: number) => number, material: THREE.Material): THREE.Mesh {
    const pos: number[] = [], idx: number[] = [];
    const n = path.count, last = path.closed ? n : n - 1;
    let open = false;
    for (let i = 0; i <= last; i++) {
      const k = i % n;
      if (!filter(k)) { open = false; continue; }
      const lx = path.tz[k], lz = -path.tx[k];
      const x = path.px[k] + lx * lateral, z = path.pz[k] + lz * lateral, y = path.py[k] - lateral * path.bank[k];
      pos.push(x, y + bottom(k), z, x, y + top(k), z);
      const m = pos.length / 3;
      if (open) idx.push(m - 4, m - 3, m - 2, m - 3, m - 1, m - 2);
      open = true;
    }
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
    geo.setIndex(idx);
    geo.computeVertexNormals();
    const mesh = owned(new THREE.Mesh(geo, material));
    mesh.castShadow = true; mesh.receiveShadow = true;
    return mesh;
  }


  /**
   * Where the land beside the road falls away (the approaches to bridges and
   * jumps), solid embankment faces run from the road edge down to the ground;
   * gap edges and bridge ends get an abutment face across the road, so no
   * road ever ends in a floating sliver.
   */
  private buildEmbankments(path: Path): void {
    const hw = path.halfWidth;
    const { track } = this;
    const flag = (i: number, f: number) => (path.flags[i] & f) !== 0;
    const rock = track.def.theme === 'canyon' || track.def.theme === 'volcano';
    const mat = new THREE.MeshLambertMaterial({ color: rock ? this.theme.obstacle : 0x8f8f96, side: THREE.DoubleSide });
    // Embankments: drop > 0 on solid road (not a bridge deck, a leaf or a gap).
    const solid = (i: number) => path.drop[i] > 0.05 && !flag(i, F_BRIDGE | F_GAP | F_TUNNEL);
    let any = false;
    for (let i = 0; i < path.count && !any; i++) any = solid(i);
    if (any) {
      for (const side of [-1, 1]) {
        const shoulder = (i: number) => (flag(i, F_RAILS) ? RAIL_SHOULDER : 0.6);
        const depth = new Float32Array(path.count);
        for (let i = 0; i < path.count; i++) {
          if (!solid(i)) continue;
          const lat = side * (hw + shoulder(i) + 2.5);
          pointAt(path, path.dist[i], lat, _pt);
          const land = this.terrainY(_pt[0], _pt[2]) - (path.py[i] - lat * path.bank[i]);
          depth[i] = Math.min(-0.5, land - 0.8);
        }
        this.view.scene.add(this.skirt(path, side * (hw + 0.6), solid, i => 0.04 + (flag(i, F_PIPE) ? 0 : rampOn(path, path.dist[i])), i => depth[i], mat));
      }
    }
    // Abutments: a face across the road wherever a deck or a gap begins or ends (stopping short of any
    // road below), all merged into one mesh per route.
    const n = path.count, last = path.closed ? n : n - 1;
    const deck = (i: number) => flag(i, F_BRIDGE | F_GAP);
    const pos: number[] = [], idx: number[] = [];
    for (let i = 1; i < last; i++) {
      const a = (i - 1 + n) % n, b = i % n;
      if (deck(a) === deck(b)) continue;
      const k = deck(a) ? b : a;
      const roadY = path.py[k], top = roadY + 0.05 + rampOn(path, path.dist[k]);
      const floor = this.roadBelow(path.px[k], path.pz[k], roadY) ? roadY - 3 : roadY - CHASM;
      const w = hw + (flag(k, F_RAILS) ? RAIL_SHOULDER : 0.6), lx = path.tz[k] * w, lz = -path.tx[k] * w;
      const m = pos.length / 3;
      pos.push(path.px[k] - lx, floor, path.pz[k] - lz, path.px[k] + lx, floor, path.pz[k] + lz, path.px[k] - lx, top, path.pz[k] - lz, path.px[k] + lx, top, path.pz[k] + lz);
      idx.push(m, m + 1, m + 2, m + 1, m + 3, m + 2);
    }
    if (!idx.length) return;
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
    geo.setIndex(idx);
    geo.computeVertexNormals();
    const faces = owned(new THREE.Mesh(geo, mat));
    faces.castShadow = true;
    this.view.scene.add(faces);
  }

  /** Overhead gantry across the road at local distance `s` carrying a banner. */
  private gantry(path: Path, s: number, text: string, color: string): void {
    // Never inside a tunnel (the banner would cut through its roof): back up to open road first,
    // or, in a long tunnel, hang the banner from the roof instead.
    const roofed = (at: number) => [-3, 0, 3].some(d => (path.flags[sampleAt(path, path.closed ? circ(at + d, path.length) : at + d)] & F_TUNNEL) !== 0);
    for (let back = 0; back <= 120 && roofed(s); back += 4) s -= 4;
    if (path.closed) s = circ(s, path.length);
    if (!path.closed && (s < 4 || s > path.length - 4)) return;
    const scene = this.view.scene, hw = path.halfWidth, inside = roofed(s);
    const label = this.lib.own(labelTexture(text, color));
    pointAt(path, s, 0, _pt);
    const banner = owned(new THREE.Mesh(new THREE.PlaneGeometry(hw * 1.4, hw * 0.35), new THREE.MeshBasicMaterial({ map: label, transparent: true, side: THREE.DoubleSide })));
    banner.position.set(_pt[0], _pt[1] + (inside ? TUNNEL_HEIGHT - 0.6 - hw * 0.175 : 6.5), _pt[2]);
    banner.rotation.y = _pt[3] + Math.PI;
    scene.add(banner);
    if (inside) return;
    for (const side of [-1, 1]) {
      pointAt(path, s, side * (hw + 1.2), _pt);
      const post = owned(new THREE.Mesh(new THREE.BoxGeometry(0.5, 7.2, 0.5), new THREE.MeshLambertMaterial({ color: 0x333333 })));
      post.position.set(_pt[0], _pt[1] + 3.6, _pt[2]);
      scene.add(post);
    }
  }

  /** On the main road ~70 m before a fork: an arrow sign naming the branch, on the side it leaves. */
  private forkGantry(branch: Path): void {
    const { track } = this;
    pointAt(branch, Math.min(FORK_ZONE, branch.length / 3), 0, _pt);
    const n = nearest(track, _pt[0], _pt[2], sampleAt(track, branch.from + FORK_ZONE), _tn, 30);
    const left = n.lateral > 0;
    const text = left ? `◀ ${routeLabel(branch)}` : `${routeLabel(branch)} ▶`;
    this.gantry(track, branch.from - 70, text, branch.boost ? '#ff9a2e' : branch.shortcut ? '#ffcc00' : '#ffffff');
  }

  /** Half-pipes: a U-shaped deck (walls rising to PIPE_HEIGHT, flat coping out to the rail) with skirts, an end lip face and rails on top. */
  private buildHalfpipes(path: Path, deckMat: THREE.Material, railMat: THREE.Material): void {
    const hw = path.halfWidth, W = hw + RAIL_SHOULDER, M = 18;
    const scene = this.view.scene;
    const skirtMat = new THREE.MeshLambertMaterial({ color: 0xb8b4ae, side: THREE.DoubleSide });
    for (const [f, t] of path.features.halfpipes) {
      const a = f * path.length, b = t * path.length;
      const pos: number[] = [], uv: number[] = [], idx: number[] = [];
      const lats: number[] = [];
      for (let k = 0; k <= M; k++) lats.push(-W + (2 * W * k) / M);
      let rows = 0, lastI = -1;
      for (let i = 0; i < path.count; i++) {
        const s = path.dist[i];
        if (s < a || s > b) continue;
        const lx = path.tz[i], lz = -path.tx[i];
        for (const lat of lats) {
          const h = pipeOn(path, s, Math.max(-hw, Math.min(hw, lat)));
          pos.push(path.px[i] + lx * lat, path.py[i] - lat * path.bank[i] + h + 0.07, path.pz[i] + lz * lat);
          uv.push((lat + W) / (2 * W), s / 14);
        }
        if (rows > 0) {
          const r1 = rows * (M + 1), r0 = r1 - (M + 1);
          for (let k = 0; k < M; k++) idx.push(r0 + k, r1 + k, r0 + k + 1, r0 + k + 1, r1 + k, r1 + k + 1);
        }
        rows++; lastI = i;
      }
      if (rows < 2) continue;
      const geo = new THREE.BufferGeometry();
      geo.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
      geo.setAttribute('uv', new THREE.Float32BufferAttribute(uv, 2));
      geo.setIndex(idx);
      geo.computeVertexNormals();
      const pipe = owned(new THREE.Mesh(geo, deckMat));
      pipe.receiveShadow = true;
      scene.add(pipe);
      const inPipe = (i: number) => path.dist[i] >= a && path.dist[i] <= b;
      const wallTop = (i: number) => pipeOn(path, path.dist[i], hw) + 0.07;
      for (const side of [-1, 1]) {
        // Outer skirt down to the ground, and the rail along the coping once the walls are up.
        scene.add(this.skirt(path, side * W, inPipe, wallTop, () => -1.2, skirtMat));
        scene.add(this.wall(path, side * (W - 0.2), side, railMat, i => inPipe(i) && path.dist[i] >= a + PIPE_ENTRY, PIPE_HEIGHT + 1.1, PIPE_HEIGHT, 0.4));
      }
      // The lip: each wall ends in a vertical face at the end of the range.
      const lx = path.tz[lastI], lz = -path.tx[lastI];
      const lp: number[] = [], li: number[] = [];
      for (const lat of lats) {
        const h = pipeOn(path, path.dist[lastI], Math.max(-hw, Math.min(hw, lat)));
        const x = path.px[lastI] + lx * lat, z = path.pz[lastI] + lz * lat, y = path.py[lastI] - lat * path.bank[lastI];
        lp.push(x, y + 0.05, z, x, y + h + 0.07, z);
      }
      for (let k = 0; k < M; k++) { const p0 = k * 2; li.push(p0, p0 + 2, p0 + 1, p0 + 1, p0 + 2, p0 + 3); }
      const lipGeo = new THREE.BufferGeometry();
      lipGeo.setAttribute('position', new THREE.Float32BufferAttribute(lp, 3));
      lipGeo.setIndex(li);
      lipGeo.computeVertexNormals();
      scene.add(owned(new THREE.Mesh(lipGeo, skirtMat)));
    }
  }

  /**
   * Drawbridges: the two leaves (deck, rails and a dark underside) pivot about
   * hinges at the ends of the span, posed every frame from the race clock; a
   * pair of bascule towers stands at each hinge.
   */
  private buildDrawbridges(path: Path, deckMat: THREE.Material, railMat: THREE.Material): void {
    const scene = this.view.scene, hw = path.halfWidth, W = hw + RAIL_SHOULDER;
    const under = new THREE.MeshLambertMaterial({ color: 0x4a4a52 });
    const tower = new THREE.MeshLambertMaterial({ color: this.track.def.theme === 'dusk' ? 0x3e4049 : 0x8a6a4a });
    for (const d of path.features.drawbridges) {
      const a = d.from * path.length, b = d.to * path.length, half = (b - a) / 2;
      const leaves: THREE.Group[] = [];
      for (const [s, flip] of [[a, 0], [b, Math.PI]] as [number, number][]) {
        pointAt(path, s, 0, _pt);
        const group = new THREE.Group();
        group.position.set(_pt[0], _pt[1] + 0.09, _pt[2]);
        group.rotation.order = 'YXZ';
        group.rotation.y = _pt[3] + flip;
        // Each leaf reaches TAIL metres back past its hinge, under the road (the paved strip stops at the
        // last whole sample before the span); raised, that tail tips down out of sight.
        const TAIL = 2.2;
        const deck = new THREE.Mesh(new THREE.BoxGeometry(hw * 2, 0.06, half + TAIL), deckMat);
        deck.position.set(0, -0.03, (half - TAIL) / 2);
        const body = new THREE.Mesh(new THREE.BoxGeometry(W * 2, 0.9, half + TAIL), under);
        body.position.set(0, -0.52, (half - TAIL) / 2);
        deck.receiveShadow = true; body.castShadow = true;
        group.add(deck, body);
        for (const side of [-1, 1]) {
          const rail = new THREE.Mesh(new THREE.BoxGeometry(0.4, 1.0, half), railMat);
          rail.position.set(side * (W - 0.2), 0.5, half / 2);
          rail.castShadow = true;
          group.add(rail);
        }
        scene.add(owned(group));
        leaves.push(group);
        // Bascule towers either side of the hinge.
        for (const side of [-1, 1]) {
          pointAt(path, s + (flip ? 2 : -2), side * (W + 1.6), _pt);
          const t = owned(new THREE.Mesh(new THREE.BoxGeometry(2.4, 9, 2.4), tower));
          t.position.set(_pt[0], _pt[1] + 3, _pt[2]);
          t.castShadow = true;
          scene.add(t);
        }
      }
      this.drawbridges.push({ def: d, leaves });
    }
  }

  /** Every coin of the track as one instanced mesh (gold discs standing on edge), posed each frame. */
  private buildCoins(): void {
    const count = coinSpots(this.track).length / 3;
    if (!count) return;
    const geo = new THREE.CylinderGeometry(0.75, 0.75, 0.16, 18);
    geo.rotateX(Math.PI / 2);
    this.coins = owned(new THREE.InstancedMesh(geo, new THREE.MeshLambertMaterial({ color: 0xffc928, emissive: 0x6a4800 }), count));
    this.coins.castShadow = true;
    this.coins.frustumCulled = false;
    this.view.scene.add(this.coins);
  }

  /** Deck slab under bridge stretches and pillars down to the land (never onto a road below). */
  private buildBridges(path: Path): void {
    const hw = path.halfWidth;
    const bridge = (i: number) => (path.flags[i] & F_BRIDGE) !== 0 && !(path.flags[i] & (F_GAP | F_DRAW));
    let any = false;
    for (let i = 0; i < path.count && !any; i++) any = bridge(i);
    if (!any) return;
    const scene = this.view.scene;
    const concrete = new THREE.MeshLambertMaterial({ color: this.track.def.theme === 'lagoon' ? 0xb08a5a : 0x9a9aa2, side: THREE.DoubleSide });
    scene.add(this.strip(path, -hw - RAIL_SHOULDER, hw + RAIL_SHOULDER, -1.2, 6, concrete, bridge, false));
    // Deck shoulders out to the rails (the road strip covers the lanes).
    const railedDeck = (i: number) => bridge(i) && (path.flags[i] & F_RAILS) !== 0;
    scene.add(this.strip(path, hw - 0.05, hw + RAIL_SHOULDER, 0.03, 6, concrete, railedDeck, false), this.strip(path, -hw - RAIL_SHOULDER, -hw + 0.05, 0.03, 6, concrete, railedDeck, false));
    for (const side of [-1, 1]) scene.add(this.wall(path, side * (hw + RAIL_SHOULDER), side, concrete, bridge, 0.05, -1.2, 0.01));
    const spots: number[] = [];
    for (let i = 0; i < path.count; i += 8) {
      if (!bridge(i)) continue;
      for (const side of [-1, 1]) {
        // Just outside the deck (and outside the rails of a stacked road below).
        const lat = side * (hw + RAIL_SHOULDER + 0.6);
        pointAt(path, path.dist[i], lat, _pt);
        const top = path.py[i] - lat * path.bank[i] - 1.2;
        const base = this.terrainY(_pt[0], _pt[2]);
        if (top - base < 2.5 || this.roadBelow(_pt[0], _pt[2], top)) continue;
        spots.push(_pt[0], base - 1, _pt[2], top - base + 1);
      }
    }
    if (!spots.length) return;
    const pillars = new THREE.InstancedMesh(new THREE.CylinderGeometry(0.7, 0.9, 1, 10), concrete, spots.length / 4);
    for (let k = 0; k < spots.length; k += 4) {
      _p.set(spots[k], spots[k + 1] + spots[k + 3] / 2, spots[k + 2]);
      _s.set(1, spots[k + 3], 1);
      pillars.setMatrixAt(k / 4, _m.compose(_p, _q.identity(), _s));
    }
    pillars.instanceMatrix.needsUpdate = true;
    pillars.computeBoundingSphere();
    pillars.castShadow = true; pillars.receiveShadow = true;
    scene.add(owned(pillars));
  }
  /** Whether a road surface (lanes and shoulder) lies under (x, z) more than 2 m below `y`. */
  private roadBelow(x: number, z: number, y: number): boolean {
    for (const p of this.track.paths) for (let i = 0; i < p.count; i += 2) {
      if (p.py[i] > y - 2) continue;
      const dx = p.px[i] - x, dz = p.pz[i] - z, r = p.halfWidth + 1.4;
      if (dx * dx + dz * dz < r * r) return true;
    }
    return false;
  }

  /** Arched tubes over tunnel stretches: rock with ceiling lights, or glass under the water. */
  private buildTunnels(path: Path): void {
    const scene = this.view.scene;
    for (const glass of [false, true]) {
      const inTube = (i: number) => (path.flags[i] & F_TUNNEL) !== 0 && ((path.flags[i] & F_GLASS) !== 0) === glass;
      const pos: number[] = [], idx: number[] = [];
      const M = 12, R = path.halfWidth + 1.6;
      let open = false;
      const lights: number[] = [];
      for (let i = 0; i < path.count; i++) {
        if (!inTube(i)) { open = false; continue; }
        const lx = path.tz[i], lz = -path.tx[i];
        for (let a = 0; a <= M; a++) {
          const t = (a / M) * Math.PI;
          const lat = Math.cos(t) * R;
          pos.push(path.px[i] + lx * lat, path.py[i] - lat * path.bank[i] + Math.sin(t) * TUNNEL_HEIGHT - 0.4, path.pz[i] + lz * lat);
        }
        if (open) {
          const b = pos.length / 3 - (M + 1), a0 = b - (M + 1);
          for (let a = 0; a < M; a++) idx.push(a0 + a, b + a, a0 + a + 1, a0 + a + 1, b + a, b + a + 1);
        }
        open = true;
        if (i % 5 === 0) lights.push(path.px[i], path.py[i] + TUNNEL_HEIGHT - 0.7, path.pz[i], Math.atan2(path.tx[i], path.tz[i]));
      }
      if (!idx.length) continue;
      const geo = new THREE.BufferGeometry();
      geo.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
      geo.setIndex(idx);
      geo.computeVertexNormals();
      const mat = glass
        ? new THREE.MeshLambertMaterial({ color: 0xa8ecff, transparent: true, opacity: 0.22, side: THREE.DoubleSide, depthWrite: false })
        : new THREE.MeshLambertMaterial({ color: this.theme.obstacle, map: this.lib.own(groundDetailTexture()), side: THREE.DoubleSide });
      const tube = owned(new THREE.Mesh(geo, mat));
      tube.castShadow = !glass; tube.receiveShadow = true;
      if (glass) tube.renderOrder = 3;
      scene.add(tube);
      const lamp = new THREE.InstancedMesh(new THREE.BoxGeometry(2.4, 0.15, 0.5), new THREE.MeshBasicMaterial({ color: glass ? 0x9ff6ff : 0xffe7a8, toneMapped: false }), lights.length / 4);
      for (let k = 0; k < lights.length; k += 4) {
        _p.set(lights[k], lights[k + 1], lights[k + 2]);
        _q.setFromAxisAngle(_up, lights[k + 3]);
        lamp.setMatrixAt(k / 4, _m.compose(_p, _q, _s.set(1, 1, 1)));
      }
      lamp.instanceMatrix.needsUpdate = true;
      lamp.computeBoundingSphere();
      scene.add(owned(lamp));
    }
  }

  private buildStart(): void {
    const { track, view } = this;
    // Start line + finish gate.
    const checker = new THREE.MeshBasicMaterial({ map: this.lib.own(checkerTexture()) });
    const start = owned(new THREE.Mesh(new THREE.PlaneGeometry(track.halfWidth * 2, 2.4), checker));
    pointAt(track, 0, 0, _pt);
    start.position.set(_pt[0], _pt[1] + 0.1, _pt[2]);
    start.rotation.set(-Math.PI / 2, _pt[3], 0, 'YXZ');
    view.scene.add(start);
  }

  private buildObstacles(): void {
    const { track, theme } = this;
    const rollerMat = new THREE.MeshLambertMaterial({ color: theme.obstacle, map: this.lib.own(track.def.theme === 'dusk' ? stripeTexture('#ffb000', '#2a2a2a') : groundDetailTexture()) });
    const crusherMat = new THREE.MeshLambertMaterial({ color: 0x6d6a72, map: this.lib.own(groundDetailTexture()) });
    const rollerGeo = this.lib.own(new THREE.SphereGeometry(ROLLER_RADIUS, 18, 12));
    const crusherGeo = this.lib.own(new THREE.BoxGeometry(CRUSHER_HALF * 2, 2.6, CRUSHER_HALF * 2));
    this.lib.own(rollerMat); this.lib.own(crusherMat);
    for (const o of track.obstacles) {
      const mesh = new THREE.Mesh(o.kind === 'roller' ? rollerGeo : crusherGeo, o.kind === 'roller' ? rollerMat : crusherMat);
      mesh.castShadow = true; mesh.receiveShadow = true;
      this.view.scene.add(mesh);
      this.obstacles.push({ mesh, roll: 0 });
    }
  }

  private buildProps(): void {
    const { track } = this;
    const batch = new PropBatch(this.lib);
    const rng = new Rng(hashString(track.def.id) ^ 0x2f1);
    const hw = track.halfWidth;
    const liquid = track.liquid;
    /** Place along a path at lateral distance (beyond the road edge), on dry land clear of every road. */
    const beside = (path: Path, key: string, s: number, side: number, dist: number, scale: number, faceRoad = true, jitterYaw = 0) => {
      pointAt(path, s, side * (path.halfWidth + dist), _pt);
      if (this.roadGap(_pt[0], _pt[2]) < dist * 0.6) return;
      const y = this.terrainY(_pt[0], _pt[2]);
      if (liquid !== null && y < liquid + 0.4) return;
      const yaw = faceRoad ? _pt[3] + (side > 0 ? -Math.PI / 2 : Math.PI / 2) : rng.range(0, Math.PI * 2);
      batch.add(key, _pt[0], y - 0.05, _pt[2], yaw + rng.range(-jitterYaw, jitterYaw), scale);
    };
    const scatter = (keys: string[], count: number, minD: number, maxD: number, scale: [number, number]) => {
      for (let n = 0; n < count; n++) {
        const path = track.paths.length > 1 && rng.chance(0.15) ? track.paths[1 + rng.int(track.paths.length - 1)] : track;
        const s = rng.range(0, path.length), side = rng.chance(0.5) ? 1 : -1, d = rng.range(minD, maxD);
        beside(path, rng.pick(keys), s, side, d, rng.range(scale[0], scale[1]), false);
      }
    };
    // Solid structures stay beyond the reachable area: outside the walls, or
    // past the rescue line on grass tracks (karts never drive through them).
    const gap = track.def.edge === 'grass' ? track.def.outOfBounds + 2 : 3;
    const t = track.def.theme;
    const urban = t === 'city' || t === 'dusk';
    // Start area: grandstands, banner towers, flags, tents.
    for (let s = -70; s <= 70; s += 12) {
      beside(track, urban ? 'race-grandStandCovered' : 'race-grandStand', s, 1, gap + 6, 11);
      if (Math.abs(s) > 20) beside(track, 'race-tentLong', s, -1, gap + 3, 6);
    }
    beside(track, 'race-pitsGarage', -24, -1, gap + 5, 10); beside(track, 'race-pitsOffice', 0, -1, gap + 5, 10); beside(track, 'race-pitsGarage', 24, -1, gap + 5, 10);
    for (let s = -90; s < 110; s += 30) { beside(track, 'race-flagCheckers', s, -1, gap, 5); beside(track, 'race-flagRed', s + 15, 1, gap, 5); }
    pointAt(track, 2, 0, _pt);
    batch.add('toy-gate-finish', _pt[0], _pt[1], _pt[2], _pt[3], (hw * 2 + 6) / 1.55, 9);
    // Billboards and flags on corners.
    for (const path of track.paths) for (let i = 0; i < path.count; i += 22) {
      if (Math.abs(path.curve[i]) < 0.02 || path.drop[i] > 0) continue;
      const side = path.curve[i] > 0 ? -1 : 1; // outside of the bend
      beside(path, rng.chance(0.5) ? 'race-billboard' : 'race-billboardLow', path.dist[i], side, gap + 4, 9);
      beside(path, rng.pick(['race-flagRed', 'race-flagGreen', 'race-flagCheckers']), path.dist[i] + 8, side, gap, 4);
    }
    if (t === 'meadow') {
      scatter(['nat-tree_default', 'nat-tree_oak', 'nat-tree_detailed', 'nat-tree_fat', 'race-treeLarge'], 520, gap, 150, [6, 9]);
      scatter(['nat-plant_bushLarge', 'nat-plant_bush', 'nat-flower_redA', 'nat-flower_yellowB', 'nat-flower_purpleA', 'nat-mushroom_redGroup'], 600, 3, 40, [5, 8]);
      scatter(['nat-tent_detailedOpen', 'nat-log_stack', 'nat-stump_old'], 50, gap, 50, [6, 8]);
      for (let i = 0; i < track.count; i += 7) if (Math.abs(track.curve[i]) > 0.018) beside(track, 'nat-fence_simple', track.dist[i], track.curve[i] > 0 ? -1 : 1, gap - 1, 5);
    } else if (urban) {
      const blocks = ['city-building-a', 'city-building-b', 'city-building-c', 'city-building-d', 'city-building-e', 'city-building-f', 'city-building-g', 'city-building-h', 'city-building-i', 'city-building-j', 'city-building-k', 'city-building-l'];
      for (const path of track.paths) for (let s = 0; s < path.length; s += 15) for (const side of [-1, 1]) {
        if (path.index === 0 && Math.abs(((s + 70) % path.length) - 70) < 80 && side > 0) continue; // grandstands
        const i = sampleAt(path, s);
        if (t === 'dusk' && (path.drop[i] > 0 || path.py[i] > 4)) continue; // not beside the freeway decks
        beside(path, rng.pick(blocks), s, side, rng.range(12, 16), rng.range(10, 12));
        if (rng.chance(0.25)) beside(path, 'race-lightPostLarge', s + 7, side, 2.4, 7);
      }
      scatter(['city-building-skyscraper-a', 'city-building-skyscraper-b', 'city-building-skyscraper-c', 'city-building-skyscraper-d'], 140, 40, 200, [12, 15]);
      scatter(['house-tree-large', 'house-tree-small', 'city-detail-parasol-a'], 160, 4, 10, [9, 12]);
      if (t === 'dusk') scatter(['race-lightPostLarge', 'city-detail-awning'], 90, 3, 9, [6, 8]);
    } else if (t === 'canyon') {
      for (let s = 0; s < track.length; s += 9) for (const side of [-1, 1]) {
        if (rng.chance(0.75)) beside(track, rng.pick(['nat-cliff_block_rock', 'nat-cliff_large_rock', 'nat-rock_tallB']), s, side, rng.range(5, 9), rng.range(9, 14), true, 0.4);
      }
      scatter(['nat-rock_tallB', 'nat-rock_tallE', 'nat-rock_tallG', 'nat-cliff_block_rock'], 260, 20, 180, [12, 26]);
      scatter(['nat-cactus_tall', 'nat-cactus_short', 'nat-rock_largeA', 'nat-rock_largeC'], 260, 6, 70, [5, 9]);
      scatter(['nat-tree_palmTall', 'nat-tree_palmBend'], 40, 10, 60, [6, 8]);
    } else if (t === 'frost') {
      scatter(['plat-tree-pine-snow', 'plat-tree-snow', 'plat-tree-pine-snow-small'], 620, gap, 170, [4, 7]);
      scatter(['plat-rocks', 'plat-block-snow-large', 'nat-rock_largeA'], 120, gap, 90, [4, 8]);
      scatter(['plat-stones'], 90, 4, gap - 2, [3, 5]);
    } else if (t === 'lagoon') {
      scatter(['nat-tree_palmTall', 'nat-tree_palmBend', 'nat-tree_palmTall'], 520, gap, 150, [6, 9]);
      scatter(['nat-plant_bushLarge', 'nat-plant_bush', 'nat-flower_redA', 'nat-flower_yellowB', 'nat-flower_purpleA'], 520, 3, 40, [5, 8]);
      scatter(['nat-rock_largeA', 'nat-rock_largeC', 'nat-cliff_block_rock'], 120, gap, 90, [6, 12]);
      scatter(['city-detail-parasol-a', 'nat-tent_detailedOpen', 'nat-log_stack'], 70, gap, 40, [6, 9]);
    } else {
      // Volcano: basalt spires along the road, boulder fields, dead stumps.
      for (let s = 0; s < track.length; s += 12) for (const side of [-1, 1]) {
        if (rng.chance(0.45)) beside(track, rng.pick(['nat-cliff_block_rock', 'nat-cliff_large_rock', 'nat-rock_tallE']), s, side, rng.range(gap, gap + 8), rng.range(8, 14), true, 0.5);
      }
      scatter(['nat-rock_tallB', 'nat-rock_tallE', 'nat-rock_tallG', 'nat-cliff_large_rock'], 300, 20, 180, [12, 28]);
      scatter(['nat-rock_largeA', 'nat-rock_largeC', 'plat-rocks', 'nat-stump_old', 'nat-log_stack'], 260, 5, 70, [5, 9]);
    }
    const group = new THREE.Group();
    batch.build(group, true);
    this.view.scene.add(group);
  }

  private buildKart(r: RenderRacer): KartVisual {
    const def = racerDef(r.racer);
    const root = new THREE.Group();
    const body = new THREE.Group();
    const kart = this.lib.instance(def.kart);
    body.add(kart);
    body.scale.setScalar(KART_SCALE);
    root.add(body);
    const wheels: THREE.Object3D[] = [], front: THREE.Object3D[] = [];
    let character: THREE.Object3D | null = null;
    kart.traverse(o => {
      if (/wheel-(front|back)-(left|right)$/.test(o.name)) { wheels.push(o); if (o.name.includes('front')) front.push(o); }
      if (o.name.endsWith('character')) character = o;
    });
    let mixer: THREE.AnimationMixer | null = null;
    if (def.driver) {
      if (character) (character as THREE.Object3D).visible = false;
      const driver = this.lib.character(def.driver);
      driver.position.set(0, 0.2, -0.12);
      body.add(driver);
      mixer = new THREE.AnimationMixer(driver);
      mixer.clipAction(this.lib.clip('drive')).play();
    }
    const flames = [-0.18, 0.18].map(x => {
      const f = new THREE.Mesh(this.flameGeo, this.flameMat);
      f.rotation.x = -Math.PI / 2;
      f.position.set(x * KART_SCALE, 0.55, -1.25);
      f.visible = false;
      root.add(f);
      return f;
    });
    const bubble = new THREE.Mesh(this.bubbleGeo, this.bubbleMat);
    bubble.position.y = 1.1; bubble.visible = false;
    root.add(bubble);
    // Glider: a canopy in the racer's colour on two struts, folded away until launch.
    const wing = new THREE.Group();
    const canopy = new THREE.Mesh(this.wingGeo, this.lib.own(new THREE.MeshLambertMaterial({ color: def.color })));
    canopy.position.y = 1.35; canopy.castShadow = true;
    wing.add(canopy);
    for (const x of [-0.7, 0.7]) { const strut = new THREE.Mesh(this.strutGeo, this.strutMat); strut.position.set(x, 0.7, 0); wing.add(strut); }
    wing.position.set(0, 1.2, -0.2);
    wing.visible = false;
    root.add(wing);
    let label: THREE.Sprite | null = null;
    if (r.human && r.id !== this.self) {
      const tex = this.lib.own(labelTexture(r.name, '#ffffff'));
      label = new THREE.Sprite(this.lib.own(new THREE.SpriteMaterial({ map: tex, depthTest: false, transparent: true })));
      label.scale.set(4, 1, 1); label.position.y = 3.6; label.renderOrder = 10;
      root.add(label);
    }
    this.view.scene.add(root);
    return { racer: r.racer, root, body, wheels, front, mixer, flames, bubble, label, wing, wheelSpin: 0, tilt: 0, bounce: 0, visualYaw: 0, spinAngle: 0, pitch: 0, wingOpen: 0 };
  }

  // ── Frame ────────────────────────────────────────────────────────────────
  effect(e: KartEvent): void {
    const p = this.particles;
    switch (e.type) {
      case 'box': p.burst(e.x, e.y + 1.2, e.z, 26, 0xc58cff, 9, 0.7, 0.6); break;
      case 'explode': p.burst(e.x, e.y + 0.8, e.z, 70, 0xffa040, 16, 1.4, 0.8); for (let i = 0; i < 18; i++) this.smoke.spawn(e.x, e.y + 1, e.z, (Math.random() - 0.5) * 6, 3 + Math.random() * 3, (Math.random() - 0.5) * 6, 0x555555, 3, 1.4, 0, 1.5); break;
      case 'spin': p.burst(e.x, e.y + 1.6, e.z, 14, 0xffff88, 4, 0.5, 0.8, 0); break;
      case 'turbo': p.burst(e.x, e.y + 0.6, e.z, 24, SPARK[e.value ?? 1], 8, 0.6, 0.45); break;
      case 'star': p.burst(e.x, e.y + 1, e.z, 40, 0xffe066, 10, 0.8, 0.8); break;
      case 'bolt': p.burst(e.x, e.y + 4, e.z, 50, 0x9ad7ff, 14, 0.9, 0.6, 0); break;
      case 'respawn': p.burst(e.x, e.y + 1, e.z, 30, 0xffffff, 6, 1, 0.8); break;
      case 'land': for (let i = 0; i < 10; i++) this.smoke.spawn(e.x, e.y + 0.3, e.z, (Math.random() - 0.5) * 5, 1, (Math.random() - 0.5) * 5, 0xb9a88f, 1.6, 0.7, 0, 2); break;
      case 'glide': p.burst(e.x, e.y + 2, e.z, 30, 0x9fe6ff, 7, 0.7, 0.6, 0); break;
      case 'crush': for (let i = 0; i < 16; i++) this.smoke.spawn(e.x, e.y + 0.4, e.z, (Math.random() - 0.5) * 9, 1.5, (Math.random() - 0.5) * 9, 0x8a8a8a, 2.2, 0.9, 0, 2); break;
      case 'splash': {
        // value 1: diving under on purpose; 0: fell in the water; 2: lava.
        const y = this.track.liquid ?? e.y, lava = e.value === 2;
        p.burst(e.x, y + 0.3, e.z, e.value === 1 ? 30 : 60, lava ? 0xff7a2a : 0xe8fbff, e.value === 1 ? 6 : 10, lava ? 0.9 : 1.1, 0.9, -12);
        if (lava) for (let i = 0; i < 14; i++) this.smoke.spawn(e.x, y + 0.5, e.z, (Math.random() - 0.5) * 3, 3 + Math.random() * 2, (Math.random() - 0.5) * 3, 0x3a3030, 3, 1.6, 0, 1);
        break;
      }
      default: break;
    }
  }

  frame(state: KartState, dt: number, lookBack: boolean): void {
    this.time += dt;
    const self = state.racers[this.self] ?? state.racers[0];
    state.racers.forEach((r, i) => {
      const visual = this.karts[i];
      if (!visual) return;
      if (visual.racer !== r.racer) {
        // A guest picked a different racer: swap the kart model in place.
        visual.mixer?.stopAllAction();
        visual.root.removeFromParent();
        this.racers[i] = { ...this.racers[i], racer: r.racer };
        this.karts[i] = this.buildKart(this.racers[i]);
      }
      this.updateKart(this.karts[i], r, dt, i === this.self);
    });
    // Item boxes spin and pop back.
    state.boxes.forEach((b, i) => {
      const box = this.boxes[i];
      if (!box) return;
      const shown = b.timer <= 0;
      const target = shown ? 4 : 0.01;
      box.scale.setScalar(approach(box.scale.x, target, shown ? 6 : 20, dt));
      box.visible = box.scale.x > 0.05;
      box.position.set(b.x, b.y - 0.9 + Math.sin(this.time * 2 + i) * 0.25, b.z);
      box.rotation.set(0.3, this.time * 1.5 + i, 0.2);
    });
    this.updateObstacles(state);
    this.updateCoins(state);
    for (const db of this.drawbridges) {
      const th = DRAW_ANGLE * drawLift(db.def, state.raceTime);
      for (const leaf of db.leaves) leaf.rotation.x = -th;
    }
    this.syncPool(this.bananas, state.hazards, 'toy-item-banana', 5, (o, h) => { o.position.set(h.x, h.y + 0.05, h.z); });
    this.syncPool(this.bombs, state.shells, 'plat-bomb', 2.6, (o, s) => {
      o.position.set(s.x, s.y - 0.4 + Math.abs(Math.sin(this.time * 9)) * 0.4, s.z); o.rotation.y = s.yaw;
      if (Math.random() < 0.6) this.particles.spawn(s.x, s.y + 0.6, s.z, 0, 1.5, 0, 0xffa040, 0.5, 0.35);
    });
    if (this.lava) { this.lava.offset.x = (this.time * 0.02) % 1; this.lava.offset.y = (this.time * 0.013) % 1; }
    this.particles.update(dt);
    this.smoke.update(dt);
    this.updateCamera(state, self, dt, lookBack);
    this.updateFog();
    this.view.focusShadow(self.x, self.y, self.z);
    this.view.render();
  }

  /** Rollers roll across the road, crushers rise and slam (same clock as the simulation). */
  private updateObstacles(state: KartState): void {
    const { track } = this;
    track.obstacles.forEach((o, i) => {
      const v = this.obstacles[i];
      if (!v) return;
      obstacleAt(track, o, state.raceTime, _ob);
      if (o.kind === 'roller') {
        const path = track.paths[o.seg];
        const k = sampleAt(path, o.s);
        v.mesh.position.set(_ob[0], _ob[1], _ob[2]);
        // Roll about the road's direction as it crosses.
        const dLat = _ob[3] - v.roll;
        v.roll = _ob[3];
        v.mesh.rotateOnWorldAxis(_p.set(path.tx[k], 0, path.tz[k]), -dLat / ROLLER_RADIUS);
      } else {
        const path = track.paths[o.seg];
        v.mesh.position.set(_ob[0], _ob[1] + 1.3 + _ob[3], _ob[2]);
        v.mesh.rotation.y = Math.atan2(path.tx[sampleAt(path, o.s)], path.tz[sampleAt(path, o.s)]);
        if (_ob[3] < 0.05 && v.roll > 0.05) for (let n = 0; n < 8; n++) this.smoke.spawn(_ob[0], _ob[1] + 0.3, _ob[2], (Math.random() - 0.5) * 8, 1, (Math.random() - 0.5) * 8, 0x8a8a8a, 2, 0.6, 0, 2);
        v.roll = _ob[3];
      }
    });
  }

  /** Coins spin in place; collected ones shrink away until they reappear. */
  private updateCoins(state: KartState): void {
    const mesh = this.coins;
    if (!mesh) return;
    const n = Math.min(mesh.count, state.coins.length);
    for (let i = 0; i < n; i++) {
      const c = state.coins[i];
      const shown = c.timer <= 0 ? 1 : 0;
      _p.set(c.x, c.y + Math.sin(this.time * 2.4 + i * 0.7) * 0.15, c.z);
      _q.setFromAxisAngle(_up, this.time * 3 + i * 0.4);
      mesh.setMatrixAt(i, _m.compose(_p, _q, _s.set(shown, shown, shown)));
    }
    mesh.instanceMatrix.needsUpdate = true;
  }

  private alive = new Set<number>();
  private syncPool<T extends { id: number }>(pool: Map<number, THREE.Object3D>, items: T[], key: string, scale: number, place: (o: THREE.Object3D, item: T) => void): void {
    const alive = this.alive;
    alive.clear();
    for (const item of items) {
      alive.add(item.id);
      let o = pool.get(item.id);
      if (!o) { o = this.lib.instance(key); o.scale.setScalar(scale); this.view.scene.add(o); pool.set(item.id, o); }
      place(o, item);
    }
    for (const [id, o] of pool) if (!alive.has(id)) { o.removeFromParent(); pool.delete(id); }
  }

  private updateKart(v: KartVisual, r: KartState['racers'][number], dt: number, isSelf: boolean): void {
    const hidden = r.respawn > 0;
    v.root.visible = !hidden;
    if (hidden) return;
    v.root.position.set(r.x, r.y, r.z);
    // Drift angle + spin-out spin are visual on top of the travel heading.
    const driftAngle = r.drift !== 0 ? -r.drift * 0.38 : 0;
    v.visualYaw = approach(v.visualYaw, driftAngle, 8, dt);
    if (r.spin > 0) v.spinAngle += dt * 14; else v.spinAngle = approach(wrapAngle(v.spinAngle), 0, 10, dt);
    v.root.rotation.y = r.yaw + v.visualYaw + v.spinAngle + (r.trick > 0 ? (0.45 - r.trick) / 0.45 * Math.PI * 2 : 0);
    // Road shape under the kart: banking rolls it, slopes pitch it.
    const path = this.track.paths[r.seg] ?? this.track;
    const k = r.hint >= 0 && r.hint < path.count ? r.hint : sampleAt(path, 0);
    const bank = r.grounded ? Math.atan(-path.bank[k]) : 0;
    const k2 = path.closed ? (k + 1) % path.count : Math.min(path.count - 1, k + 1);
    const slope = (path.py[k2] - path.py[k]) / Math.max(0.5, path.length / path.count);
    // Slope sign relative to the kart's heading (driving the wrong way flips it).
    const along = Math.sin(r.yaw) * path.tx[k] + Math.cos(r.yaw) * path.tz[k];
    v.tilt = approach(v.tilt, clamp(-r.steer * Math.abs(r.speed) * 0.004, -0.12, 0.12) + (r.drift !== 0 ? r.drift * 0.08 : 0) + bank * Math.sign(along || 1), 8, dt);
    v.body.rotation.z = v.tilt;
    v.pitch = r.grounded ? approach(v.pitch, -Math.atan(slope) * Math.sign(along || 1), 10, dt) : clamp(-r.vy * 0.02, -0.25, 0.25);
    v.body.rotation.x = v.pitch;
    v.bounce = approach(v.bounce, 0, 12, dt);
    v.wheelSpin += r.speed * dt / 0.42;
    for (const w of v.wheels) w.rotation.x = v.wheelSpin;
    for (const w of v.front) w.rotation.y = -r.steer * 0.45;
    v.mixer?.update(dt);
    // Glider canopy unfolds on launch.
    v.wingOpen = approach(v.wingOpen, r.glide ? 1 : 0, 8, dt);
    v.wing.visible = v.wingOpen > 0.02;
    if (v.wing.visible) { v.wing.scale.set(v.wingOpen, 1, 1); v.wing.rotation.z = -r.steer * 0.25 * v.wingOpen; }
    const boosting = r.boost > 0;
    for (const f of v.flames) {
      f.visible = boosting;
      if (boosting) { const s = 0.8 + Math.random() * 0.5; f.scale.set(1, s * (r.boostPower > 1.4 ? 1.4 : 1), 1); }
    }
    v.bubble.visible = r.star > 0;
    if (r.star > 0) { (v.bubble.material as THREE.MeshBasicMaterial).color.setHSL((this.time * 0.8) % 1, 1, 0.6); v.bubble.scale.setScalar(1 + Math.sin(this.time * 12) * 0.04); }
    // Particles from the rear wheels.
    const fx = Math.sin(r.yaw), fz = Math.cos(r.yaw), lx = fz, lz = -fx;
    const near = isSelf || Math.hypot(r.x - this.camPos.x, r.z - this.camPos.z) < 90;
    if (near && r.grounded && r.drift !== 0) {
      for (const side of [-1, 1]) {
        const x = r.x - fx * 1.0 + lx * side * 0.9, z = r.z - fz * 1.0 + lz * side * 0.9;
        this.particles.spawn(x, r.y + 0.25, z, (Math.random() - 0.5) * 3 - fx * 2, 1.5 + Math.random() * 2, (Math.random() - 0.5) * 3 - fz * 2, SPARK[r.driftLevel], r.driftLevel ? 0.45 : 0.3, 0.3, -9, 2);
      }
    }
    if (near && r.grounded && r.surface === 'grass' && Math.abs(r.speed) > 6 && Math.random() < 0.7) {
      this.smoke.spawn(r.x - fx * 1.2, r.y + 0.3, r.z - fz * 1.2, (Math.random() - 0.5) * 2, 1.2, (Math.random() - 0.5) * 2, this.track.def.theme === 'frost' ? 0xffffff : this.theme.bed, 1.1, 0.6, 0, 2);
    }
    // Underwater: a trail of bubbles rising to the surface.
    if (near && r.under && Math.random() < 0.6) this.particles.spawn(r.x - fx * 1.4 + (Math.random() - 0.5), r.y + 0.6, r.z - fz * 1.4 + (Math.random() - 0.5), (Math.random() - 0.5) * 0.6, 2 + Math.random() * 1.5, (Math.random() - 0.5) * 0.6, 0xd8f6ff, 0.35, 1.6, 1.5, 0.5);
    if (near && boosting && Math.random() < 0.8) this.particles.spawn(r.x - fx * 1.6, r.y + 0.6, r.z - fz * 1.6, -fx * 4, 0.5, -fz * 4, 0xffb347, 0.5, 0.25, 0, 3);
    if (r.star > 0 && Math.random() < 0.5) this.particles.spawn(r.x + (Math.random() - 0.5) * 2, r.y + Math.random() * 2, r.z + (Math.random() - 0.5) * 2, 0, 1, 0, 0xfff27a, 0.4, 0.5);
  }

  /** Blue fog once the camera dips under the water surface. */
  private updateFog(): void {
    const { track, theme } = this;
    const under = track.liquid !== null && !track.lava && this.view.camera.position.y < track.liquid;
    if (under === this.underwater) return;
    this.underwater = under;
    const fog = this.view.scene.fog as THREE.Fog;
    fog.color.setHex(under ? DEEP.color : theme.fog);
    fog.near = under ? DEEP.near : theme.fogNear;
    fog.far = under ? DEEP.far : theme.fogFar;
    (this.view.scene.background as THREE.Color).setHex(under ? DEEP.color : theme.skyHorizon);
  }
  /** Whether the camera is under water (the HUD tints the view). */
  get isUnderwater(): boolean { return this.underwater; }

  private updateCamera(state: KartState, self: KartState['racers'][number], dt: number, lookBack: boolean): void {
    const cam = this.view.camera;
    const path = this.track.paths[self.seg] ?? this.track;
    if (state.phase === 'intro') {
      // Sweep around the grid before the countdown.
      const t = Math.min(1, state.clock / 2.5);
      const ease = t * t * (3 - 2 * t);
      const a = self.yaw + Math.PI * (1 - ease);
      const d = lerp(34, 8, ease), h = lerp(30, 3.4, ease * ease);
      cam.position.set(self.x + Math.sin(a) * d, self.y + h, self.z + Math.cos(a) * d);
      // Keep the sweep above walls/cliffs beside the grid.
      const n0 = nearest(path, cam.position.x, cam.position.z, self.hint, _near, 40);
      cam.position.y = Math.max(cam.position.y, this.track.ground(cam.position.x, cam.position.z, n0) + 6);
      cam.lookAt(self.x, self.y + 1.2, self.z);
      this.camPos.copy(cam.position); this.camLook.set(self.x, self.y + 1.2, self.z); this.camReady = true;
      return;
    }
    const heading = r_heading(self);
    const back = lookBack ? -1 : 1;
    const dist = 7.8 + clamp(self.speed, 0, 40) * 0.05 + (self.glide ? 3 : 0);
    _v.set(self.x - Math.sin(heading) * dist * back, self.y + 3.4 + (self.glide ? 1.6 : 0), self.z - Math.cos(heading) * dist * back);
    // Keep the camera above the terrain, and under the roof in tunnels.
    const n = nearest(path, _v.x, _v.z, self.hint, _near, 20);
    const ground = this.track.ground(_v.x, _v.z, n, state.raceTime);
    if (Math.abs(n.lateral) < path.halfWidth + 6 || !(path.drop[n.index] > 0)) _v.y = Math.max(_v.y, ground + 1.5);
    if (path.flags[n.index] & F_TUNNEL && Math.abs(n.lateral) < path.halfWidth + 1) _v.y = Math.min(_v.y, n.y + TUNNEL_HEIGHT - 1.4);
    _look.set(self.x + Math.sin(heading) * 4 * back, self.y + 1.3, self.z + Math.cos(heading) * 4 * back);
    if (!this.camReady || lookBack) { this.camPos.copy(_v); this.camLook.copy(_look); this.camReady = true; }
    this.camPos.x = approach(this.camPos.x, _v.x, 7, dt); this.camPos.y = approach(this.camPos.y, _v.y, 5, dt); this.camPos.z = approach(this.camPos.z, _v.z, 7, dt);
    this.camLook.x = approach(this.camLook.x, _look.x, 14, dt); this.camLook.y = approach(this.camLook.y, _look.y, 10, dt); this.camLook.z = approach(this.camLook.z, _look.z, 14, dt);
    cam.position.copy(this.camPos);
    cam.lookAt(this.camLook);
    const fov = 68 + (self.boost > 0 ? 9 : 0) + clamp(self.speed - 20, 0, 20) * 0.25 + (self.glide ? 6 : 0);
    cam.fov = approach(cam.fov, fov, 4, dt);
    cam.updateProjectionMatrix();
  }

  stats() { return { ...this.view.stats(), camera: { x: this.camPos.x, y: this.camPos.y, z: this.camPos.z }, underwater: this.underwater }; }

  dispose(): void {
    this.particles.dispose();
    this.smoke.dispose();
    for (const k of this.karts) k.mixer?.stopAllAction();
    for (const o of this.obstacles) o.mesh.removeFromParent();
    this.view.dispose();
    this.lib.dispose();
  }
}

/** Sign text for a branch: its name and what kind of route it is. */
function routeLabel(path: Path): string {
  return path.boost ? `${path.name} · mushroom!` : path.shortcut ? `${path.name} · shortcut` : path.name;
}

/** Camera follows the direction of travel (velocity) blended with the heading. */
function r_heading(r: KartState['racers'][number]): number {
  const sp = Math.hypot(r.vx, r.vz);
  if (sp < 3 || r.speed < 0) return r.yaw;
  const vel = Math.atan2(r.vx, r.vz);
  return r.yaw + wrapAngle(vel - r.yaw) * 0.6;
}
