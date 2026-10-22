/**
 * Kart road surfaces: every route's tarmac (world-space asphalt with normal
 * and roughness maps, so a branch overlapping the main road at a fork is
 * pixel-identical and the join is invisible), boardwalk planks and metal
 * decks on the themed bridges, worn racing grooves, edge lines broken at fork
 * mouths, tyre marks through the corners, raised rumble curbs (theme colours,
 * a second pair on banked bends), off-road sand / snow / cinders / mud, glossy
 * ice, sponsor banners and neon along the barrier walls, and dash panels with
 * scrolling chevrons. Each kind of paint is merged into one mesh per route.
 */
import * as THREE from 'three';
import type { AssetLibrary } from '../../common/assets';
import { owned } from '../../common/view';
import { F_BRIDGE, F_DRAW, F_GAP, F_PIPE, F_TUNNEL, pointAt, rampOn, sampleAt, type Path, type Track } from '../tracks';
import { barriers, edgeClear } from '../layout';
import { branchFade, skidAlpha, wallBanners } from './dressing';
import { patchWorld, type SharedUniforms } from './atmosphere';
import { asphaltSurface, bannerTexture, dashTexture, grooveTexture, paintTexture, rumbleTexture, skidTexture, surfaceTexture, type SurfaceKind } from './surfaces';
import type { KartTheme } from './themes';

/** One cross-section point of a swept profile: lateral offset, height above the road, texture u. */
type ProfilePoint = [number, number, number];
const _pt = [0, 0, 0, 0];
/** Tarmac tile size in world metres (asphalt UVs are world XZ). */
const ASPHALT_TILE = 7;

/**
 * Accumulates swept ribbons into one geometry: positions, uv (path u/v or
 * world XZ), and a per-vertex alpha in an RGBA colour attribute.
 */
export class Ribbons {
  private pos: number[] = []; private uv: number[] = []; private col: number[] = []; private idx: number[] = [];
  /**
   * Sweeps `profile` along `path` wherever `filter` allows; v = distance /
   * `vScale` (negative flips it), or world XZ / |vScale| when `world`.
   * `alpha` fades per sample. Heights follow the road's banking (and its
   * ramps unless `ramps` is false: wall-mounted pieces follow the wall).
   */
  sweep(path: Path, profile: ProfilePoint[], lift: number, vScale: number, filter: (i: number) => boolean, alpha?: (i: number) => number, world = false, ramps = true): this {
    const n = path.count, last = path.closed ? n : n - 1, P = profile.length;
    let open = false;
    for (let i = 0; i <= last; i++) {
      const k = i % n;
      const a = filter(k) ? (alpha ? alpha(k) : 1) : -1;
      if (a < 0) { open = false; continue; }
      const lx = path.tz[k], lz = -path.tx[k], b = path.bank[k];
      const y = path.py[k] + lift + (ramps ? rampOn(path, path.dist[k]) : 0);
      const v = (i === n && path.closed ? path.length : path.dist[k]) / vScale;
      for (const [lat, h, u] of profile) {
        const x = path.px[k] + lx * lat, z = path.pz[k] + lz * lat;
        this.pos.push(x, y + h - lat * b, z);
        if (world) this.uv.push(x / Math.abs(vScale), z / Math.abs(vScale)); else this.uv.push(u, v);
        this.col.push(1, 1, 1, a);
      }
      if (open) {
        const b1 = this.pos.length / 3 - P, b0 = b1 - P;
        for (let p = 0; p < P - 1; p++) this.idx.push(b0 + p, b1 + p, b0 + p + 1, b0 + p + 1, b1 + p, b1 + p + 1);
      }
      open = true;
    }
    return this;
  }
  get empty(): boolean { return this.idx.length === 0; }
  mesh(material: THREE.Material, alpha = false, order = 0): THREE.Mesh {
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.Float32BufferAttribute(this.pos, 3));
    geo.setAttribute('uv', new THREE.Float32BufferAttribute(this.uv, 2));
    if (alpha) geo.setAttribute('color', new THREE.Float32BufferAttribute(this.col, 4));
    geo.setIndex(this.idx);
    geo.computeVertexNormals();
    const mesh = owned(new THREE.Mesh(geo, material));
    mesh.receiveShadow = true;
    // Overlays sort among themselves below the water surface (renderOrder 2), which writes no depth.
    mesh.renderOrder = order;
    return mesh;
  }
}

