/**
 * The static world for Storm Royale's renderer: the terrain (vertex-coloured
 * heightfield with a textured PBR surface), sea / lakes / fountains, the
 * walk-in architecture merged into one mesh per surface family (plaster,
 * brick, concrete, glass, rust… each with its own procedural texture and
 * roughness), Dead City's streets and pavements, kit props batched per 120 m
 * chunk so frustum culling works (decor fades out past 140 m, charred wrecks
 * get a burnt copy of their materials), and the fires that burn on the map.
 */
import * as THREE from 'three';
import { PropBatch, type AssetLibrary } from '../../common/assets';
import { owned } from '../../common/view';
import { Particles } from '../../common/particles';
import { smoothstep } from '../../common/math';
import { pavementTexture, streetTexture, surfaceTexture, type SurfaceKind } from '../../common/textures';
import { BLOCK_KINDS, GRID, HALF, SEA_SURFACE, terrainHeight, type Fire, type Road, type ShooterMap } from '../map';
import { blockGeometry } from '../meshes';

export interface GroundColours { low: number; mid: number; high: number; rock: number; sand: number }
const CHUNK = 120, DECOR_RANGE = 140;
const DECOR = /grass|flower|bush|plant_|stones|mushroom/;
/** Roughness / metalness per surface family. */
const FINISH: Partial<Record<SurfaceKind, [number, number]>> = {
  plaster: [0.92, 0], wood: [0.85, 0], roof: [0.8, 0], metal: [0.5, 0.55], concrete: [0.95, 0], brick: [0.9, 0], rubble: [0.97, 0],
  charred: [1, 0], sandbag: [0.98, 0], asphalt: [0.95, 0], tile: [0.45, 0.05], glass: [0.12, 0.6], rust: [0.75, 0.35], hazard: [0.7, 0.1], planter: [1, 0],
};

/** The heightfield: colours by height, slope and theme, multiplied by a ground texture. */
export function terrainMesh(map: ShooterMap, c: GroundColours, own: <T extends { dispose(): void }>(r: T) => T): THREE.Mesh {
  const n = map.size;
  const pos = new Float32Array(n * n * 3), col = new Float32Array(n * n * 3), uv = new Float32Array(n * n * 2);
  const k = new THREE.Color(), low = new THREE.Color(c.low), mid = new THREE.Color(c.mid), high = new THREE.Color(c.high), rock = new THREE.Color(c.rock), sand = new THREE.Color(c.sand);
  const lake = map.waters[0], city = map.theme === 'city';
  const park = map.pois.find(p => p.kind === 'park'), moss = new THREE.Color(0x56683a), scorch = new THREE.Color(0x2a2522);
  for (let iz = 0; iz < n; iz++) for (let ix = 0; ix < n; ix++) {
    const i = iz * n + ix, x = -HALF + ix * GRID, z = -HALF + iz * GRID, h = map.heights[i];
    pos[i * 3] = x; pos[i * 3 + 1] = h; pos[i * 3 + 2] = z;
    uv[i * 2] = x / 4; uv[i * 2 + 1] = z / 4;
    const hx = map.heights[iz * n + Math.min(n - 1, ix + 1)] - map.heights[iz * n + Math.max(0, ix - 1)];
    const hz = map.heights[Math.min(n - 1, iz + 1) * n + ix] - map.heights[Math.max(0, iz - 1) * n + ix];
    const slope = Math.hypot(hx, hz) / (GRID * 2);
    k.copy(low).lerp(mid, smoothstep(2, 9, h)).lerp(high, smoothstep(9, 16, h) * 0.6);
    // Patches and worn ground for variety.
    const patch = Math.sin(x * 0.083 + Math.cos(z * 0.05) * 2) * Math.cos(z * 0.071 + Math.sin(x * 0.04) * 2);
    k.lerp(map.theme === 'isle' ? mid : high, smoothstep(0.2, 0.8, patch) * 0.5);
    k.multiplyScalar(0.9 + 0.1 * Math.sin(x * 0.31) * Math.sin(z * 0.27));
    if (map.theme === 'isle') k.lerp(sand, 1 - smoothstep(0.6, 2.2, h));
    else if (city) {
      // Dead lots, a mossy park, scorched craters and the berm's rubble.
      if (park) k.lerp(moss, smoothstep(park.r + 12, park.r - 6, Math.hypot(x - park.x, z - park.z)) * 0.85);
      k.lerp(scorch, smoothstep(-0.3, -1.4, h) * 0.8);
      k.lerp(sand, smoothstep(0.3, 0.7, patch) * 0.25);
    } else {
      k.lerp(sand, smoothstep(0.3, 0.6, Math.sin(x * 0.05) * Math.cos(z * 0.04) * 0.5 + 0.5) * 0.35);
      if (lake) k.lerp(sand, 1 - smoothstep(lake.y + 0.2, lake.y + 1.6, h)).lerp(mid, smoothstep(lake.r + 10, lake.r, Math.hypot(x - lake.x, z - lake.z)) * 0.25);
    }
    k.lerp(rock, smoothstep(0.45, 0.9, slope));
    col[i * 3] = k.r; col[i * 3 + 1] = k.g; col[i * 3 + 2] = k.b;
  }
  const index = new Uint32Array((n - 1) * (n - 1) * 6);
  let o = 0;
  for (let iz = 0; iz < n - 1; iz++) for (let ix = 0; ix < n - 1; ix++) { const a = iz * n + ix, b = a + 1, cc = a + n, d = cc + 1; index[o++] = a; index[o++] = cc; index[o++] = b; index[o++] = b; index[o++] = cc; index[o++] = d; }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(pos, 3));
  geo.setAttribute('color', new THREE.BufferAttribute(col, 3));
  geo.setAttribute('uv', new THREE.BufferAttribute(uv, 2));
  geo.setIndex(new THREE.BufferAttribute(index, 1)); geo.computeVertexNormals();
  const tex = own(surfaceTexture(map.theme === 'isle' ? 'grass' : 'dirt', 3));
  const mesh = owned(new THREE.Mesh(geo, new THREE.MeshStandardMaterial({ vertexColors: true, map: tex, roughness: 0.96, metalness: 0 })));
  mesh.receiveShadow = true;
  return mesh;
}

