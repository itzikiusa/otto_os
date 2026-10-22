/**
 * Kart Grand Prix renderer: builds the track world and animates it from
 * simulation / snapshot state. The world: a sky dome with sun and drifting
 * clouds (stars at dusk), softened sun shadows, terrain with themed detail,
 * lagoon water (waves, Fresnel sky, glitter, shoreline foam, caustics below)
 * or flowing lava that lights the rock around it, every route's road
 * (render/road.ts: world-space asphalt, grooves, tyre marks, paint, rumble
 * curbs, special surfaces, banners, neon, dash panels), barrier walls and
 * rails, banking, bridges with pillars and abutments, embankments, tunnels
 * with ribs and lamps, ramps, launch pads, half-pipes, drawbridges, the start
 * grid, item boxes, coins, obstacles, fork signage, chevron boards, themed
 * Kenney props and cheering crowds on the grandstands. Placement of anything
 * solid comes from layout.ts (and render/dressing.ts, held to the same rule).
 *
 * Animated: karts, drivers, gliders, boost flames tinted by what fired them,
 * drift-spark glows per tier, shells by kind, lightning strikes, shockwaves,
 * dust / snow / water spray, slipstream streaks, burnouts, fireworks at the
 * finish, a spring-lagged chase camera with FOV kick, speed lines and shake.
 * Under the water surface the camera fogs over blue.
 */
import * as THREE from 'three';
import { AssetLibrary, PropBatch } from '../common/assets';
import { GameView, owned } from '../common/view';
import { Particles } from '../common/particles';
import { approach, clamp, fbm, hashString, lerp, makeNoise, smoothstep, wrapAngle } from '../common/math';
import { checkerTexture, labelTexture, stripeTexture } from '../common/textures';
import {
  coinSpots, CRUSHER_HALF, DRAW_ANGLE, drawLift, F_BRIDGE, F_DRAW, F_GAP, F_RAILS, F_TUNNEL, LAUNCH_HEIGHT, LAUNCH_LENGTH, nearest,
  pipeOn, RAIL_SHOULDER, newNear, obstacleAt, pointAt, rampOn, RAMP_HEIGHT, RAMP_LENGTH, ROLLER_RADIUS, sampleAt, TUNNEL_HEIGHT, type Drawbridge, type Near, type Path, type Track,
} from './tracks';
import {
  abutments, barriers, forkSign, gantries, pillars, propLayout, skirts, terrainGrid, towers, tubeMask, TUBE_PAD, worldHeight, type Barrier, type Gantry, type PropPlacement,
} from './layout';
import { racerDef, RACERS, type RacerId } from './roster';
import { SHELL_KINDS, type KartEvent, type KartState, type ShellKind } from './sim';
import { THEMES, type KartTheme } from './render/themes';
import { RoadKit } from './render/road';
import { lavaMaterial, patchCrowd, patchWorld, skyMaterial, waterMaterial, type SharedUniforms } from './render/atmosphere';
import { barrierTexture, chevronBoardTexture, glowTexture, gridSlotTexture, terrainTexture, type TerrainKind } from './render/surfaces';
import { chevronBoards, extraProps, fireworkSpots, gridSlots, CHEVRON_HALF, CHEVRON_HEIGHT } from './render/dressing';
import { Bolts, Rings, ShellKit, SpeedLines } from './render/fx';
import { chaseFov, Shake, spring, springTo, type Spring } from './render/camera';

const KART_SCALE = 2.0;
/** Drift spark colours per mini-turbo tier (0: still charging). */
const SPARK = [0x9a9a9a, 0x4fc3ff, 0xff9a2e, 0xd36bff];
/** Boost flame tints: dash panel / ramp, mushroom, rocket start, golden mushroom, slipstream. */
const FLAME = { pad: 0xffa030, mushroom: 0xff5a1e, start: 0xffd040, golden: 0xffd84a, draft: 0x9fe6ff };
/** Shell kinds, by the index the `throw` / `explode` events carry. */
const SHELL_TINT: Record<ShellKind, number> = { green: 0x3fe04a, red: 0xff4a3a, blue: 0x4f8fff, bomb: 0xffa040 };
const FIREWORK = [0xff4f6a, 0xffd23a, 0x4fc3ff, 0x7dff6a, 0xd36bff, 0xffffff];
const TERRAIN: Record<string, TerrainKind> = { meadow: 'grass', city: 'pave', canyon: 'sand', frost: 'snow', lagoon: 'grass', volcano: 'rock', dusk: 'pave' };
/** Underwater fog. */
const DEEP = { color: 0x0d5f78, near: 2, far: 120 };
const _near: Near = newNear();
const _pt = [0, 0, 0, 0];
const _ob = [0, 0, 0, 0];
const _cp = [0, 0, 0, 0];
const _v = new THREE.Vector3();
const _look = new THREE.Vector3();
const _m = new THREE.Matrix4();
const _q = new THREE.Quaternion();
const _s = new THREE.Vector3();
const _p = new THREE.Vector3();
const _up = new THREE.Vector3(0, 1, 0);