/** Builds every route's surfaces with materials shared across routes. */
export class RoadKit {
  /** Scrolled each frame: the dash-panel chevrons. */
  readonly dash: THREE.CanvasTexture;
  /** Every material whose program needs the caustics / glow patch (the renderer applies the same to the terrain). */
  readonly lit: THREE.Material[] = [];
  private asphalt: THREE.MeshStandardMaterial;
  private deck: THREE.MeshStandardMaterial | null = null;
  private groove: THREE.MeshLambertMaterial;
  private paint: THREE.MeshLambertMaterial;
  private skid: THREE.MeshLambertMaterial;
  private curb: THREE.MeshLambertMaterial;
  private bankCurb: THREE.MeshLambertMaterial;
  private offroad: THREE.MeshLambertMaterial;
  private ice: THREE.MeshStandardMaterial;
  private banner: THREE.MeshLambertMaterial;
  private neon: THREE.MeshBasicMaterial | null;
  private panel: THREE.MeshBasicMaterial;
  private frame: THREE.MeshLambertMaterial;
  private deckKind: SurfaceKind | null;

  constructor(private lib: AssetLibrary, private track: Track, private theme: KartTheme, private shared: SharedUniforms, private scene: THREE.Scene) {
    const own = <T extends { dispose(): void }>(r: T): T => lib.own(r);
    const set = asphaltSurface();
    for (const t of [set.map, set.normalMap, set.roughnessMap]) own(t);
    this.asphalt = own(new THREE.MeshStandardMaterial({ color: theme.road, map: set.map, normalMap: set.normalMap, normalScale: new THREE.Vector2(0.7, 0.7), roughnessMap: set.roughnessMap, roughness: 1, metalness: 0 }));
    // Themed decks: boardwalk planks over the lagoon, steel plate on the neon skyway's flyovers.
    const t = track.def.theme;
    this.deckKind = t === 'lagoon' ? 'planks' : t === 'dusk' ? 'metal' : null;
    if (this.deckKind) {
      const metal = this.deckKind === 'metal';
      this.deck = own(new THREE.MeshStandardMaterial({ color: metal ? 0x9aa4b4 : 0xffffff, map: own(surfaceTexture(this.deckKind)), roughness: metal ? 0.45 : 0.85, metalness: metal ? 0.6 : 0 }));
    }
    const overlay = (map: THREE.Texture, order = 0) => own(new THREE.MeshLambertMaterial({ map, transparent: true, depthWrite: false, vertexColors: true, polygonOffset: true, polygonOffsetFactor: -1 - order, polygonOffsetUnits: -2 - order }));
    this.groove = overlay(own(grooveTexture()));
    this.skid = overlay(own(skidTexture()), 1);
    this.paint = overlay(own(paintTexture()), 2);
    this.curb = own(new THREE.MeshLambertMaterial({ map: own(rumbleTexture(...theme.curb)) }));
    this.bankCurb = own(new THREE.MeshLambertMaterial({ map: own(rumbleTexture(...theme.bankCurb)) }));
    this.offroad = overlay(own(surfaceTexture(theme.offroad)), 1);
    this.ice = own(new THREE.MeshStandardMaterial({ map: own(surfaceTexture('ice')), transparent: true, roughness: 0.12, metalness: 0.05, vertexColors: true, depthWrite: false, polygonOffset: true, polygonOffsetFactor: -2, polygonOffsetUnits: -3 }));
    this.banner = own(new THREE.MeshLambertMaterial({ map: own(bannerTexture(...theme.banner)), side: THREE.DoubleSide, polygonOffset: true, polygonOffsetFactor: -1, polygonOffsetUnits: -2 }));
    this.neon = theme.neon !== null ? own(new THREE.MeshBasicMaterial({ color: theme.neon, toneMapped: false })) : null;
    this.dash = own(dashTexture());
    this.dash.repeat.set(1, 2);
    this.panel = own(new THREE.MeshBasicMaterial({ map: this.dash, transparent: true, toneMapped: false, depthWrite: false, polygonOffset: true, polygonOffsetFactor: -3, polygonOffsetUnits: -4 }));
    this.frame = own(new THREE.MeshLambertMaterial({ color: 0x2a2a2e }));
    this.lit.push(this.asphalt, this.curb, this.bankCurb);
    if (this.deck) this.lit.push(this.deck);
    // Under the lagoon: caustics on the tarmac (and on the land: the renderer patches the terrain too).
    if (track.liquid !== null && !track.lava) for (const m of this.lit) patchWorld(m, { caustics: track.liquid }, shared);
  }