/** Sea (isle), desert floor (mesa) or ash flats (city) beyond the edge, and every lake / fountain. */
export function waterMeshes(map: ShooterMap, floor: number): THREE.Mesh[] {
  const out: THREE.Mesh[] = [];
  const water = () => new THREE.MeshStandardMaterial({ color: 0x2b7fbf, roughness: 0.12, metalness: 0.2, transparent: true, opacity: 0.86 });
  if (map.theme === 'isle') {
    const sea = owned(new THREE.Mesh(new THREE.PlaneGeometry(3000, 3000), water()));
    sea.rotation.x = -Math.PI / 2; sea.position.y = SEA_SURFACE;
    out.push(sea);
  } else {
    const ground = owned(new THREE.Mesh(new THREE.PlaneGeometry(3000, 3000), new THREE.MeshStandardMaterial({ color: floor, roughness: 1 })));
    ground.rotation.x = -Math.PI / 2; ground.position.y = -1;
    out.push(ground);
    for (const w of map.waters) {
      const m = owned(new THREE.Mesh(new THREE.CircleGeometry(w.r, 48), map.theme === 'city' ? new THREE.MeshStandardMaterial({ color: 0x3a4a3a, roughness: 0.08, metalness: 0.3, transparent: true, opacity: 0.9 }) : water()));
      m.rotation.x = -Math.PI / 2; m.position.set(w.x, w.y, w.z);
      out.push(m);
    }
  }
  return out;
}

/** All blocks as one merged mesh per surface family, each with its texture and finish. */
export function architectureMeshes(map: ShooterMap, own: <T extends { dispose(): void }>(r: T) => T): THREE.Mesh[] {
  const groups = new Map<SurfaceKind, typeof map.blocks>();
  for (const b of map.blocks) {
    const kind = (BLOCK_KINDS[b.mat] ?? 'plaster') as SurfaceKind;
    const list = groups.get(kind);
    if (list) list.push(b); else groups.set(kind, [b]);
  }
  const out: THREE.Mesh[] = [];
  for (const [kind, blocks] of groups) {
    const [roughness, metalness] = FINISH[kind] ?? [0.9, 0];
    const mesh = owned(new THREE.Mesh(blockGeometry(blocks), new THREE.MeshStandardMaterial({ vertexColors: true, map: own(surfaceTexture(kind)), roughness, metalness })));
    mesh.castShadow = true; mesh.receiveShadow = true; mesh.name = `arch-${kind}`;
    out.push(mesh);
  }
  return out;
}