interface KartVisual {
  racer: RacerId; root: THREE.Group; body: THREE.Group; wheels: THREE.Object3D[]; front: THREE.Object3D[];
  mixer: THREE.AnimationMixer | null; flames: THREE.Mesh[]; cores: THREE.Mesh[]; flameMat: THREE.MeshBasicMaterial;
  exhaust: THREE.Sprite[]; exhaustMat: THREE.SpriteMaterial; sparks: THREE.Sprite[]; sparkMat: THREE.SpriteMaterial;
  bubble: THREE.Mesh; label: THREE.Sprite | null; wing: THREE.Group;
  wheelSpin: number; tilt: number; bounce: number; visualYaw: number; spinAngle: number; pitch: number; wingOpen: number;
  /** What the current boost's flames look like, the last drift tier seen, the shrink from lightning. */
  flameTint: number; lastLevel: number; size: number;
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
  private road!: RoadKit;
  private shared: SharedUniforms = { time: { value: 0 } };
  private cheer = { value: 0.3 };
  private sky!: THREE.ShaderMaterial;
  private karts: KartVisual[] = [];
  private boxes: THREE.Object3D[] = [];
  private bananas = new Map<number, THREE.Object3D>();
  private shells = new Map<number, { kind: ShellKind; object: THREE.Object3D }>();
  private shellKit!: ShellKit;
  private obstacles: ObstacleVisual[] = [];
  private drawbridges: DrawbridgeVisual[] = [];
  private coins: THREE.InstancedMesh | null = null;
  private particles = new Particles(3200, true);
  private smoke = new Particles(900, false);
  private speedLines!: SpeedLines;
  private bolts!: Bolts;
  private rings!: Rings;
  private camPos = { x: spring(), y: spring(), z: spring() };
  private camLook = new THREE.Vector3();
  private camReady = false;
  /** Walls per path index, for keeping the chase camera on the road side of them (built on first use). */
  private camWalls = new Map<number, Barrier[]>();
  private shake = new Shake();
  private flash = 0;
  private fireworks = 0;
  private fireworkAt = 0;
  private spots: number[] = [];
  private underwater = false;
  private time = 0;
  /** Test / capture hook: a fixed camera pose [x, y, z, lookX, lookY, lookZ] overriding the chase camera. */
  pin: number[] | null = null;
  private flameGeo = new THREE.ConeGeometry(0.24, 1.3, 10, 1, true);
  private coreGeo = new THREE.ConeGeometry(0.11, 0.9, 8, 1, true);
  private coreMat = new THREE.MeshBasicMaterial({ color: 0xfff6d8, transparent: true, opacity: 0.95, blending: THREE.AdditiveBlending, depthWrite: false, toneMapped: false });
  private glow = glowTexture();
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
    this.view = new GameView(host, this.theme, 2048, { shadowExtent: 60 });
    this.view.camera.far = 1500;
    // Softened (filtered) sun shadows under the karts and props.
    this.view.sun.shadow.radius = 2.5;
    this.view.setAtmosphere(this.theme, new THREE.Vector3(...this.theme.sunOffset));
    this.lib = new AssetLibrary(signal);
    this.racers = racers;
    this.self = self;
  }

  async load(): Promise<void> {
    // Every seated driver is loaded up front: guests may switch racer during the intro.
    const drivers = RACERS.map(r => r.driver).filter((d): d is string => !!d);
    await Promise.all([this.lib.loadKit('kart-kit.glb'), drivers.length ? this.lib.loadCharacters(drivers) : Promise.resolve()]);
    for (const m of [this.flameGeo, this.coreGeo, this.coreMat, this.glow, this.bubbleGeo, this.bubbleMat, this.wingGeo, this.strutGeo, this.strutMat]) this.lib.own(m);
    this.buildWorld();
    for (const r of this.racers) this.karts.push(this.buildKart(r));
    const scene = this.view.scene;
    scene.add(this.particles.points, this.smoke.points);
    // The camera carries the speed lines, so it joins the scene graph.
    scene.add(this.view.camera);
    this.speedLines = new SpeedLines(this.lib, this.view.camera);
    this.bolts = new Bolts(this.lib, scene);
    this.rings = new Rings(this.lib, scene);
    this.shellKit = new ShellKit(this.lib);
    this.spots = fireworkSpots(this.track);
  }

  // ── World ────────────────────────────────────────────────────────────────
  private buildWorld(): void {
    const { track, theme } = this;
    const scene = this.view.scene;
    this.sky = skyMaterial({ top: theme.skyTop, horizon: theme.skyHorizon, ground: theme.skyGround, sun: theme.sun, sunDir: new THREE.Vector3(...theme.sunOffset), cloud: theme.cloud, cover: theme.cover, stars: theme.stars }, this.shared);
    this.view.setSky(this.sky);
    const noise = makeNoise(hashString(track.def.id) ^ 0x51ed);
    const { x0, z0, nx, nz, step } = terrainGrid(track);
    const positions = new Float32Array(nx * nz * 3), colors = new Float32Array(nx * nz * 3), uvs = new Float32Array(nx * nz * 2);
    const palette = theme.ground.map(c => new THREE.Color(c));
    const edge = new THREE.Color(theme.edge), bed = new THREE.Color(theme.bed);
    const tmp = new THREE.Color();
    const liquid = track.liquid;
    // Water depth over the land, baked for the water shader's tint and foam (8 m full scale).
    const depth = liquid !== null && !track.lava ? new Uint8Array(nx * nz) : null;
    for (let iz = 0; iz < nz; iz++) for (let ix = 0; ix < nx; ix++) {
      const k = iz * nx + ix, x = x0 + ix * step, z = z0 + iz * step;
      // Terrain plus far-field shaping (mountains, sea) beyond every route (see layout.worldHeight).
      const y = worldHeight(track, x, z);
      // Main-loop distance drives the colouring.
      let bd = Infinity, bi = 0;
      for (let i = 0; i < track.count; i += 6) { const d = (track.px[i] - x) ** 2 + (track.pz[i] - z) ** 2; if (d < bd) { bd = d; bi = i; } }
      const near = nearest(track, x, z, bi, _near, 8);
      const off = Math.min(Math.abs(near.lateral) - track.halfWidth, Math.sqrt(bd) - track.halfWidth);
      positions[k * 3] = x; positions[k * 3 + 1] = y; positions[k * 3 + 2] = z;
      uvs[k * 2] = x / 6; uvs[k * 2 + 1] = z / 6;
      if (depth && liquid !== null) depth[k] = Math.round(clamp((liquid - y) / 8, 0, 1) * 255);
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
    const groundMat = new THREE.MeshLambertMaterial({ vertexColors: true, map: this.lib.own(terrainTexture(TERRAIN[track.def.theme] ?? 'grass')) });
    // Caustics on the lake / lagoon bed, or the lava's glow on the rock around it.
    if (liquid !== null) patchWorld(groundMat, track.lava ? { glow: { level: liquid, color: 0xff5a1a } } : { caustics: liquid }, this.shared);
    const terrain = owned(new THREE.Mesh(geo, groundMat));
    terrain.receiveShadow = true;
    scene.add(terrain);
    if (liquid !== null) {
      let mat: THREE.Material;
      if (track.lava) mat = this.lib.own(lavaMaterial(theme.liquid, this.shared));
      else {
        const tex = this.lib.own(new THREE.DataTexture(depth!, nx, nz, THREE.RedFormat, THREE.UnsignedByteType));
        tex.magFilter = tex.minFilter = THREE.LinearFilter;
        tex.needsUpdate = true;
        // Texel centres sit on the grid vertices.
        const rect = new THREE.Vector4(x0 - step / 2, z0 - step / 2, nx * step, nz * step);
        mat = this.lib.own(waterMaterial({ deep: theme.liquid, shallow: theme.shallow, top: theme.skyTop, horizon: theme.skyHorizon, sun: theme.sun, sunDir: new THREE.Vector3(...theme.sunOffset) }, tex, rect, this.shared));
      }
      const surface = owned(new THREE.Mesh(new THREE.PlaneGeometry(6000, 6000), mat));
      surface.rotation.x = -Math.PI / 2; surface.position.y = liquid;
      surface.renderOrder = 2;
      scene.add(surface);
    }
    this.road = new RoadKit(this.lib, track, theme, this.shared, scene);
    for (const p of track.paths) this.buildRoad(p);
    for (const g of gantries(track)) this.gantry(g);
    this.buildCoins();
    this.buildStart();
    this.buildObstacles();
    this.buildProps();
    this.buildChevrons();
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

  private buildRoad(path: Path): void {
    const { track, theme, view } = this;
    const scene = view.scene;
    const hw = path.halfWidth;
    // Tarmac, grooves, tyre marks, paint, curbs, off-road and ice, banners, neon and dash panels (render/road.ts).
    this.road.build(path);
    const lift = path.index === 0 ? 0.06 : 0.075;
    // Barriers on walled roads, rails on bridges and along half-pipe copings
    // (layout.barriers: matching the collision limit, none over gaps or in tunnels, open at forks).
    const [wa, wb] = theme.wall;
    const wallMat = new THREE.MeshLambertMaterial({ map: this.lib.own(barrierTexture(wa, wb)) });
    if (theme.neon !== null) { wallMat.emissive.setHex(theme.neon); wallMat.emissiveIntensity = 0.08; }
    for (const b of barriers(track, path)) if (b.kind !== 'deck') scene.add(this.wall(path, b.lateral, b.side, wallMat, i => b.mask[i] === 1, b.top, b.bottom, b.thick));
    this.buildBridges(path);
    this.buildEmbankments(path);
    this.buildTunnels(path);
    this.buildHalfpipes(path, this.road.tarmac(3, 2));
    this.buildDrawbridges(path, this.road.tarmac(2.3, 2), wallMat);
    // Kicker ramps: striped deck over the rising road + lip face.
    const stripes = new THREE.MeshLambertMaterial({ map: this.lib.own(stripeTexture()) });
    for (const at of path.features.ramps) {
      const s0 = at * path.length - RAMP_LENGTH;
      scene.add(this.strip(path, -hw, hw, lift + 0.04, 2, stripes, i => { const d = path.dist[i] - s0; return d >= -2 && d <= RAMP_LENGTH + 2; }));
      this.lip(path, at, RAMP_HEIGHT, 0x333333);
    }
    // Launch ramps (and glider pads): long striped ramp, chevrons, a gantry with a banner.
    const glideStripes = new THREE.MeshLambertMaterial({ map: this.lib.own(stripeTexture('#38b6ff', '#ffffff')) });
    for (const l of path.features.launches) {
      const s0 = l.at * path.length - LAUNCH_LENGTH;
      scene.add(this.strip(path, -hw, hw, lift + 0.04, 3, l.glide ? glideStripes : stripes, i => { const d = path.dist[i] - s0; return d >= -2 && d <= LAUNCH_LENGTH + 1; }));
      this.lip(path, l.at, LAUNCH_HEIGHT, l.glide ? 0x1f6fb2 : 0x333333);
    }
    // Solid sides under kicker and launch ramps (open where another route leaves beside them).
    for (const sk of skirts(track, path)) if (sk.kind === 'ramp') {
      const mat = new THREE.MeshLambertMaterial({ color: sk.glide ? 0x1f6fb2 : 0x333333, side: THREE.DoubleSide });
      scene.add(this.skirt(path, sk.lateral, i => sk.mask[i] === 1, i => sk.top[i], i => sk.bottom[i], mat));
    }
    // A sign in the gore where each branch splits from the main road, facing the oncoming racers.
    const fork = forkSign(track, path);
    if (fork) {
      const tex = this.lib.own(labelTexture(fork.text, fork.color));
      const sign = owned(new THREE.Mesh(new THREE.PlaneGeometry(9, 2.2), new THREE.MeshBasicMaterial({ map: tex, transparent: true, side: THREE.DoubleSide })));
      sign.position.set(fork.x, fork.y + 3.6, fork.z);
      sign.rotation.y = fork.yaw + Math.PI;
      const post = owned(new THREE.Mesh(new THREE.BoxGeometry(0.4, 3.6, 0.4), new THREE.MeshLambertMaterial({ color: 0x333333 })));
      post.position.set(fork.x, fork.y + 1.8, fork.z);
      scene.add(sign, post);
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
    const { track } = this;
    const rock = track.def.theme === 'canyon' || track.def.theme === 'volcano';
    const mat = new THREE.MeshLambertMaterial({ color: rock ? this.theme.obstacle : 0x8f8f96, side: THREE.DoubleSide });
    // Embankments: drop > 0 on solid road (not a bridge deck, a leaf or a gap), open at fork mouths.
    for (const sk of skirts(track, path)) if (sk.kind === 'embankment') this.view.scene.add(this.skirt(path, sk.lateral, i => sk.mask[i] === 1, i => sk.top[i], i => sk.bottom[i], mat));
    // Abutments: a face across the road wherever a deck or a gap begins or ends (stopping short of any
    // road below or beside), all merged into one mesh per route.
    const quads = abutments(track, path);
    if (!quads.length) return;
    const pos: number[] = [], idx: number[] = [];
    for (let k = 0; k < quads.length; k += 6) {
      const [ax, az, bx, bz, floor, top] = quads.slice(k, k + 6);
      const m = pos.length / 3;
      pos.push(ax, floor, az, bx, floor, bz, ax, top, az, bx, top, bz);
      idx.push(m, m + 1, m + 2, m + 1, m + 3, m + 2);
    }
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
    geo.setIndex(idx);
    geo.computeVertexNormals();
    const faces = owned(new THREE.Mesh(geo, mat));
    faces.castShadow = true;
    this.view.scene.add(faces);
  }

  /** Overhead gantry across the road carrying a banner (placed by layout.gantries: posts never on another route). */
  private gantry(g: Gantry): void {
    const scene = this.view.scene, hw = g.width / 1.4;
    const label = this.lib.own(labelTexture(g.text, g.color));
    const banner = owned(new THREE.Mesh(new THREE.PlaneGeometry(g.width, hw * 0.35), new THREE.MeshBasicMaterial({ map: label, transparent: true, side: THREE.DoubleSide })));
    banner.position.set(g.x, g.y + (g.inside ? TUNNEL_HEIGHT - 0.6 - hw * 0.175 : 6.5), g.z);
    banner.rotation.y = g.yaw + Math.PI;
    scene.add(banner);
    for (const [x, y, z] of g.posts) {
      const post = owned(new THREE.Mesh(new THREE.BoxGeometry(0.5, 7.2, 0.5), new THREE.MeshLambertMaterial({ color: 0x333333 })));
      post.position.set(x, y + 3.6, z);
      scene.add(post);
    }
  }

  /** Half-pipes: a U-shaped deck (walls rising to PIPE_HEIGHT, flat coping out to the rail) with skirts, an end lip face and rails on top. */
  private buildHalfpipes(path: Path, deckMat: THREE.Material): void {
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
    // Outer skirts down to the ground (the coping rails are barriers, see buildRoad).
    for (const sk of skirts(this.track, path)) if (sk.kind === 'pipe') scene.add(this.skirt(path, sk.lateral, i => sk.mask[i] === 1, i => sk.top[i], i => sk.bottom[i], skirtMat));
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
      }
      this.drawbridges.push({ def: d, leaves });
    }
    // Bascule towers either side of each hinge.
    for (const [x, y, z] of towers(path)) {
      const t = owned(new THREE.Mesh(new THREE.BoxGeometry(2.4, 9, 2.4), tower));
      t.position.set(x, y + 3, z);
      t.castShadow = true;
      scene.add(t);
    }
  }

  /** Every coin of the track as one instanced mesh (gold discs standing on edge), posed each frame. */
  private buildCoins(): void {
    const count = coinSpots(this.track).length / 3;
    if (!count) return;
    const geo = new THREE.CylinderGeometry(0.75, 0.75, 0.16, 18);
    geo.rotateX(Math.PI / 2);
    this.coins = owned(new THREE.InstancedMesh(geo, new THREE.MeshStandardMaterial({ color: 0xffc928, emissive: 0x6a4400, metalness: 0.35, roughness: 0.3 }), count));
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
    for (const b of barriers(this.track, path)) if (b.kind === 'deck') scene.add(this.wall(path, b.lateral, b.side, concrete, i => b.mask[i] === 1, b.top, b.bottom, b.thick));
    // Pillars just outside the deck, down to the land (never onto a road below or beside).
    const spots = pillars(this.track, path);
    if (!spots.length) return;
    const columns = new THREE.InstancedMesh(new THREE.CylinderGeometry(0.7, 0.9, 1, 10), concrete, spots.length / 4);
    for (let k = 0; k < spots.length; k += 4) {
      _p.set(spots[k], spots[k + 1] + spots[k + 3] / 2, spots[k + 2]);
      _s.set(1, spots[k + 3], 1);
      columns.setMatrixAt(k / 4, _m.compose(_p, _q.identity(), _s));
    }
    columns.instanceMatrix.needsUpdate = true;
    columns.computeBoundingSphere();
    columns.castShadow = true; columns.receiveShadow = true;
    scene.add(owned(columns));
  }
  /** Arched tubes over tunnel stretches: rock with timber ribs and ceiling lights, or glass with steel ribs under the water. */
  private buildTunnels(path: Path): void {
    const scene = this.view.scene;
    for (const glass of [false, true]) {
      const tube = tubeMask(path, glass);
      const inTube = (i: number) => tube[i] === 1;
      const pos: number[] = [], idx: number[] = [];
      const M = 12, R = path.halfWidth + TUBE_PAD;
      let open = false;
      const lights: number[] = [], ribs: number[] = [];
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
        const yaw = Math.atan2(path.tx[i], path.tz[i]);
        if (i % 5 === 0) lights.push(path.px[i], path.py[i] + TUNNEL_HEIGHT - 0.7, path.pz[i], yaw);
        if (i % 3 === 0) ribs.push(path.px[i], path.py[i] - 0.4, path.pz[i], yaw);
      }
      if (!idx.length) continue;
      const geo = new THREE.BufferGeometry();
      geo.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
      geo.setIndex(idx);
      geo.computeVertexNormals();
      const mat = glass
        ? new THREE.MeshStandardMaterial({ color: 0xa8ecff, transparent: true, opacity: 0.2, roughness: 0.05, metalness: 0.2, side: THREE.DoubleSide, depthWrite: false })
        : new THREE.MeshLambertMaterial({ color: this.theme.obstacle, map: this.lib.own(terrainTexture('rock')), side: THREE.DoubleSide });
      const mesh = owned(new THREE.Mesh(geo, mat));
      mesh.castShadow = !glass; mesh.receiveShadow = true;
      if (glass) mesh.renderOrder = 3;
      scene.add(mesh);
      const lamp = new THREE.InstancedMesh(new THREE.BoxGeometry(2.4, 0.15, 0.5), new THREE.MeshBasicMaterial({ color: glass ? 0x9ff6ff : 0xffe7a8, toneMapped: false }), lights.length / 4);
      for (let k = 0; k < lights.length; k += 4) {
        _p.set(lights[k], lights[k + 1], lights[k + 2]);
        _q.setFromAxisAngle(_up, lights[k + 3]);
        lamp.setMatrixAt(k / 4, _m.compose(_p, _q, _s.set(1, 1, 1)));
      }
      lamp.instanceMatrix.needsUpdate = true;
      lamp.computeBoundingSphere();
      scene.add(owned(lamp));
      // Ribs: half rings hugging the tube (an ellipse: R across, TUNNEL_HEIGHT up), every ~6 m.
      const ribGeo = new THREE.TorusGeometry(R - 0.12, glass ? 0.1 : 0.16, 5, 18, Math.PI);
      const rib = new THREE.InstancedMesh(ribGeo, new THREE.MeshLambertMaterial({ color: glass ? 0xd8e2ea : 0x6a4a30 }), ribs.length / 4);
      for (let k = 0; k < ribs.length; k += 4) {
        _p.set(ribs[k], ribs[k + 1], ribs[k + 2]);
        _q.setFromAxisAngle(_up, ribs[k + 3]);
        rib.setMatrixAt(k / 4, _m.compose(_p, _q, _s.set(1, TUNNEL_HEIGHT / (R - 0.12), 1)));
      }
      rib.instanceMatrix.needsUpdate = true;
      rib.computeBoundingSphere();
      rib.castShadow = !glass; rib.receiveShadow = true;
      scene.add(owned(rib));
    }
  }

  /** Start / finish: a two-row checker band across the road and the painted grid boxes behind it. */
  private buildStart(): void {
    const { track, view } = this;
    const checker = this.lib.own(checkerTexture());
    const band = owned(new THREE.Mesh(new THREE.PlaneGeometry(track.halfWidth * 2, 3.2), new THREE.MeshLambertMaterial({ map: checker, polygonOffset: true, polygonOffsetFactor: -2, polygonOffsetUnits: -3 })));
    pointAt(track, 0, 0, _pt);
    band.position.set(_pt[0], _pt[1] + 0.08, _pt[2]);
    band.rotation.set(-Math.PI / 2, _pt[3], 0, 'YXZ');
    band.receiveShadow = true;
    view.scene.add(band);
    const slots = gridSlots(track);
    const geo = new THREE.PlaneGeometry(track.halfWidth * 0.5, 3.4);
    geo.rotateX(-Math.PI / 2);
    // Box drawn ahead of each seat: the kart's nose sits in the open bracket.
    geo.translate(0, 0, 0.6);
    const mat = new THREE.MeshLambertMaterial({ map: this.lib.own(gridSlotTexture()), transparent: true, depthWrite: false, polygonOffset: true, polygonOffsetFactor: -2, polygonOffsetUnits: -3 });
    const grid = owned(new THREE.InstancedMesh(geo, mat, slots.length / 4));
    for (let k = 0; k < slots.length; k += 4) {
      _p.set(slots[k], slots[k + 1] + 0.075, slots[k + 2]);
      _q.setFromAxisAngle(_up, slots[k + 3]);
      grid.setMatrixAt(k / 4, _m.compose(_p, _q, _s.set(1, 1, 1)));
    }
    grid.instanceMatrix.needsUpdate = true;
    grid.computeBoundingSphere();
    grid.receiveShadow = true; grid.renderOrder = 0.4;
    view.scene.add(grid);
  }

  private buildObstacles(): void {
    const { track, theme } = this;
    const rollerMat = new THREE.MeshLambertMaterial({ color: theme.obstacle, map: this.lib.own(track.def.theme === 'dusk' ? stripeTexture('#ffb000', '#2a2a2a') : terrainTexture('rock')) });
    const crusherMat = new THREE.MeshLambertMaterial({ color: 0x6d6a72, map: this.lib.own(terrainTexture('rock')) });
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

  /**
   * Themed props (layout.propLayout) plus the dressing extras (floodlights,
   * corner grandstands: render/dressing.extraProps), all clear of every
   * route's road; then a cheering crowd on every grandstand.
   */
  private buildProps(): void {
    const batch = new PropBatch(this.lib);
    const all: PropPlacement[] = [...propLayout(this.track), ...extraProps(this.track)];
    for (const p of all) batch.add(p.key, p.x, p.y, p.z, p.yaw, p.scale, p.sy);
    const group = new THREE.Group();
    batch.build(group, true);
    this.view.scene.add(group);
    this.buildCrowds(all.filter(p => /grandStand/.test(p.key)));
  }

  /**
   * Spectators on the grandstands: each stand model's seating is found once
   * by casting rays down onto it (the lowest upward-facing step under a roof),
   * then every placed stand fills its seats with bobbing, waving fans in two
   * instanced draws (shirts in the theme's colours, heads).
   */
  private buildCrowds(stands: PropPlacement[]): void {
    if (!stands.length) return;
    const seatsByKey = new Map<string, number[]>();
    const ray = new THREE.Raycaster();
    const down = new THREE.Vector3(0, -1, 0), n = new THREE.Vector3();
    for (const key of new Set(stands.map(s => s.key))) {
      const model = this.lib.model(key);
      const meshes = model.parts.map(p => { const m = new THREE.Mesh(p.geometry, p.material); m.applyMatrix4(p.matrix); m.updateMatrixWorld(true); return m; });
      const box = model.box, H = box.max.y - box.min.y, seats: number[] = [];
      const G = 16;
      for (let a = 0; a < G; a++) for (let b = 0; b < G; b++) {
        const x = lerp(box.min.x, box.max.x, (a + 0.5) / G), z = lerp(box.min.z, box.max.z, (b + 0.5) / G);
        ray.set(_v.set(x, box.max.y + 1, z), down);
        const hits = ray.intersectObjects(meshes, false).filter(h => h.face && n.copy(h.face.normal).transformDirection(h.object.matrixWorld).y > 0.8 && h.point.y > box.min.y + H * 0.08);
        if (!hits.length) continue;
        // Under a roof take the step below it; the step needs head room above.
        const seat = hits[hits.length - 1], above = hits.length > 1 ? hits[hits.length - 2].point.y : Infinity;
        if (above - seat.point.y < H * 0.18) continue;
        seats.push(x, seat.point.y, z);
      }
      seatsByKey.set(key, seats);
    }
    const rng = makeNoise(hashString(this.track.def.id) ^ 0xc40d);
    const spots: number[] = [];
    for (const s of stands) {
      const seats = seatsByKey.get(s.key) ?? [];
      const box = this.lib.model(s.key).box;
      const cx = (box.min.x + box.max.x) / 2, cz = (box.min.z + box.max.z) / 2, c = Math.cos(s.yaw), sn = Math.sin(s.yaw);
      for (let k = 0; k < seats.length; k += 3) {
        // Skip a few seats so the stands are full but not regimented.
        if (rng(seats[k] * 13 + s.x, seats[k + 2] * 13 + s.z) > 0.55) continue;
        const lx = (seats[k] - cx) * s.scale, lz = (seats[k + 2] - cz) * s.scale;
        spots.push(s.x + lx * c + lz * sn, s.y + (seats[k + 1] - box.min.y) * s.sy, s.z - lx * sn + lz * c);
      }
    }
    if (!spots.length) return;
    const count = spots.length / 3;
    const bodyGeo = this.lib.own(new THREE.CylinderGeometry(0.26, 0.34, 0.95, 6));
    bodyGeo.translate(0, 0.48, 0);
    const headGeo = this.lib.own(new THREE.SphereGeometry(0.21, 8, 6));
    headGeo.translate(0, 1.16, 0);
    const bodyMat = this.lib.own(new THREE.MeshLambertMaterial({ color: 0xffffff }));
    const headMat = this.lib.own(new THREE.MeshLambertMaterial({ color: 0xffffff }));
    patchCrowd(bodyMat, this.shared, this.cheer); patchCrowd(headMat, this.shared, this.cheer);
    const bodies = new THREE.InstancedMesh(bodyGeo, bodyMat, count), heads = new THREE.InstancedMesh(headGeo, headMat, count);
    const col = new THREE.Color(), skins = [0xf1c8a0, 0xd9a273, 0xa86d43, 0x6e4426, 0xf5d7bd];
    for (let i = 0; i < count; i++) {
      _p.set(spots[i * 3], spots[i * 3 + 1], spots[i * 3 + 2]);
      _s.setScalar(0.9 + ((i * 7919) % 13) / 60);
      _m.compose(_p, _q.identity(), _s);
      bodies.setMatrixAt(i, _m); heads.setMatrixAt(i, _m);
      bodies.setColorAt(i, col.setHex(this.theme.crowd[(i * 31) % this.theme.crowd.length]));
      heads.setColorAt(i, col.setHex(skins[(i * 17) % skins.length]));
    }
    for (const im of [bodies, heads]) { im.instanceMatrix.needsUpdate = true; if (im.instanceColor) im.instanceColor.needsUpdate = true; im.computeBoundingSphere(); im.castShadow = false; im.receiveShadow = true; }
    this.view.scene.add(owned(bodies), owned(heads));
  }

  /** Chevron boards round the outside of the sharp bends (render/dressing.chevronBoards): instanced posts and faces. */
  private buildChevrons(): void {
    const boards = chevronBoards(this.track);
    if (!boards.length) return;
    const faceGeo = this.lib.own(new THREE.PlaneGeometry(CHEVRON_HALF * 2, 1.5));
    faceGeo.translate(0, CHEVRON_HEIGHT - 0.75, 0);
    const postGeo = this.lib.own(new THREE.BoxGeometry(0.16, CHEVRON_HEIGHT - 0.4, 0.16));
    postGeo.translate(0, (CHEVRON_HEIGHT - 0.4) / 2, 0);
    const faces = new THREE.InstancedMesh(faceGeo, this.lib.own(new THREE.MeshLambertMaterial({ map: this.lib.own(chevronBoardTexture()), side: THREE.DoubleSide })), boards.length);
    const posts = new THREE.InstancedMesh(postGeo, this.lib.own(new THREE.MeshLambertMaterial({ color: 0x3a3a3e })), boards.length * 2);
    boards.forEach((b, k) => {
      _q.setFromAxisAngle(_up, b.yaw);
      // Mirrored for right-hand bends: the arrows always point the way the road turns.
      faces.setMatrixAt(k, _m.compose(_p.set(b.x, b.y, b.z), _q, _s.set(-b.dir, 1, 1)));
      for (const side of [-1, 1]) {
        _v.set(side * (CHEVRON_HALF - 0.3), 0, -0.05).applyQuaternion(_q);
        posts.setMatrixAt(k * 2 + (side > 0 ? 1 : 0), _m.compose(_p.set(b.x + _v.x, b.y, b.z + _v.z), _q, _s.set(1, 1, 1)));
      }
    });
    for (const im of [faces, posts]) { im.instanceMatrix.needsUpdate = true; im.computeBoundingSphere(); im.castShadow = true; im.receiveShadow = true; }
    this.view.scene.add(faces, posts);
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
    // Boost: a tinted outer flame and a white-hot core per exhaust, with a glow at the pipe.
    const flameMat = this.lib.own(new THREE.MeshBasicMaterial({ color: FLAME.pad, transparent: true, opacity: 0.85, blending: THREE.AdditiveBlending, depthWrite: false, toneMapped: false }));
    const exhaustMat = this.lib.own(new THREE.SpriteMaterial({ map: this.glow, color: FLAME.pad, blending: THREE.AdditiveBlending, depthWrite: false, toneMapped: false }));
    const flames: THREE.Mesh[] = [], cores: THREE.Mesh[] = [], exhaust: THREE.Sprite[] = [];
    for (const x of [-0.18, 0.18]) {
      const f = new THREE.Mesh(this.flameGeo, flameMat);
      f.rotation.x = -Math.PI / 2; f.position.set(x * KART_SCALE, 0.55, -1.3); f.visible = false;
      const c = new THREE.Mesh(this.coreGeo, this.coreMat);
      c.rotation.x = -Math.PI / 2; c.position.set(x * KART_SCALE, 0.55, -1.2); c.visible = false;
      const g = new THREE.Sprite(exhaustMat);
      g.position.set(x * KART_SCALE, 0.58, -1.05); g.scale.setScalar(1.2); g.visible = false;
      root.add(f, c, g);
      flames.push(f); cores.push(c); exhaust.push(g);
    }
    // Drift: a spark glow at each rear wheel in the tier's colour.
    const sparkMat = this.lib.own(new THREE.SpriteMaterial({ map: this.glow, color: SPARK[1], blending: THREE.AdditiveBlending, depthWrite: false, toneMapped: false }));
    const sparks = [-1, 1].map(side => {
      const s = new THREE.Sprite(sparkMat);
      s.position.set(side * 0.95, 0.3, -0.95); s.visible = false;
      root.add(s);
      return s;
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
    return {
      racer: r.racer, root, body, wheels, front, mixer, flames, cores, flameMat, exhaust, exhaustMat, sparks, sparkMat, bubble, label, wing,
      wheelSpin: 0, tilt: 0, bounce: 0, visualYaw: 0, spinAngle: 0, pitch: 0, wingOpen: 0, flameTint: FLAME.pad, lastLevel: 0, size: 1,
    };
  }

  // ── Frame ────────────────────────────────────────────────────────────────
  /** Distance from the camera's racer, for shake and nearby-only effects. */
  private nearSelf(x: number, z: number): number {
    const k = this.karts[this.self];
    return k ? Math.hypot(x - k.root.position.x, z - k.root.position.z) : Infinity;
  }
  effect(e: KartEvent): void {
    const p = this.particles, own = e.racer === this.self, kart = this.karts[e.racer];
    switch (e.type) {
      case 'box': p.burst(e.x, e.y + 1.2, e.z, 30, 0xc58cff, 9, 0.7, 0.6); p.burst(e.x, e.y + 1.2, e.z, 12, 0xffffff, 6, 0.5, 0.4); break;
      case 'explode': {
        const kind = SHELL_KINDS[e.value ?? 3] ?? 'bomb';
        if (kind === 'bomb' || kind === 'blue') {
          // A fireball, a smoke column, a shockwave and a jolt for anyone close.
          p.burst(e.x, e.y + 0.8, e.z, 90, 0xffa040, 18, 1.5, 0.8);
          p.burst(e.x, e.y + 1.2, e.z, 40, kind === 'blue' ? 0x7fb0ff : 0xfff0a0, 10, 1.1, 0.5);
          for (let i = 0; i < 22; i++) this.smoke.spawn(e.x, e.y + 1, e.z, (Math.random() - 0.5) * 7, 3 + Math.random() * 4, (Math.random() - 0.5) * 7, 0x4a4a4a, 3.4, 1.6, 0, 1.4);
          this.rings.spawn(e.x, e.y, e.z, kind === 'blue' ? 0x7fb0ff : 0xffb060, 9, 0.55);
          this.shake.add(clamp(0.7 - this.nearSelf(e.x, e.z) / 30, 0, 0.7));
        } else {
          // A shell shatters: shards in its colour and white.
          p.burst(e.x, e.y + 0.8, e.z, 36, SHELL_TINT[kind], 11, 0.8, 0.6, -14);
          p.burst(e.x, e.y + 0.8, e.z, 16, 0xffffff, 8, 0.6, 0.4, -14);
          this.rings.spawn(e.x, e.y, e.z, SHELL_TINT[kind], 3.5, 0.35);
          this.shake.add(clamp(0.35 - this.nearSelf(e.x, e.z) / 30, 0, 0.35));
        }
        break;
      }
      case 'spin': p.burst(e.x, e.y + 1.6, e.z, 18, 0xffff88, 5, 0.6, 0.8, 0); if (own) this.shake.add(0.25); break;
      case 'bump': p.burst(e.x, e.y + 0.6, e.z, 10, 0xffd080, 6, 0.4, 0.3); if (own) this.shake.add(0.15); break;
      case 'throw': p.burst(e.x, e.y + 1, e.z, 12, SHELL_TINT[SHELL_KINDS[e.value ?? 0] ?? 'green'], 5, 0.5, 0.35, 0); break;
      case 'boost': {
        // value 0 ramp / pad, 1 mushroom, 2 perfect start, 3 golden.
        const tint = e.value === 1 ? FLAME.mushroom : e.value === 2 ? FLAME.start : e.value === 3 ? FLAME.golden : FLAME.pad;
        if (kart) kart.flameTint = tint;
        p.burst(e.x, e.y + 0.6, e.z, e.value ? 30 : 18, tint, 8, 0.7, 0.4);
        this.rings.spawn(e.x, e.y, e.z, tint, e.value ? 4 : 2.5, 0.35);
        if (own) this.shake.add(e.value ? 0.18 : 0.08);
        break;
      }
      case 'turbo':
        if (kart) kart.flameTint = SPARK[e.value ?? 1];
        p.burst(e.x, e.y + 0.6, e.z, 26 + (e.value ?? 1) * 8, SPARK[e.value ?? 1], 8, 0.65, 0.45);
        this.rings.spawn(e.x, e.y, e.z, SPARK[e.value ?? 1], 2 + (e.value ?? 1), 0.3);
        break;
      case 'draft':
        if (kart) kart.flameTint = FLAME.draft;
        p.burst(e.x, e.y + 1, e.z, 24, 0xe8fbff, 7, 0.45, 0.35, 0);
        break;
      case 'burnout': for (let i = 0; i < 16; i++) this.smoke.spawn(e.x + (Math.random() - 0.5) * 2, e.y + 0.4, e.z + (Math.random() - 0.5) * 2, (Math.random() - 0.5) * 2, 1.5 + Math.random(), (Math.random() - 0.5) * 2, 0x3a3a3a, 2.4, 1.2, 0, 1.5); break;
      case 'star': p.burst(e.x, e.y + 1, e.z, 50, 0xffe066, 10, 0.8, 0.8); p.burst(e.x, e.y + 1, e.z, 20, 0xffffff, 8, 0.6, 0.6); break;
      // Lightning: the user's own bolt flashes the sky (no burst over them: it never strikes its user);
      // each opponent struck gets a `zap` (below).
      case 'bolt': this.flash = 1; this.rings.spawn(e.x, e.y, e.z, 0x9ad7ff, 5, 0.45); break;
      case 'zap':
        this.bolts.strike(e.x, e.y, e.z);
        p.burst(e.x, e.y + 1, e.z, 40, 0x9ad7ff, 10, 0.8, 0.5);
        this.rings.spawn(e.x, e.y, e.z, 0xbfe8ff, 4, 0.4);
        if (own) this.shake.add(0.5);
        break;
      case 'respawn': p.burst(e.x, e.y + 1, e.z, 30, 0xffffff, 6, 1, 0.8); this.rings.spawn(e.x, e.y, e.z, 0xffffff, 3, 0.4); break;
      case 'land':
        for (let i = 0; i < 12; i++) this.smoke.spawn(e.x, e.y + 0.3, e.z, (Math.random() - 0.5) * 6, 1, (Math.random() - 0.5) * 6, this.dustColor(), 1.8, 0.7, 0, 2);
        if (own) this.shake.add(0.22);
        break;
      case 'glide': p.burst(e.x, e.y + 2, e.z, 30, 0x9fe6ff, 7, 0.7, 0.6, 0); break;
      case 'crush': for (let i = 0; i < 16; i++) this.smoke.spawn(e.x, e.y + 0.4, e.z, (Math.random() - 0.5) * 9, 1.5, (Math.random() - 0.5) * 9, 0x8a8a8a, 2.2, 0.9, 0, 2); if (own) this.shake.add(0.6); break;
      case 'splash': {
        // value 1: diving under on purpose; 0: fell in the water; 2: lava.
        const y = this.track.liquid ?? e.y, lava = e.value === 2;
        p.burst(e.x, y + 0.3, e.z, e.value === 1 ? 30 : 60, lava ? 0xff7a2a : 0xe8fbff, e.value === 1 ? 6 : 10, lava ? 0.9 : 1.1, 0.9, -12);
        this.rings.spawn(e.x, y, e.z, lava ? 0xff7a2a : 0xffffff, e.value === 1 ? 3 : 5, 0.6);
        if (lava) for (let i = 0; i < 14; i++) this.smoke.spawn(e.x, y + 0.5, e.z, (Math.random() - 0.5) * 3, 3 + Math.random() * 2, (Math.random() - 0.5) * 3, 0x3a3030, 3, 1.6, 0, 1);
        break;
      }
      case 'finish': this.fireworks = Math.max(this.fireworks, own ? 7 : 3.5); break;
      default: break;
    }
  }
  private dustColor(): number {
    const k = this.theme.offroad;
    return k === 'snow' ? 0xffffff : k === 'cinder' ? 0x4a3a36 : k === 'mud' ? 0xb0a080 : this.theme.bed;
  }

  frame(state: KartState, dt: number, lookBack: boolean): void {
    this.time += dt;
    this.shared.time.value = this.time;
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
    this.syncBananas(state);
    this.syncShells(state);
    this.road.dash.offset.y = (-this.time * 1.4) % 1;
    this.updateFireworks(dt);
    // The crowd cheers as the race passes the grandstands and goes wild at the finish.
    const d = Math.hypot(self.x - this.track.px[0], self.z - this.track.pz[0]);
    this.cheer.value = state.phase === 'done' ? 1 : approach(this.cheer.value, 0.25 + 0.75 * (1 - smoothstep(30, 140, d)), 2, dt);
    // A lightning flash lights the whole sky for a moment.
    this.flash = Math.max(0, this.flash - dt * 3.5);
    this.view.hemi.intensity = this.theme.ambient * (1 + this.flash * 1.6);
    this.particles.update(dt);
    this.smoke.update(dt);
    this.bolts.update(dt, this.view.camera);
    this.rings.update(dt);
    this.updateCamera(state, self, dt, lookBack);
    this.speedLines.update(dt, state.phase === 'race' ? clamp((self.boost > 0 ? 0.65 : 0) + smoothstep(26, 40, self.speed) * 0.5, 0, 1) : 0, Math.max(0, self.speed));
    this.updateFog();
    if (this.pin) this.view.focusShadow(this.pin[3], this.pin[4], this.pin[5]); else this.view.focusShadow(self.x, self.y, self.z);
    this.view.render();
  }

  /** Fireworks over the finish line after a racer crosses it: a burst every ~0.3 s from the launch spots. */
  private updateFireworks(dt: number): void {
    if (this.fireworks <= 0) return;
    this.fireworks -= dt;
    this.fireworkAt -= dt;
    if (this.fireworkAt > 0 || !this.spots.length) return;
    this.fireworkAt = 0.22 + Math.random() * 0.2;
    const k = Math.floor(Math.random() * (this.spots.length / 3)) * 3;
    const x = this.spots[k] + (Math.random() - 0.5) * 16, y = this.spots[k + 1] + (Math.random() - 0.5) * 8, z = this.spots[k + 2] + (Math.random() - 0.5) * 16;
    const color = FIREWORK[Math.floor(Math.random() * FIREWORK.length)];
    this.particles.burst(x, y, z, 110, color, 16, 1.3, 1.4, -5);
    this.particles.burst(x, y, z, 30, 0xffffff, 9, 0.9, 0.7, -5);
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
        if (_ob[3] < 0.05 && v.roll > 0.05) {
          for (let n = 0; n < 8; n++) this.smoke.spawn(_ob[0], _ob[1] + 0.3, _ob[2], (Math.random() - 0.5) * 8, 1, (Math.random() - 0.5) * 8, 0x8a8a8a, 2, 0.6, 0, 2);
          if (this.nearSelf(_ob[0], _ob[2]) < 25) this.shake.add(0.2);
        }
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
  private syncBananas(state: KartState): void {
    const alive = this.alive, pool = this.bananas;
    alive.clear();
    for (const h of state.hazards) {
      alive.add(h.id);
      let o = pool.get(h.id);
      if (!o) { o = this.lib.instance('toy-item-banana'); o.scale.setScalar(5); this.view.scene.add(o); pool.set(h.id, o); }
      o.position.set(h.x, h.y + 0.05, h.z);
    }
    for (const [id, o] of pool) if (!alive.has(id)) { o.removeFromParent(); pool.delete(id); }
  }
  /** Shells by kind (green, red, blue models; the bomb is the kit's), spinning, with a trail in their colour. */
  private syncShells(state: KartState): void {
    const alive = this.alive, pool = this.shells;
    alive.clear();
    for (const s of state.shells) {
      alive.add(s.id);
      let entry = pool.get(s.id);
      if (entry && entry.kind !== s.kind) { entry.object.removeFromParent(); entry = undefined; }
      if (!entry) {
        const object = s.kind === 'bomb' ? this.lib.instance('plat-bomb') : this.shellKit.build(s.kind);
        if (s.kind === 'bomb') object.scale.setScalar(2.6);
        this.view.scene.add(object);
        entry = { kind: s.kind, object };
        pool.set(s.id, entry);
      }
      const o = entry.object;
      if (s.kind === 'bomb') {
        o.position.set(s.x, s.y - 0.4 + Math.abs(Math.sin(this.time * 9)) * 0.4, s.z); o.rotation.y = s.yaw;
        if (Math.random() < 0.6) this.particles.spawn(s.x, s.y + 1.4, s.z, (Math.random() - 0.5), 1.5, (Math.random() - 0.5), 0xffd080, 0.4, 0.3);
      } else {
        // The blue shell flies high over the pack; the others skim the road spinning.
        o.position.set(s.x, s.y - 0.3 + (s.kind === 'blue' ? 3.5 + Math.sin(this.time * 4) * 0.3 : 0), s.z);
        o.rotation.y = this.time * (s.kind === 'blue' ? 4 : 14);
        if (Math.random() < 0.7) this.particles.spawn(s.x, o.position.y + 0.4, s.z, 0, 0.6, 0, SHELL_TINT[s.kind], 0.55, 0.35);
      }
    }
    for (const [id, e] of pool) if (!alive.has(id)) { e.object.removeFromParent(); pool.delete(id); }
  }

  private updateKart(v: KartVisual, r: KartState['racers'][number], dt: number, isSelf: boolean): void {
    const hidden = r.respawn > 0;
    v.root.visible = !hidden;
    if (hidden) return;
    v.root.position.set(r.x, r.y, r.z);
    // Lightning shrinks a racer while it is slowed; after a rescue the kart blinks while it is untouchable.
    v.size = approach(v.size, r.slow > 0 ? 0.62 : 1, 8, dt);
    v.root.scale.setScalar(v.size);
    v.body.visible = !(r.ghost > 0 && Math.floor(this.time * 14) % 2 === 0);
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
    // A burnout spins the wheels on the spot.
    v.wheelSpin += (r.stall > 0 ? 40 : r.speed) * dt / 0.42;
    for (const w of v.wheels) w.rotation.x = v.wheelSpin;
    for (const w of v.front) w.rotation.y = -r.steer * 0.45;
    v.mixer?.update(dt);
    // Glider canopy unfolds on launch.
    v.wingOpen = approach(v.wingOpen, r.glide ? 1 : 0, 8, dt);
    v.wing.visible = v.wingOpen > 0.02;
    if (v.wing.visible) { v.wing.scale.set(v.wingOpen, 1, 1); v.wing.rotation.z = -r.steer * 0.25 * v.wingOpen; }
    // Boost flames: longer for stronger boosts, flickering, in the colour of whatever fired them
    // (gold all through a golden-mushroom window).
    const boosting = r.boost > 0;
    if (r.golden > 0) v.flameTint = FLAME.golden;
    if (boosting) { v.flameMat.color.setHex(v.flameTint); v.exhaustMat.color.setHex(v.flameTint); }
    const len = 0.8 + clamp(r.boostPower - 1.25, 0, 0.35) * 2.6;
    for (let n = 0; n < 2; n++) {
      v.flames[n].visible = v.cores[n].visible = v.exhaust[n].visible = boosting;
      if (!boosting) continue;
      const s = len * (0.8 + Math.random() * 0.45) * Math.min(1, r.boost * 4 + 0.3);
      v.flames[n].scale.set(1, s, 1);
      v.flames[n].position.z = -1.3 - (s - 1) * 0.6;
      v.cores[n].scale.set(1, s * 0.85, 1);
      v.exhaust[n].scale.setScalar(1.1 + Math.random() * 0.5);
    }
    v.bubble.visible = r.star > 0;
    if (r.star > 0) { (v.bubble.material as THREE.MeshBasicMaterial).color.setHSL((this.time * 0.8) % 1, 1, 0.6); v.bubble.scale.setScalar(1 + Math.sin(this.time * 12) * 0.04); }
    // Particles from the rear wheels.
    const fx = Math.sin(r.yaw), fz = Math.cos(r.yaw), lx = fz, lz = -fx;
    const camDist = Math.hypot(r.x - this.view.camera.position.x, r.z - this.view.camera.position.z);
    const near = isSelf || camDist < 90;
    // Drift sparks: a glow at each rear wheel in the tier's colour (flaring on each new tier) plus sparks.
    const drifting = near && r.grounded && r.drift !== 0;
    for (const s of v.sparks) s.visible = drifting && r.driftLevel > 0;
    if (drifting && r.driftLevel > 0) {
      v.sparkMat.color.setHex(SPARK[r.driftLevel]);
      const flare = r.driftLevel > v.lastLevel ? 2.2 : 1;
      for (const s of v.sparks) s.scale.setScalar((0.9 + r.driftLevel * 0.35 + Math.random() * 0.5) * flare);
      if (r.driftLevel > v.lastLevel) this.particles.burst(r.x - fx, r.y + 0.4, r.z - fz, 22, SPARK[r.driftLevel], 7, 0.5, 0.35, -10);
    }
    v.lastLevel = r.drift !== 0 ? r.driftLevel : 0;
    if (drifting) {
      for (const side of [-1, 1]) {
        const x = r.x - fx * 1.0 + lx * side * 0.9, z = r.z - fz * 1.0 + lz * side * 0.9;
        this.particles.spawn(x, r.y + 0.25, z, (Math.random() - 0.5) * 3 - fx * 2, 1.5 + Math.random() * 2, (Math.random() - 0.5) * 3 - fz * 2, SPARK[r.driftLevel], r.driftLevel ? 0.45 : 0.3, 0.3, -9, 2);
        if (r.driftLevel === 0 && Math.random() < 0.3) this.smoke.spawn(x, r.y + 0.2, z, -fx, 0.6, -fz, 0xdadada, 0.9, 0.5, 0, 2);
      }
    }
    // Off-road: dust, snow or cinders kicked up behind.
    if (near && r.grounded && r.surface === 'grass' && Math.abs(r.speed) > 6 && Math.random() < 0.8) {
      this.smoke.spawn(r.x - fx * 1.2, r.y + 0.3, r.z - fz * 1.2, (Math.random() - 0.5) * 2 - fx * 2, 1.2 + Math.random(), (Math.random() - 0.5) * 2 - fz * 2, this.dustColor(), 1.0, 0.55, 0, 2);
    }
    // Skimming the water surface: a rooster tail of spray.
    const liquid = this.track.liquid;
    if (near && liquid !== null && !this.track.lava && r.grounded && !r.under && r.y < liquid + 0.3 && r.y > liquid - 1.4 && Math.abs(r.speed) > 8) {
      for (const side of [-1, 1]) this.particles.spawn(r.x - fx * 1.3 + lx * side * 0.8, liquid + 0.2, r.z - fz * 1.3 + lz * side * 0.8, -fx * 3 + lx * side * 2.5, 4 + Math.random() * 3, -fz * 3 + lz * side * 2.5, 0xe8fbff, 0.55, 0.6, -14, 1);
    }
    // Underwater: a trail of bubbles rising to the surface.
    if (near && r.under && Math.random() < 0.6) this.particles.spawn(r.x - fx * 1.4 + (Math.random() - 0.5), r.y + 0.6, r.z - fz * 1.4 + (Math.random() - 0.5), (Math.random() - 0.5) * 0.6, 2 + Math.random() * 1.5, (Math.random() - 0.5) * 0.6, 0xd8f6ff, 0.35, 1.6, 1.5, 0.5);
    if (near && boosting && Math.random() < 0.9) this.particles.spawn(r.x - fx * 1.9, r.y + 0.6, r.z - fz * 1.9, -fx * 5, 0.5, -fz * 5, v.flameTint, 0.55, 0.25, 0, 3);
    // Burnout: tyre smoke pouring off the spinning rear wheels.
    if (near && r.stall > 0 && Math.random() < 0.8) for (const side of [-1, 1]) this.smoke.spawn(r.x - fx * 1.0 + lx * side * 0.9, r.y + 0.3, r.z - fz * 1.0 + lz * side * 0.9, (Math.random() - 0.5) * 1.5 - fx * 1.5, 0.8 + Math.random(), (Math.random() - 0.5) * 1.5 - fz * 1.5, 0x5a5a5a, 2, 0.9, 0, 1.5);
    // Slipstream: wind streaks pouring past while the draft charges.
    if (near && r.draft > 0.25 && Math.random() < 0.7) {
      const side = Math.random() < 0.5 ? -1 : 1;
      this.particles.spawn(r.x + fx * 2.5 + lx * side * (1 + Math.random()), r.y + 0.6 + Math.random() * 1.4, r.z + fz * 2.5 + lz * side * (1 + Math.random()), -fx * 22, 0, -fz * 22, 0xdff6ff, 0.18, 0.3, 0, 0);
    }
    if (r.golden > 0 && Math.random() < 0.6) this.particles.spawn(r.x + (Math.random() - 0.5) * 2.4, r.y + Math.random() * 2, r.z + (Math.random() - 0.5) * 2.4, 0, 1.2, 0, 0xffd84a, 0.4, 0.5);
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
    this.sky.uniforms.under.value = under ? 1 : 0;
  }
  /** Whether the camera is under water (the HUD tints the view). */
  get isUnderwater(): boolean { return this.underwater; }

  private updateCamera(state: KartState, self: KartState['racers'][number], dt: number, lookBack: boolean): void {
    const cam = this.view.camera;
    const path = this.track.paths[self.seg] ?? this.track;
    if (this.pin) {
      cam.position.set(this.pin[0], this.pin[1], this.pin[2]);
      cam.lookAt(this.pin[3], this.pin[4], this.pin[5]);
      cam.fov = 68; cam.updateProjectionMatrix();
      return;
    }
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
      this.resetCamera(cam.position, _look.set(self.x, self.y + 1.2, self.z));
      return;
    }
    const heading = r_heading(self);
    const back = lookBack ? -1 : 1;
    // Pulled back a little at speed and on a boost, further on the glider.
    const dist = 7.8 + clamp(self.speed, 0, 40) * 0.05 + (self.boost > 0 ? 0.9 : 0) + (self.glide ? 3 : 0);
    _v.set(self.x - Math.sin(heading) * dist * back, self.y + 3.4 + (self.glide ? 1.6 : 0), self.z - Math.cos(heading) * dist * back);
    // Keep the camera above the terrain, and under the roof in tunnels.
    const n = nearest(path, _v.x, _v.z, self.hint, _near, 20);
    // On walled roads keep the camera on the road side of the barrier: a kart drifting along a wall
    // swings it outward, into pit canopies, grandstands and buildings (fork mouths have no wall).
    let walls = this.camWalls.get(path.index);
    if (!walls) { walls = barriers(this.track, path).filter(b => b.kind === 'wall'); this.camWalls.set(path.index, walls); }
    for (const b of walls) {
      const limit = Math.abs(b.lateral) - 0.6;
      if (b.mask[n.index] && Math.sign(n.lateral) === b.side && Math.abs(n.lateral) > limit) {
        pointAt(path, n.s, b.side * limit, _cp);
        _v.x = _cp[0]; _v.z = _cp[2];
      }
    }
    const ground = this.track.ground(_v.x, _v.z, n, state.raceTime);
    if (Math.abs(n.lateral) < path.halfWidth + 6 || !(path.drop[n.index] > 0)) _v.y = Math.max(_v.y, ground + 1.5);
    if (path.flags[n.index] & F_TUNNEL && Math.abs(n.lateral) < path.halfWidth + 1) _v.y = Math.min(_v.y, n.y + TUNNEL_HEIGHT - 1.4);
    _look.set(self.x + Math.sin(heading) * 4 * back, self.y + 1.3, self.z + Math.cos(heading) * 4 * back);
    if (!this.camReady || lookBack) this.resetCamera(_v, _look);
    // A critically damped spring trails the kart (it lags ~2·speed/ω behind: a few metres more on a boost)
    // the look point leads it more tightly.
    const pos = this.camPos;
    cam.position.set(springTo(pos.x, _v.x, 15, dt), springTo(pos.y, _v.y, 11, dt), springTo(pos.z, _v.z, 15, dt));
    this.camLook.x = approach(this.camLook.x, _look.x, 14, dt); this.camLook.y = approach(this.camLook.y, _look.y, 10, dt); this.camLook.z = approach(this.camLook.z, _look.z, 14, dt);
    cam.lookAt(this.camLook);
    // Shake on landings, hits and nearby blasts.
    this.shake.update(dt);
    if (this.shake.trauma > 0) {
      cam.position.x += this.shake.offset(this.time, 0, 0.35); cam.position.y += this.shake.offset(this.time, 1, 0.3);
      cam.rotation.z += this.shake.offset(this.time, 2, 0.05);
    }
    cam.fov = approach(cam.fov, chaseFov(self.speed, self.boost, self.boostPower, self.glide), self.boost > 0 ? 7 : 3, dt);
    cam.updateProjectionMatrix();
  }
  private resetCamera(pos: THREE.Vector3, look: THREE.Vector3): void {
    const c = this.camPos;
    for (const [s, x] of [[c.x, pos.x], [c.y, pos.y], [c.z, pos.z]] as [Spring, number][]) { s.x = x; s.v = 0; }
    this.camLook.copy(look);
    this.camReady = true;
  }

  stats() { return { ...this.view.stats(), camera: { x: this.camPos.x.x, y: this.camPos.y.x, z: this.camPos.z.x }, underwater: this.underwater }; }

  dispose(): void {
    this.particles.dispose();
    this.smoke.dispose();
    for (const k of this.karts) k.mixer?.stopAllAction();
    for (const o of this.obstacles) o.mesh.removeFromParent();
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