  /** Every surface of one route. */
  build(path: Path): void {
    const { track, scene } = this;
    const hw = path.halfWidth;
    const flag = (i: number, f: number) => (path.flags[i] & f) !== 0;
    const paved = (i: number) => !flag(i, F_GAP | F_DRAW | F_PIPE);
    const { left, right } = edgeClear(track, path);
    const frac = (i: number) => path.dist[i] / path.length;
    // Branches sit a hair above the main road where they overlap at the fork and merge.
    const lift = path.index === 0 ? 0.06 : 0.075;
    const decked = (i: number) => this.deck !== null && flag(i, F_BRIDGE);
    const flat: ProfilePoint[] = [[-hw, 0, 0], [hw, 0, 1]];
    scene.add(new Ribbons().sweep(path, flat, lift, ASPHALT_TILE, i => paved(i) && !decked(i), undefined, true).mesh(this.asphalt));
    if (this.deck) {
      const deck = new Ribbons().sweep(path, flat, lift, 4, i => paved(i) && decked(i));
      if (!deck.empty) scene.add(deck.mesh(this.deck));
    }
    // Racing grooves and corner tyre marks (both fade out in fork mouths).
    const plain = (i: number) => paved(i) && !decked(i);
    scene.add(new Ribbons().sweep(path, flat, lift + 0.004, 30, plain, i => branchFade(path, i)).mesh(this.groove, true, 0.1));
    const skid = skidAlpha(path);
    const marks = new Ribbons().sweep(path, [[-hw * 0.92, 0, 0], [hw * 0.92, 0, 1]], lift + 0.008, 22, i => plain(i) && skid[i] > 0.02, i => skid[i] * 0.9);
    if (!marks.empty) scene.add(marks.mesh(this.skid, true, 0.2));
    // Edge lines, broken wherever another route's tarmac meets this one.
    const lines = new Ribbons();
    for (const side of [-1, 1]) {
      const mask = side > 0 ? left : right;
      const a = side * (hw - 0.55), b = side * (hw - 0.22);
      lines.sweep(path, side > 0 ? [[a, 0, 0], [b, 0, 1]] : [[b, 0, 0], [a, 0, 1]], lift + 0.012, 12, i => plain(i) && mask[i] === 1 && !this.onIce(path, i));
    }
    if (!lines.empty) scene.add(lines.mesh(this.paint, true, 0.4));
    this.buildCurbs(path, lift, paved, left, right);
    // Off-road patches and ice, faded in and out along the road.
    for (const o of path.features.offroad) {
      const [l0, l1] = o.lanes ?? [-1, 1];
      const ends = (i: number) => Math.min(1, (frac(i) - o.from) * path.length / 6, (o.to - frac(i)) * path.length / 6);
      scene.add(new Ribbons().sweep(path, [[l0 * hw - 0.6, 0, 0], [l1 * hw + 0.6, 0, 1]], lift + 0.02, 8, i => paved(i) && frac(i) >= o.from && frac(i) <= o.to, ends).mesh(this.offroad, true, 0.3));
    }
    if (path.features.ice.length) {
      const ends = (i: number) => { for (const [a, b] of path.features.ice) if (frac(i) >= a && frac(i) <= b) return Math.min(1, (frac(i) - a) * path.length / 4, (b - frac(i)) * path.length / 4); return 0; };
      scene.add(new Ribbons().sweep(path, flat, lift + 0.02, 10, i => paved(i) && this.onIce(path, i), i => 0.35 + ends(i) * 0.65).mesh(this.ice, true, 0.5));
    }
    // Banners hung on the barrier walls (reading left to right from the road on both sides).
    for (const b of wallBanners(track, path)) {
      const lat = b.lateral - b.side * 0.04;
      const r = new Ribbons().sweep(path, [[lat, b.top - 0.12, 0], [lat, 0.12, 1]], 0, b.side > 0 ? 36 : -36, i => b.mask[i] === 1, undefined, false, false);
      if (!r.empty) scene.add(r.mesh(this.banner));
    }
    // Neon along the barrier tops.
    if (this.neon) {
      const neon = new Ribbons();
      for (const b of barriers(track, path)) {
        if (b.kind !== 'wall' && b.kind !== 'rail') continue;
        const a = b.lateral, c = b.lateral + b.side * b.thick;
        neon.sweep(path, b.side > 0 ? [[a, b.top + 0.02, 0], [c, b.top + 0.02, 1]] : [[c, b.top + 0.02, 0], [a, b.top + 0.02, 1]], 0, 4, i => b.mask[i] === 1, undefined, false, false);
      }
      if (!neon.empty) scene.add(neon.mesh(this.neon));
    }
    this.buildPads(path, lift);
  }