/** A strip following the terrain along a road, between `from` and `to` metres off its centre line. */
function strip(map: ShooterMap, roads: Road[], from: number, to: number, lift: (r: Road) => number, vScale: number): THREE.BufferGeometry {
  const pos: number[] = [], uv: number[] = [], idx: number[] = [];
  for (const r of roads) {
    const len = Math.hypot(r.x1 - r.x0, r.z1 - r.z0), dx = (r.x1 - r.x0) / len, dz = (r.z1 - r.z0) / len, px = -dz, pz = dx;
    const steps = Math.ceil(len / 2), base = pos.length / 3, y0 = lift(r);
    for (let s = 0; s <= steps; s++) {
      const t = (s / steps) * len;
      for (const [o, u] of [[from, 0], [(from + to) / 2, 0.5], [to, 1]]) {
        const x = r.x0 + dx * t + px * o, z = r.z0 + dz * t + pz * o;
        pos.push(x, terrainHeight(map, x, z) + y0, z);
        uv.push(u, t / vScale);
      }
      if (s > 0) for (let c = 0; c < 2; c++) { const a = base + (s - 1) * 3 + c, b = a + 3; idx.push(a, a + 1, b, a + 1, b + 1, b); }
    }
  }
  const g = new THREE.BufferGeometry();
  g.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
  g.setAttribute('uv', new THREE.Float32BufferAttribute(uv, 2));
  g.setIndex(idx); g.computeVertexNormals();
  return g;
}
/** Dead City's streets (cracked asphalt with faded lines) and their pavements. */
export function roadMeshes(map: ShooterMap, own: <T extends { dispose(): void }>(r: T) => T): THREE.Mesh[] {
  if (!map.roads.length) return [];
  // North-south streets sit a hair below east-west ones so crossings never z-fight.
  const lift = (r: Road) => (r.x0 === r.x1 ? 0.05 : 0.07);
  const half = map.roads[0].w / 2;
  const road = owned(new THREE.Mesh(strip(map, map.roads, -half, half, lift, 24), new THREE.MeshStandardMaterial({ map: own(streetTexture()), roughness: 0.93, polygonOffset: true, polygonOffsetFactor: -2, polygonOffsetUnits: -2 })));
  const pg = [strip(map, map.roads, half, half + 3, r => lift(r) + 0.1, 3), strip(map, map.roads, -half - 3, -half, r => lift(r) + 0.1, 3)];
  const walk = own(pavementTexture());
  const pavements = pg.map(g => owned(new THREE.Mesh(g, new THREE.MeshStandardMaterial({ map: walk, roughness: 0.95, polygonOffset: true, polygonOffsetFactor: -1, polygonOffsetUnits: -1 }))));
  for (const m of [road, ...pavements]) m.receiveShadow = true;
  return [road, ...pavements];
}

/**
 * Kit props batched per 120 m chunk (solid, decor, burnt), so off-screen
 * chunks are culled and far decor is hidden; keeps where each tree trunk's
 * instance lives so felled trees can be hidden.
 */
export class PropChunks {
  readonly group = new THREE.Group();
  readonly trees: { meshes: THREE.InstancedMesh[]; index: number }[];
  private decor: { group: THREE.Group; x: number; z: number }[] = [];
  private burnt = new Map<THREE.Material, THREE.Material>();

  constructor(map: ShooterMap, lib: AssetLibrary) {
    const side = Math.ceil((HALF * 2) / CHUNK);
    const chunkOf = (x: number, z: number) => Math.min(side - 1, Math.max(0, Math.floor((x + HALF) / CHUNK))) + Math.min(side - 1, Math.max(0, Math.floor((z + HALF) / CHUNK))) * side;
    const batches = new Map<string, PropBatch>();
    const counters = new Map<string, number>();
    const where = new Int32Array(map.props.length), slot = new Int32Array(map.props.length);
    map.props.forEach((p, i) => {
      const chunk = chunkOf(p.x, p.z), kind = p.burnt ? 'burnt' : DECOR.test(p.key) ? 'decor' : 'solid';
      const id = `${chunk}:${kind}`;
      let b = batches.get(id);
      if (!b) { b = new PropBatch(lib); batches.set(id, b); }
      const ck = `${id}:${p.key}`, n = counters.get(ck) ?? 0;
      counters.set(ck, n + 1);
      where[i] = chunk; slot[i] = n;
      b.add(p.key, p.x, p.y - 0.05, p.z, p.yaw, p.scale);
    });
    const solidByChunk = new Map<number, Map<string, THREE.InstancedMesh[]>>();
    for (const [id, b] of batches) {
      const [chunkText, kind] = id.split(':'), chunk = Number(chunkText);
      const g = new THREE.Group();
      b.build(g, kind !== 'decor', DECOR, kind === 'burnt' ? m => this.char(m, lib) : undefined);
      this.group.add(g);
      if (kind === 'decor') this.decor.push({ group: g, x: -HALF + ((chunk % side) + 0.5) * CHUNK, z: -HALF + (Math.floor(chunk / side) + 0.5) * CHUNK });
      if (kind === 'solid') {
        const byKey = new Map<string, THREE.InstancedMesh[]>();
        for (const o of g.children) if (o instanceof THREE.InstancedMesh) { const l = byKey.get(o.name) ?? []; l.push(o); byKey.set(o.name, l); }
        solidByChunk.set(chunk, byKey);
      }
    }
    this.trees = map.trunkProps.map(prop => ({ meshes: solidByChunk.get(where[prop])?.get(map.props[prop].key) ?? [], index: slot[prop] }));
  }

