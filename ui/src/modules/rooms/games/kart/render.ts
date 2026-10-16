/**
 * Kart Grand Prix renderer: builds the track world (terrain, road, curbs or
 * barriers, ramps, dash panels, item boxes, themed Kenney props) and animates
 * karts, drivers, items and effects from simulation/snapshot state.
 */
import * as THREE from 'three';
import { AssetLibrary, PropBatch } from '../common/assets';
import { GameView, owned, type Theme } from '../common/view';
import { Particles } from '../common/particles';
import { approach, clamp, fbm, hashString, lerp, makeNoise, Rng, smoothstep, wrapAngle } from '../common/math';
import { asphaltTexture, checkerTexture, chevronTexture, curbTexture, groundDetailTexture, labelTexture, stripeTexture } from '../common/textures';
import { nearest, pointAt, rampHeight, RAMP_LENGTH, RAMP_HEIGHT, type Near, type Track } from './tracks';
import { racerDef, RACERS, type RacerId } from './roster';
import type { KartEvent, KartState } from './sim';

const THEMES: Record<Track['def']['theme'], Theme & { ground: number[]; far: number; water: number | null; road: string; edge: number }> = {
  meadow: { skyTop: 0x3f86d8, skyHorizon: 0xcde9ff, fog: 0xd6ecff, fogNear: 180, fogFar: 720, sun: 0xfff3dc, sunIntensity: 2.6, ambientSky: 0xcfe6ff, ambientGround: 0x5d7a3a, ambient: 1.1, ground: [0x6cbf4a, 0x58a93c, 0x8fcd5c, 0x7bb84f], far: -10, water: 0x3d9bd6, road: '#5a5e66', edge: 0x9bd36a },
  city: { skyTop: 0x4d8fd6, skyHorizon: 0xd7e7f5, fog: 0xd9e4ef, fogNear: 160, fogFar: 640, sun: 0xfff1d6, sunIntensity: 2.5, ambientSky: 0xdde8f5, ambientGround: 0x666a70, ambient: 1.15, ground: [0x9a9ea6, 0x8e939b, 0xa4a8ae, 0x7fae5a], far: 0, water: null, road: '#4d5158', edge: 0xb9bdc4 },
  canyon: { skyTop: 0x3a78c4, skyHorizon: 0xf5d9b0, fog: 0xf0d2a8, fogNear: 170, fogFar: 700, sun: 0xffe2b8, sunIntensity: 2.8, ambientSky: 0xffe7c9, ambientGround: 0x9a5a35, ambient: 1.05, ground: [0xd9955a, 0xc8763f, 0xe0ac70, 0xb86534], far: 34, water: null, road: '#6a5e58', edge: 0xe7b47a },
  frost: { skyTop: 0x6d9fd6, skyHorizon: 0xe8f2fc, fog: 0xe6eff9, fogNear: 150, fogFar: 640, sun: 0xffffff, sunIntensity: 2.2, ambientSky: 0xeaf3ff, ambientGround: 0x9fb3c8, ambient: 1.25, ground: [0xf3f7fc, 0xe2ebf5, 0xffffff, 0xd2deeb], far: 30, water: null, road: '#5d6670', edge: 0xffffff },
};
const KART_SCALE = 2.0;
const SPARK = [0x9a9a9a, 0x4fc3ff, 0xff9a2e, 0xd36bff];
const _near: Near = { index: 0, t: 0, s: 0, lateral: 0, y: 0, tx: 0, tz: 1, distance: 0 };
const _pt = [0, 0, 0, 0];
const _v = new THREE.Vector3();
const _look = new THREE.Vector3();

interface KartVisual {
  racer: RacerId; root: THREE.Group; body: THREE.Group; wheels: THREE.Object3D[]; front: THREE.Object3D[];
  mixer: THREE.AnimationMixer | null; flames: THREE.Mesh[]; bubble: THREE.Mesh; label: THREE.Sprite | null;
  wheelSpin: number; tilt: number; bounce: number; visualYaw: number; spinAngle: number;
}