  /** Tarmac for meshes with their own 0..1 UVs (half-pipe decks, drawbridge leaves): the same maps repeated `u` × `v`. */
  tarmac(u: number, v: number): THREE.MeshStandardMaterial {
    const m = this.lib.own(this.asphalt.clone());
    for (const k of ['map', 'normalMap', 'roughnessMap'] as const) {
      const t = this.lib.own(this.asphalt[k]!.clone());
      t.repeat.set(u, v);
      m[k] = t;
    }
    return m;
  }

  private onIce(path: Path, i: number): boolean {
    const f = path.dist[i] / path.length;
    return path.features.ice.some(([a, b]) => f >= a && f <= b);
  }

  /**
   * Raised rumble curbs: along open (grass) roads everywhere, along walled
   * roads through the corners, never on decks, in tunnels or across a fork
   * mouth; a second colour pair on banked bends.
   */
  private buildCurbs(path: Path, lift: number, paved: (i: number) => boolean, left: Uint8Array, right: Uint8Array): void {
    const hw = path.halfWidth, n = path.count;
    const open = path.edge === 'grass';
    const W = open ? 1.15 : 1.0;
    const corner = new Uint8Array(n);
    if (!open) for (let i = 0; i < n; i++) {
      if (Math.abs(path.curve[i]) < 0.014) continue;
      for (let k = -5; k <= 5; k++) { const j = path.closed ? (i + k + n) % n : Math.max(0, Math.min(n - 1, i + k)); corner[j] = 1; }
    }
    const ok = (i: number) => paved(i) && !(path.flags[i] & (F_BRIDGE | F_TUNNEL)) && (open || corner[i] === 1);
    const banked = (i: number) => Math.abs(path.bank[i]) > 0.02;
    for (const [mat, pick] of [[this.curb, (i: number) => !banked(i)], [this.bankCurb, banked]] as [THREE.Material, (i: number) => boolean][]) {
      const r = new Ribbons();
      for (const side of [-1, 1]) {
        const mask = side > 0 ? left : right;
        // Inner lip flush with the road, a short ramp up to a 12 cm crown, rounding down outside.
        const prof: ProfilePoint[] = [[hw - 0.05, 0, 0], [hw + 0.22, 0.12, 0.25], [hw + W - 0.25, 0.12, 0.75], [hw + W, 0.02, 1]];
        const p = prof.map(([l, h, u]): ProfilePoint => [side * l, h, u]);
        r.sweep(path, side > 0 ? p : p.reverse(), lift + 0.005, 3, i => ok(i) && pick(i) && mask[i] === 1);
      }
      if (!r.empty) { const m = r.mesh(mat); m.castShadow = true; this.scene.add(m); }
    }
  }

  /** Dash panels: a dark frame with a glowing panel whose chevrons scroll forward (one shared texture). */
  private buildPads(path: Path, lift: number): void {
    const pads = path.features.boostPads;
    if (!pads.length) return;
    const hw = path.halfWidth;
    const frameGeo = new THREE.BoxGeometry(4.4, 0.08, 5.0), panelGeo = new THREE.PlaneGeometry(3.8, 4.4);
    panelGeo.rotateX(-Math.PI / 2);
    const frames = owned(new THREE.InstancedMesh(frameGeo, this.frame, pads.length));
    const panels = owned(new THREE.InstancedMesh(panelGeo, this.panel, pads.length));
    const m = new THREE.Matrix4(), q = new THREE.Quaternion(), e = new THREE.Euler(), p = new THREE.Vector3(), s = new THREE.Vector3(1, 1, 1);
    pads.forEach((pad, k) => {
      const at = pad.at * path.length, lat = pad.lane * hw;
      pointAt(path, at, lat, _pt);
      const i = sampleAt(path, at);
      const y = _pt[1] + lift - lat * path.bank[i];
      e.set(0, _pt[3], -Math.atan(path.bank[i]), 'YXZ'); q.setFromEuler(e);
      frames.setMatrixAt(k, m.compose(p.set(_pt[0], y + 0.02, _pt[2]), q, s));
      panels.setMatrixAt(k, m.compose(p.set(_pt[0], y + 0.075, _pt[2]), q, s));
    });
    for (const im of [frames, panels]) { im.instanceMatrix.needsUpdate = true; im.computeBoundingSphere(); im.receiveShadow = true; }
    panels.renderOrder = 0.6;
    this.scene.add(frames, panels);
  }
}
