// Kart world builder: the KartRenderer base class that owns the renderer's
// state and builds the static track world (terrain, liquid, roads, walls,
// bridges, tunnels, half-pipes, drawbridges, coins, start grid, obstacles,
// props, crowds and chevrons). ../render.ts animates it.
import * as THREE from 'three';
import { AssetLibrary, PropBatch } from '../../common/assets';
import { GameView, owned } from '../../common/view';
import { Particles } from '../../common/particles';
import { clamp, fbm, hashString, lerp, makeNoise, smoothstep } from '../../common/math';
import { checkerTexture, labelTexture, stripeTexture } from '../../common/textures';
import { coinSpots, CRUSHER_HALF, F_BRIDGE, F_DRAW, F_GAP, F_RAILS, LAUNCH_HEIGHT, LAUNCH_LENGTH, nearest, pipeOn, RAIL_SHOULDER, pointAt, rampOn, RAMP_HEIGHT, RAMP_LENGTH, ROLLER_RADIUS, TUNNEL_HEIGHT, type Path, type Track } from '../tracks';
import { abutments, barriers, forkSign, gantries, pillars, propLayout, skirts, terrainGrid, towers, tubeMask, TUBE_PAD, worldHeight, type Barrier, type Gantry, type PropPlacement } from '../layout';
import type { ShellKind } from '../sim';
import { THEMES, type KartTheme } from './themes';
import { RoadKit } from './road';
import { lavaMaterial, patchCrowd, patchWorld, skyMaterial, waterMaterial, type SharedUniforms } from './atmosphere';
import { barrierTexture, chevronBoardTexture, glowTexture, gridSlotTexture, terrainTexture } from './surfaces';
import { chevronBoards, extraProps, gridSlots, CHEVRON_HALF, CHEVRON_HEIGHT } from './dressing';
import { Bolts, Rings, ShellKit, SpeedLines } from './fx';
import { Shake, spring } from './camera';
import { TERRAIN, _near, _pt, _v, _m, _q, _s, _p, _up, type KartVisual, type ObstacleVisual, type DrawbridgeVisual, type RenderRacer } from './shared';

export class KartWorld {
  readonly view: GameView;
  protected lib: AssetLibrary;
  protected track: Track;
  protected theme: KartTheme;
  protected road!: RoadKit;
  protected shared: SharedUniforms = { time: { value: 0 } };
  protected cheer = { value: 0.3 };
  protected sky!: THREE.ShaderMaterial;
  protected karts: KartVisual[] = [];
  protected boxes: THREE.Object3D[] = [];
  protected bananas = new Map<number, THREE.Object3D>();
  protected shells = new Map<number, { kind: ShellKind; object: THREE.Object3D }>();
  protected shellKit!: ShellKit;
  protected obstacles: ObstacleVisual[] = [];
  protected drawbridges: DrawbridgeVisual[] = [];
  protected coins: THREE.InstancedMesh | null = null;
  protected particles = new Particles(3200, true);
  protected smoke = new Particles(900, false);
  protected speedLines!: SpeedLines;
  protected bolts!: Bolts;
  protected rings!: Rings;
  protected camPos = { x: spring(), y: spring(), z: spring() };
  protected camLook = new THREE.Vector3();
  protected camReady = false;
  /** Walls per path index, for keeping the chase camera on the road side of them (built on first use). */
  protected camWalls = new Map<number, Barrier[]>();
  protected shake = new Shake();
  protected flash = 0;
  protected fireworks = 0;
  protected fireworkAt = 0;
  protected spots: number[] = [];
  protected underwater = false;
  protected time = 0;
  /** Test / capture hook: a fixed camera pose [x, y, z, lookX, lookY, lookZ] overriding the chase camera. */
  pin: number[] | null = null;
  protected flameGeo = new THREE.ConeGeometry(0.24, 1.3, 10, 1, true);
  protected coreGeo = new THREE.ConeGeometry(0.11, 0.9, 8, 1, true);
  protected coreMat = new THREE.MeshBasicMaterial({ color: 0xfff6d8, transparent: true, opacity: 0.95, blending: THREE.AdditiveBlending, depthWrite: false, toneMapped: false });
  protected glow = glowTexture();
  protected bubbleGeo = new THREE.SphereGeometry(2.0, 20, 14);
  protected bubbleMat = new THREE.MeshBasicMaterial({ color: 0xffe066, transparent: true, opacity: 0.22, blending: THREE.AdditiveBlending, depthWrite: false });
  protected wingGeo = new THREE.BoxGeometry(4.2, 0.08, 1.5);
  protected strutGeo = new THREE.CylinderGeometry(0.05, 0.05, 1.3, 6);
  protected strutMat = new THREE.MeshLambertMaterial({ color: 0x333333 });
  protected racers: RenderRacer[];
  protected self: number;

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

  // ── World ────────────────────────────────────────────────────────────────
  protected buildWorld(): void {
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

}