export interface RenderRacer { id: number; racer: RacerId; human: boolean; name: string }

export class KartRenderer {
  readonly view: GameView;
  private lib: AssetLibrary;
  private track: Track;
  private theme: (typeof THEMES)[keyof typeof THEMES];
  private karts: KartVisual[] = [];
  private boxes: THREE.Object3D[] = [];
  private bananas = new Map<number, THREE.Object3D>();
  private bombs = new Map<number, THREE.Object3D>();
  private particles = new Particles(2400, true);
  private smoke = new Particles(600, false);
  private camPos = new THREE.Vector3();
  private camLook = new THREE.Vector3();
  private camReady = false;
  private time = 0;
  private flameGeo = new THREE.ConeGeometry(0.22, 1.2, 10, 1, true);
  private flameMat = new THREE.MeshBasicMaterial({ color: 0xff8a2a, transparent: true, opacity: 0.85, blending: THREE.AdditiveBlending, depthWrite: false });
  private bubbleGeo = new THREE.SphereGeometry(2.0, 20, 14);
  private bubbleMat = new THREE.MeshBasicMaterial({ color: 0xffe066, transparent: true, opacity: 0.22, blending: THREE.AdditiveBlending, depthWrite: false });
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
    for (const m of [this.flameGeo, this.flameMat, this.bubbleGeo, this.bubbleMat]) this.lib.own(m);
  }

  // ── World ────────────────────────────────────────────────────────────────
  private buildWorld(): void {
    const { track, theme } = this;
    const scene = this.view.scene;
    const noise = makeNoise(hashString(track.def.id) ^ 0x51ed);
    let minX = Infinity, maxX = -Infinity, minZ = Infinity, maxZ = -Infinity;
    for (let i = 0; i < track.count; i++) {
      minX = Math.min(minX, track.px[i]); maxX = Math.max(maxX, track.px[i]);
      minZ = Math.min(minZ, track.pz[i]); maxZ = Math.max(maxZ, track.pz[i]);
    }
    const margin = 260, step = 4;
    const x0 = minX - margin, z0 = minZ - margin;
    const nx = Math.ceil((maxX - minX + margin * 2) / step) + 1, nz = Math.ceil((maxZ - minZ + margin * 2) / step) + 1;
    const positions = new Float32Array(nx * nz * 3), colors = new Float32Array(nx * nz * 3), uvs = new Float32Array(nx * nz * 2);
    const palette = theme.ground.map(c => new THREE.Color(c));
    const edge = new THREE.Color(theme.edge);
    const tmp = new THREE.Color();
    const coarse = (x: number, z: number): Near => {
      // Coarse global search, then refine locally.
      let best = Infinity, bi = 0;
      for (let i = 0; i < track.count; i += 6) {
        const d = (track.px[i] - x) ** 2 + (track.pz[i] - z) ** 2;
        if (d < best) { best = d; bi = i; }
      }
      return nearest(track, x, z, bi, _near, 8);
    };
    for (let iz = 0; iz < nz; iz++) for (let ix = 0; ix < nx; ix++) {
      const k = iz * nx + ix, x = x0 + ix * step, z = z0 + iz * step;
      const near = coarse(x, z);
      const off = Math.abs(near.lateral) - track.halfWidth;
      let y = off <= 1 ? near.y - 0.35 : track.ground(x, z, near);
      const far = smoothstep(60, 240, off);
      y += far * theme.far * (0.55 + 0.45 * fbm(noise, x * 0.006, z * 0.006, 3));
      positions[k * 3] = x; positions[k * 3 + 1] = y; positions[k * 3 + 2] = z;
      uvs[k * 2] = x / 6; uvs[k * 2 + 1] = z / 6;
      const n = fbm(noise, x * 0.03, z * 0.03, 3) * 0.5 + 0.5;
      tmp.copy(palette[0]).lerp(palette[1], n);
      tmp.lerp(palette[2], smoothstep(0.55, 0.9, fbm(noise, x * 0.011 + 40, z * 0.011, 2) * 0.5 + 0.5) * 0.6);
      if (track.def.theme === 'city') tmp.lerp(palette[3], smoothstep(30, 60, off) * smoothstep(0.4, 0.7, n));
      if (track.def.theme === 'canyon') tmp.lerp(palette[3], smoothstep(4, 30, y - near.y) * 0.8);
      if (off < 3.5 && off > -1) tmp.lerp(edge, 0.55 * (1 - smoothstep(0, 3.5, off)));
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
    if (theme.water !== null) {
      const water = owned(new THREE.Mesh(new THREE.PlaneGeometry(4000, 4000), new THREE.MeshLambertMaterial({ color: theme.water, transparent: true, opacity: 0.92 })));
      water.rotation.x = -Math.PI / 2; water.position.y = -3.2;
      scene.add(water);
    }
    this.buildRoad();
    this.buildProps();
    for (let i = 0; i < track.def.itemRows.length * 5; i++) {
      const box = this.lib.instance('toy-item-box');
      box.scale.setScalar(4);
      scene.add(box);
      this.boxes.push(box);
    }
  }

  /** Strip mesh following the centre line between two lateral offsets. */
  private strip(fromLat: number, toLat: number, lift: number, vScale: number, material: THREE.Material, filter?: (i: number) => boolean, withRamp = true): THREE.Mesh {
    const { track } = this;
    const pos: number[] = [], uv: number[] = [], idx: number[] = [];
    let v = 0, open = false;
    for (let i = 0; i <= track.count; i++) {
      const k = i % track.count;
      if (filter && !filter(k)) { open = false; continue; }
      const s = track.dist[k];
      const ramp = withRamp ? rampHeight(track.def, track.length, s) : 0;
      const lx = track.tz[k], lz = -track.tx[k];
      const y = track.py[k] + lift + ramp;
      pos.push(track.px[k] + lx * fromLat, y, track.pz[k] + lz * fromLat, track.px[k] + lx * toLat, y, track.pz[k] + lz * toLat);
      v = i === 0 ? 0 : v + (track.length / track.count) / vScale;
      uv.push(0, v, 1, v);
      const n = pos.length / 3;
      if (open) idx.push(n - 4, n - 2, n - 3, n - 3, n - 2, n - 1);
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

  private buildRoad(): void {
    const { track, theme, view } = this;
    const hw = track.halfWidth;
    const asphalt = this.lib.own(asphaltTexture(theme.road));
    view.scene.add(this.strip(-hw, hw, 0.06, 14, new THREE.MeshLambertMaterial({ map: asphalt })));
    const ice = track.def.ice;
    if (ice.length) {
      const iceMat = new THREE.MeshLambertMaterial({ color: 0xbfe6ff, transparent: true, opacity: 0.75 });
      view.scene.add(this.strip(-hw, hw, 0.09, 10, iceMat, i => ice.some(([a, b]) => track.dist[i] / track.length >= a && track.dist[i] / track.length <= b)));
    }
    if (track.def.edge === 'grass') {
      const curb = new THREE.MeshLambertMaterial({ map: this.lib.own(curbTexture()) });
      view.scene.add(this.strip(hw, hw + 1.1, 0.08, 3, curb), this.strip(-hw - 1.1, -hw, 0.08, 3, curb));
    } else {
      // Continuous barrier walls matching the collision limit.
      const wallMat = new THREE.MeshLambertMaterial({ map: this.lib.own(curbTexture(track.def.theme === 'city' ? '#d8322f' : '#b5562c', track.def.theme === 'city' ? '#f4f4f4' : '#e8c79a')) });
      for (const side of [-1, 1]) view.scene.add(this.wall(side * (hw + 1.4), side, wallMat));
    }
    // Start line + finish gate.
    const checker = new THREE.MeshBasicMaterial({ map: this.lib.own(checkerTexture()) });
    const start = owned(new THREE.Mesh(new THREE.PlaneGeometry(hw * 2, 2.4), checker));
    pointAt(track, 0, 0, _pt);
    start.position.set(_pt[0], _pt[1] + 0.1, _pt[2]);
    start.rotation.set(-Math.PI / 2, _pt[3], 0, 'YXZ');
    view.scene.add(start);
    // Dash panels.
    const chevron = new THREE.MeshBasicMaterial({ map: this.lib.own(chevronTexture()), toneMapped: false });
    for (const pad of track.def.boostPads) {
      const mesh = owned(new THREE.Mesh(new THREE.PlaneGeometry(4.4, 4.4), chevron));
      pointAt(track, pad.at * track.length, pad.lane * hw, _pt);
      mesh.position.set(_pt[0], _pt[1] + 0.12, _pt[2]);
      mesh.rotation.set(-Math.PI / 2, _pt[3] + Math.PI, 0, 'YXZ');
      view.scene.add(mesh);
    }
    // Ramps: striped deck over the rising road + lip face.
    const stripes = new THREE.MeshLambertMaterial({ map: this.lib.own(stripeTexture()) });
    for (const at of track.def.ramps) {
      const s0 = at * track.length - RAMP_LENGTH;
      view.scene.add(this.strip(-hw, hw, 0.1, 2, stripes, i => {
        const d = track.dist[i] - s0;
        return d >= -2 && d <= RAMP_LENGTH + 2;
      }));
      pointAt(track, at * track.length, 0, _pt);
      const face = owned(new THREE.Mesh(new THREE.BoxGeometry(hw * 2, RAMP_HEIGHT, 0.5), new THREE.MeshLambertMaterial({ color: 0x333333 })));
      face.position.set(_pt[0], _pt[1] + RAMP_HEIGHT / 2 + 0.05, _pt[2]);
      face.rotation.y = _pt[3];
      view.scene.add(face);
    }
  }

  private wall(lateral: number, side: number, material: THREE.Material): THREE.Mesh {
    const { track } = this;
    const pos: number[] = [], uv: number[] = [], idx: number[] = [];
    const h = track.def.theme === 'city' ? 1.0 : 1.3, t = 0.6;
    for (let i = 0; i <= track.count; i++) {
      const k = i % track.count;
      const lx = track.tz[k], lz = -track.tx[k];
      const bx = track.px[k] + lx * lateral, bz = track.pz[k] + lz * lateral, y = track.py[k];
      const ox = lx * side * t, oz = lz * side * t;
      // inner bottom, inner top, outer top, outer bottom
      pos.push(bx, y - 0.4, bz, bx, y + h, bz, bx + ox, y + h, bz + oz, bx + ox, y - 0.4, bz + oz);
      const v = track.dist[k] / 4 + (i === track.count ? track.length / 4 : 0);
      uv.push(0, v, 0.5, v, 0.75, v, 1, v);
      if (i > 0) {
        const n = pos.length / 3, a = n - 8, b = n - 4;
        for (let f = 0; f < 3; f++) {
          const p = a + f, q = a + f + 1, r = b + f, s = b + f + 1;
          if (side > 0) idx.push(p, r, q, q, r, s); else idx.push(p, q, r, q, s, r);
        }
      }
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

  private buildProps(): void {
    const { track } = this;
    const batch = new PropBatch(this.lib);
    const rng = new Rng(hashString(track.def.id) ^ 0x2f1);
    const hw = track.halfWidth;
    const clearOf = (x: number, z: number, gap: number) => {
      let best = Infinity;
      for (let i = 0; i < track.count; i += 3) best = Math.min(best, (track.px[i] - x) ** 2 + (track.pz[i] - z) ** 2);
      return Math.sqrt(best) > hw + gap;
    };
    const groundAt = (x: number, z: number) => {
      let best = Infinity, bi = 0;
      for (let i = 0; i < track.count; i += 6) { const d = (track.px[i] - x) ** 2 + (track.pz[i] - z) ** 2; if (d < best) { best = d; bi = i; } }
      const n = nearest(track, x, z, bi, _near, 8);
      return track.ground(x, z, n);
    };
    /** Place along the track at lateral distance (beyond the road edge). */
    const beside = (key: string, s: number, side: number, dist: number, scale: number, faceRoad = true, jitterYaw = 0) => {
      pointAt(track, s, side * (hw + dist), _pt);
      if (!clearOf(_pt[0], _pt[2], dist * 0.6)) return;
      const yaw = faceRoad ? _pt[3] + (side > 0 ? -Math.PI / 2 : Math.PI / 2) : rng.range(0, Math.PI * 2);
      batch.add(key, _pt[0], groundAt(_pt[0], _pt[2]) - 0.05, _pt[2], yaw + rng.range(-jitterYaw, jitterYaw), scale);
    };
    const scatter = (keys: string[], count: number, minD: number, maxD: number, scale: [number, number]) => {
      for (let n = 0; n < count; n++) {
        const s = rng.range(0, track.length), side = rng.chance(0.5) ? 1 : -1, d = rng.range(minD, maxD);
        beside(rng.pick(keys), s, side, d, rng.range(scale[0], scale[1]), false);
      }
    };
    // Solid structures stay beyond the reachable area: outside the walls, or
    // past the rescue line on grass tracks (karts never drive through them).
    const gap = track.def.edge === 'grass' ? track.def.outOfBounds + 2 : 3;
    // Start area: grandstands, banner towers, flags, tents.
    for (let s = -70; s <= 70; s += 12) {
      beside(track.def.theme === 'city' ? 'race-grandStandCovered' : 'race-grandStand', s, 1, gap + 6, 11);
      if (Math.abs(s) > 20) beside('race-tentLong', s, -1, gap + 3, 6);
    }
    beside('race-pitsGarage', -24, -1, gap + 5, 10); beside('race-pitsOffice', 0, -1, gap + 5, 10); beside('race-pitsGarage', 24, -1, gap + 5, 10);
    for (let s = -90; s < 110; s += 30) { beside('race-bannerTowerRed', s, -1, gap + 0.5, 6); beside('race-bannerTowerGreen', s + 15, 1, gap + 0.5, 6); }
    pointAt(track, 2, 0, _pt);
    batch.add('toy-gate-finish', _pt[0], _pt[1], _pt[2], _pt[3], (hw * 2 + 6) / 1.55, 9);
    // Billboards and flags on corners.
    for (let i = 0; i < track.count; i += 22) {
      if (Math.abs(track.curve[i]) < 0.02) continue;
      const side = track.curve[i] > 0 ? -1 : 1; // outside of the bend
      beside(rng.chance(0.5) ? 'race-billboard' : 'race-billboardLow', track.dist[i], side, gap + 4, 9);
      beside(rng.pick(['race-flagRed', 'race-flagGreen', 'race-flagCheckers']), track.dist[i] + 8, side, gap, 4);
    }
    const t = track.def.theme;
    if (t === 'meadow') {
      scatter(['nat-tree_default', 'nat-tree_oak', 'nat-tree_detailed', 'nat-tree_fat', 'race-treeLarge'], 520, gap, 150, [6, 9]);
      scatter(['nat-plant_bushLarge', 'nat-plant_bush', 'nat-flower_redA', 'nat-flower_yellowB', 'nat-flower_purpleA', 'nat-mushroom_redGroup'], 600, 3, 40, [5, 8]);
      scatter(['nat-tent_detailedOpen', 'nat-log_stack', 'nat-stump_old'], 50, gap, 50, [6, 8]);
      for (let i = 0; i < track.count; i += 7) if (Math.abs(track.curve[i]) > 0.018) beside('nat-fence_simple', track.dist[i], track.curve[i] > 0 ? -1 : 1, gap - 1, 5);
    } else if (t === 'city') {
      const blocks = ['city-building-a', 'city-building-b', 'city-building-c', 'city-building-d', 'city-building-e', 'city-building-f', 'city-building-g', 'city-building-h', 'city-building-i', 'city-building-j', 'city-building-k', 'city-building-l'];
      for (let s = 0; s < track.length; s += 15) for (const side of [-1, 1]) {
        if (Math.abs(((s + 70) % track.length) - 70) < 80 && side > 0) continue; // grandstands
        beside(rng.pick(blocks), s, side, rng.range(12, 16), rng.range(10, 12));
        if (rng.chance(0.25)) beside('race-lightPostLarge', s + 7, side, 2.4, 7);
      }
      scatter(['city-building-skyscraper-a', 'city-building-skyscraper-b', 'city-building-skyscraper-c', 'city-building-skyscraper-d'], 120, 40, 200, [12, 15]);
      scatter(['house-tree-large', 'house-tree-small', 'city-detail-parasol-a'], 160, 4, 10, [9, 12]);
    } else if (t === 'canyon') {
      for (let s = 0; s < track.length; s += 9) for (const side of [-1, 1]) {
        if (rng.chance(0.75)) beside(rng.pick(['nat-cliff_block_rock', 'nat-cliff_large_rock', 'nat-rock_tallB']), s, side, rng.range(5, 9), rng.range(9, 14), true, 0.4);
      }
      scatter(['nat-rock_tallB', 'nat-rock_tallE', 'nat-rock_tallG', 'nat-cliff_block_rock'], 260, 20, 180, [12, 26]);
      scatter(['nat-cactus_tall', 'nat-cactus_short', 'nat-rock_largeA', 'nat-rock_largeC'], 260, 6, 70, [5, 9]);
      scatter(['nat-tree_palmTall', 'nat-tree_palmBend'], 40, 10, 60, [6, 8]);
    } else {
      scatter(['plat-tree-pine-snow', 'plat-tree-snow', 'plat-tree-pine-snow-small'], 620, gap, 170, [4, 7]);
      scatter(['plat-rocks', 'plat-block-snow-large', 'nat-rock_largeA'], 120, gap, 90, [4, 8]);
      scatter(['plat-stones'], 90, 4, gap - 2, [3, 5]);
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
    let label: THREE.Sprite | null = null;
    if (r.human && r.id !== this.self) {
      const tex = this.lib.own(labelTexture(r.name, '#ffffff'));
      label = new THREE.Sprite(this.lib.own(new THREE.SpriteMaterial({ map: tex, depthTest: false, transparent: true })));
      label.scale.set(4, 1, 1); label.position.y = 3.6; label.renderOrder = 10;
      root.add(label);
    }
    this.view.scene.add(root);
    return { racer: r.racer, root, body, wheels, front, mixer, flames, bubble, label, wheelSpin: 0, tilt: 0, bounce: 0, visualYaw: 0, spinAngle: 0 };
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
    this.syncPool(this.bananas, state.hazards, 'toy-item-banana', 5, (o, h) => { o.position.set(h.x, h.y + 0.05, h.z); });
    this.syncPool(this.bombs, state.shells, 'plat-bomb', 2.6, (o, s) => {
      o.position.set(s.x, s.y - 0.4 + Math.abs(Math.sin(this.time * 9)) * 0.4, s.z); o.rotation.y = s.yaw;
      if (Math.random() < 0.6) this.particles.spawn(s.x, s.y + 0.6, s.z, 0, 1.5, 0, 0xffa040, 0.5, 0.35);
    });
    this.particles.update(dt);
    this.smoke.update(dt);
    this.updateCamera(state, self, dt, lookBack);
    this.view.focusShadow(self.x, self.y, self.z);
    this.view.render();
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
    v.tilt = approach(v.tilt, clamp(-r.steer * Math.abs(r.speed) * 0.004, -0.12, 0.12) + (r.drift !== 0 ? r.drift * 0.08 : 0), 8, dt);
    v.body.rotation.z = v.tilt;
    v.body.rotation.x = r.grounded ? approach(v.body.rotation.x, 0, 10, dt) : clamp(-r.vy * 0.02, -0.25, 0.25);
    v.bounce = approach(v.bounce, 0, 12, dt);
    v.wheelSpin += r.speed * dt / 0.42;
    for (const w of v.wheels) w.rotation.x = v.wheelSpin;
    for (const w of v.front) w.rotation.y = -r.steer * 0.45;
    v.mixer?.update(dt);
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
      this.smoke.spawn(r.x - fx * 1.2, r.y + 0.3, r.z - fz * 1.2, (Math.random() - 0.5) * 2, 1.2, (Math.random() - 0.5) * 2, this.track.def.theme === 'frost' ? 0xffffff : 0x8a7a55, 1.1, 0.6, 0, 2);
    }
    if (near && boosting && Math.random() < 0.8) this.particles.spawn(r.x - fx * 1.6, r.y + 0.6, r.z - fz * 1.6, -fx * 4, 0.5, -fz * 4, 0xffb347, 0.5, 0.25, 0, 3);
    if (r.star > 0 && Math.random() < 0.5) this.particles.spawn(r.x + (Math.random() - 0.5) * 2, r.y + Math.random() * 2, r.z + (Math.random() - 0.5) * 2, 0, 1, 0, 0xfff27a, 0.4, 0.5);
  }

  private updateCamera(state: KartState, self: KartState['racers'][number], dt: number, lookBack: boolean): void {
    const cam = this.view.camera;
    if (state.phase === 'intro') {
      // Sweep around the grid before the countdown.
      const t = Math.min(1, state.clock / 2.5);
      const ease = t * t * (3 - 2 * t);
      const a = self.yaw + Math.PI * (1 - ease);
      const d = lerp(30, 8, ease), h = lerp(22, 3.4, ease);
      cam.position.set(self.x + Math.sin(a) * d, self.y + h, self.z + Math.cos(a) * d);
      cam.lookAt(self.x, self.y + 1.2, self.z);
      this.camPos.copy(cam.position); this.camLook.set(self.x, self.y + 1.2, self.z); this.camReady = true;
      return;
    }
    const heading = r_heading(self);
    const back = lookBack ? -1 : 1;
    const dist = 7.8 + clamp(self.speed, 0, 40) * 0.05;
    _v.set(self.x - Math.sin(heading) * dist * back, self.y + 3.4, self.z - Math.cos(heading) * dist * back);
    // Keep the camera above the terrain.
    const n = nearest(this.track, _v.x, _v.z, self.hint, _near, 20);
    _v.y = Math.max(_v.y, this.track.ground(_v.x, _v.z, n) + 1.5);
    _look.set(self.x + Math.sin(heading) * 4 * back, self.y + 1.3, self.z + Math.cos(heading) * 4 * back);
    if (!this.camReady || lookBack) { this.camPos.copy(_v); this.camLook.copy(_look); this.camReady = true; }
    this.camPos.x = approach(this.camPos.x, _v.x, 7, dt); this.camPos.y = approach(this.camPos.y, _v.y, 5, dt); this.camPos.z = approach(this.camPos.z, _v.z, 7, dt);
    this.camLook.x = approach(this.camLook.x, _look.x, 14, dt); this.camLook.y = approach(this.camLook.y, _look.y, 10, dt); this.camLook.z = approach(this.camLook.z, _look.z, 14, dt);
    cam.position.copy(this.camPos);
    cam.lookAt(this.camLook);
    const fov = 68 + (self.boost > 0 ? 9 : 0) + clamp(self.speed - 20, 0, 20) * 0.25;
    cam.fov = approach(cam.fov, fov, 4, dt);
    cam.updateProjectionMatrix();
  }

  stats() { return { ...this.view.stats(), camera: { x: this.camPos.x, y: this.camPos.y, z: this.camPos.z } }; }

  dispose(): void {
    this.particles.dispose();
    this.smoke.dispose();
    for (const k of this.karts) k.mixer?.stopAllAction();
    this.view.dispose();
    this.lib.dispose();
  }
}

/** Camera follows the direction of travel (velocity) blended with the heading. */
function r_heading(r: KartState['racers'][number]): number {
  const sp = Math.hypot(r.vx, r.vz);
  if (sp < 3 || r.speed < 0) return r.yaw;
  const vel = Math.atan2(r.vx, r.vz);
  return r.yaw + wrapAngle(vel - r.yaw) * 0.6;
}