  /** A charred copy of a kit material (cached). */
  private char(m: THREE.Material, lib: AssetLibrary): THREE.Material {
    let out = this.burnt.get(m);
    if (!out) {
      const src = m as THREE.MeshStandardMaterial;
      const c = lib.own(new THREE.MeshStandardMaterial({ color: (src.color ?? new THREE.Color(0x666666)).clone().multiplyScalar(0.16).add(new THREE.Color(0.05, 0.04, 0.035)), map: src.map ?? null, roughness: 1, metalness: 0.05 }));
      this.burnt.set(m, c);
      out = c;
    }
    return out;
  }

  /** Decor in chunks whose centre is beyond range is hidden. */
  update(camX: number, camZ: number): void {
    for (const d of this.decor) d.group.visible = Math.hypot(d.x - camX, d.z - camZ) < DECOR_RANGE + CHUNK * 0.5;
  }
}

/** The map's fires: flame and smoke particles near the camera, flickering light from the two closest. */
export class Fires {
  readonly group = new THREE.Group();
  private list: Fire[];
  private flames = new Particles(1800, true);
  private plume = new Particles(1100, false);
  private lights: THREE.PointLight[] = [];
  private acc: Float32Array;
  private time = 0;
  constructor(list: Fire[]) {
    this.list = list;
    this.acc = new Float32Array(list.length);
    for (let i = 0; i < 2; i++) { const l = new THREE.PointLight(0xff8a3a, 0, 22, 1.5); this.lights.push(l); this.group.add(l); }
    this.group.add(this.flames.points, this.plume.points);
  }
  update(dt: number, cx: number, cz: number): void {
    this.time += dt;
    let n0 = -1, n1 = -1, d0 = Infinity, d1 = Infinity;
    for (let i = 0; i < this.list.length; i++) {
      const f = this.list[i], d = Math.hypot(f.x - cx, f.z - cz);
      if (d < d0) { d1 = d0; n1 = n0; d0 = d; n0 = i; } else if (d < d1) { d1 = d; n1 = i; }
      if (d > 170) continue;
      // Flames: a few per frame (fewer far away); a smoke puff every so often.
      this.acc[i] += dt * f.s * (d < 70 ? 70 : 28);
      while (this.acc[i] >= 1) {
        this.acc[i] -= 1;
        const r = 0.5 * f.s, hot = Math.random();
        this.flames.spawn(f.x + (Math.random() - 0.5) * r * 2, f.y + 0.1, f.z + (Math.random() - 0.5) * r * 2, (Math.random() - 0.5) * 0.6, 1.6 + Math.random() * 1.8 * f.s, (Math.random() - 0.5) * 0.6,
          hot < 0.3 ? 0xffe28a : hot < 0.75 ? 0xff8a2a : 0xd8401a, (0.7 + Math.random() * 0.7) * f.s, 0.45 + Math.random() * 0.4, 0.8, 1.2);
        if (Math.random() < 0.09) this.plume.spawn(f.x, f.y + 1.2 * f.s, f.z, 0.5 + Math.random() * 0.6, 1.6 + Math.random(), (Math.random() - 0.5) * 0.6, Math.random() < 0.5 ? 0x2c2724 : 0x3a3430, (2.4 + Math.random() * 2.2) * f.s, 4.5 + Math.random() * 2, 0.15, 0.25);
      }
    }
    [n0, n1].forEach((i, k) => {
      const l = this.lights[k];
      if (i < 0 || (k === 0 ? d0 : d1) > 60) { l.intensity = 0; return; }
      const f = this.list[i];
      l.position.set(f.x, f.y + 1.4 * f.s, f.z);
      l.intensity = (16 + Math.sin(this.time * 17 + k) * 4 + Math.sin(this.time * 31) * 3) * f.s;
    });
    this.flames.update(dt); this.plume.update(dt);
  }
  dispose(): void { this.flames.dispose(); this.plume.dispose(); }
}

