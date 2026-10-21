/**
 * Kart tracks: a closed main loop plus optional branches (forks that leave the
 * loop and rejoin it later), each a centripetal Catmull-Rom centre line sampled
 * every ~2 m. Pure data + geometry (no three.js) so the simulation, renderer and
 * tests share it. Coordinates: metres, Y up. Distance `s` runs from the start
 * line along the main loop; a branch has its own local distance and maps onto
 * main-loop progress linearly between its fork and merge, so laps, ranking and
 * wrong-way detection work on every route.
 *
 * Per-sample flags carry the course furniture the simulation needs in O(1):
 * bridges (the ground falls away beside the deck), gaps (no road: jump or
 * glide), tunnels, rails, ice, off-road patches, banking and stretches that run
 * under the water surface. Overpasses (a stretch passing above another) are
 * detected and railed automatically.
 */
import { clamp, fbm, hashString, makeNoise, smoothstep } from '../common/math.ts';

export type TrackId = 'meadow' | 'downtown' | 'canyon' | 'frost' | 'lagoon' | 'volcano' | 'skyway';
export type EdgeKind = 'grass' | 'wall';
export type ThemeId = 'meadow' | 'city' | 'canyon' | 'frost' | 'lagoon' | 'volcano' | 'dusk';
/** [from, to] as fractions of a path's length. */
export type Range = [number, number];
type P3 = [number, number, number];
/** Big launch ramp: `glide` opens the glider (slow fall, long flight). */
export interface Launch { at: number; power: number; glide?: boolean }
/** Slow sand / mud / gravel over [from, to], optionally only between two lanes (-1..1 of half width). */
export interface Offroad { from: number; to: number; lanes?: [number, number] }
/** Elevated deck: beside it the ground falls away (water, lava or a ravine). Rails keep racers on. */
export interface Bridge { from: number; to: number; rails: boolean }
export interface Tunnel { from: number; to: number; glass?: boolean }
export type ObstacleKind = 'roller' | 'crusher';
/** Course hazard. Rollers sweep across the road; crushers slam down on `lane`. Motion is a pure function of race time. */
export interface ObstacleDef { at: number; kind: ObstacleKind; period: number; phase?: number; lane?: number }
export interface Features {
  /** Fractions of the path carrying a row of item boxes. */
  itemRows: number[];
  /** Dash panels: path fraction + lateral position (-1..1 of half width). */
  boostPads: { at: number; lane: number }[];
  /** Full-width kicker ramps (path fraction of the lip). */
  ramps: number[];
  /** Low-grip ranges. */
  ice: Range[];
  launches: Launch[];
  offroad: Offroad[];
  bridges: Bridge[];
  /** No road: a chasm (or open water) crossed by a launch or the glider. */
  gaps: Range[];
  tunnels: Tunnel[];
  /** Ranges banked from their curvature (outside edge raised). */
  banked: Range[];
  obstacles: ObstacleDef[];
}
export interface BranchDef extends Partial<Features> {
  name: string;
  /** Leaves the main loop at lap fraction `from` and rejoins it at `to` (from < to; never across the start line). */
  from: number; to: number;
  /** Interior control points [x, z, y]; the ends are taken from the main loop. */
  points: P3[];
  width?: number; edge?: EdgeKind; outOfBounds?: number;
  /** Chance a CPU takes this route, per difficulty (easy, normal, hard). */
  cpu: [number, number, number];
  /** Shorter but riskier (narrow, off-road or unrailed). */
  shortcut?: boolean;
}
export interface TrackDef extends Partial<Features> {
  id: TrackId; name: string; blurb: string;
  /** [x, z, y] control points, start line at the first point. */
  points: P3[];
  width: number;
  edge: EdgeKind;
  /** Grass tracks: distance beyond the road edge where a racer is rescued. */
  outOfBounds: number;
  itemRows: number[];
  boostPads: { at: number; lane: number }[];
  ramps: number[];
  ice: Range[];
  hills: number;
  theme: ThemeId;
  branches?: BranchDef[];
  /** A water (drive under it) or lava (rescue) surface at height `level`. */
  liquid?: { level: number; kind: 'water' | 'lava' };
  /** [x, z, radius]: the land sinks below the liquid (lagoons, lava lakes); never under the road itself. */
  pools?: [number, number, number][];
}

// ── Authoring helpers ───────────────────────────────────────────────────────
/** Control points along a circular arc (a helix when y changes). Degrees; x = cx + r·cos a, z = cz + r·sin a. */
function arc(cx: number, cz: number, r: number, a0: number, a1: number, y0: number, y1: number, step = 30, r1 = r): P3[] {
  const n = Math.max(1, Math.round(Math.abs(a1 - a0) / step));
  const out: P3[] = [];
  for (let k = 0; k <= n; k++) {
    const f = k / n, a = (a0 + (a1 - a0) * f) * Math.PI / 180, rr = r + (r1 - r) * f;
    out.push([cx + rr * Math.cos(a), cz + rr * Math.sin(a), y0 + (y1 - y0) * f]);
  }
  return out;
}

export const TRACK_DEFS: Record<TrackId, TrackDef> = {
  meadow: {
    id: 'meadow', name: 'Sunny Meadow', blurb: 'Rolling farmland, a lakeside sweep and a hilltop jump. A friendly first Grand Prix.',
    points: [[0, 0, 0], [100, 0, 0], [150, 20, 1], [170, 60, 3], [150, 100, 5], [110, 110, 6], [80, 140, 7], [90, 185, 8], [140, 200, 7], [200, 190, 5], [250, 210, 4], [270, 260, 4], [230, 300, 3], [150, 300, 2], [60, 290, 3], [-10, 260, 5], [-40, 210, 7], [-20, 160, 8], [-50, 110, 6], [-100, 100, 4], [-130, 60, 2], [-100, 15, 0], [-50, 0, 0]],
    width: 17, edge: 'grass', outOfBounds: 16, itemRows: [0.16, 0.47, 0.76], boostPads: [{ at: 0.31, lane: 0.35 }, { at: 0.62, lane: -0.35 }, { at: 0.9, lane: 0 }],
    ramps: [0.555], ice: [], hills: 9, theme: 'meadow',
  },
  downtown: {
    id: 'downtown', name: 'Downtown Dash', blurb: 'Tight city blocks between towers, barrier-lined corners and a boulevard boost lane.',
    points: [[0, 0, 0], [120, 0, 0], [150, 15, 0], [160, 45, 0], [160, 90, 1], [140, 115, 2], [100, 120, 3], [80, 140, 4], [80, 190, 5], [100, 215, 5], [150, 220, 4], [175, 245, 3], [170, 290, 2], [130, 305, 1], [40, 305, 0], [0, 290, 0], [-20, 260, 0], [-20, 200, 1], [-50, 180, 2], [-90, 175, 2], [-110, 150, 1], [-110, 60, 0], [-90, 20, 0], [-50, 0, 0]],
    width: 16, edge: 'wall', outOfBounds: 0, itemRows: [0.14, 0.43, 0.7], boostPads: [{ at: 0.05, lane: -0.4 }, { at: 0.36, lane: 0.3 }, { at: 0.58, lane: 0 }, { at: 0.86, lane: 0.4 }],
    ramps: [0.27], ice: [], hills: 0, theme: 'city',
  },
  canyon: {
    id: 'canyon', name: 'Canyon Run', blurb: 'Red-rock switchbacks, steep climbs and a big jump over the dry riverbed.',
    points: [[0, 0, 0], [90, -10, 2], [140, 10, 5], [160, 50, 8], [130, 90, 10], [80, 100, 11], [60, 130, 12], [80, 170, 13], [140, 175, 15], [190, 150, 17], [230, 170, 18], [240, 220, 16], [200, 260, 13], [130, 270, 10], [70, 250, 9], [20, 270, 10], [-40, 260, 11], [-70, 220, 9], [-60, 170, 7], [-100, 140, 6], [-110, 90, 4], [-80, 40, 2], [-50, 10, 0]],
    width: 16, edge: 'wall', outOfBounds: 0, itemRows: [0.18, 0.5, 0.77], boostPads: [{ at: 0.33, lane: 0 }, { at: 0.64, lane: 0.4 }, { at: 0.92, lane: -0.3 }],
    ramps: [0.41, 0.84], ice: [], hills: 22, theme: 'canyon',
  },
  frost: {
    id: 'frost', name: 'Frostbite Pass', blurb: 'Snowy pines, an icy lake crossing and a mountain-pass descent. Watch your grip.',
    points: [[0, 0, 0], [110, 0, 1], [160, -30, 3], [220, -30, 5], [260, 0, 7], [270, 50, 9], [240, 90, 11], [190, 100, 12], [160, 130, 12], [170, 180, 13], [220, 200, 14], [250, 240, 12], [220, 280, 9], [150, 290, 6], [80, 270, 4], [40, 230, 3], [0, 240, 3], [-50, 250, 4], [-90, 220, 5], [-90, 170, 4], [-60, 130, 3], [-80, 90, 2], [-90, 40, 1], [-50, 5, 0]],
    width: 17, edge: 'grass', outOfBounds: 15, itemRows: [0.15, 0.44, 0.73], boostPads: [{ at: 0.28, lane: -0.3 }, { at: 0.57, lane: 0.3 }, { at: 0.88, lane: 0 }],
    ramps: [0.66], ice: [[0.47, 0.55]], hills: 16, theme: 'frost',
  },
  lagoon: {
    id: 'lagoon', name: 'Coral Lagoon', blurb: 'A figure-eight over and under a tropical lagoon: a seabed run beneath the bridge, a glider leap off the sea cliffs and a rail-less boardwalk shortcut.',
    points: [
      [460, 175, 2], [380, 195, 2], [300, 200, 2], [235, 175, 2],
      // Dive: the coast road slopes under the lagoon and crosses the seabed beneath the bridge.
      [175, 125, 0], [110, 75, -5], [40, 22, -9], [-30, -30, -9], [-100, -82, -5], [-160, -128, 0], [-215, -170, 2],
      // West lobe: dune jump, banked jungle bend; the boardwalk cuts across the cove.
      [-280, -205, 3], [-360, -215, 4], [-440, -190, 5], [-505, -130, 6], [-535, -50, 6], [-525, 35, 5], [-485, 110, 4], [-415, 165, 3], [-335, 190, 3], [-260, 175, 4],
      // The bridge back across the lagoon, over the seabed road.
      [-200, 140, 6], [-120, 85, 8], [-40, 28, 9], [40, -30, 9], [120, -88, 8], [190, -135, 6],
      // East lobe: climb onto the sea cliffs, glide across the cove, sweep home.
      [255, -175, 8], [335, -205, 13], [420, -200, 18], [490, -160, 22], [535, -95, 23], [555, -20, 15], [560, 55, 5], [535, 125, 3],
    ],
    width: 17, edge: 'grass', outOfBounds: 15,
    itemRows: [0.1, 0.4, 0.62, 0.86], boostPads: [{ at: 0.05, lane: 0 }, { at: 0.2, lane: 0.4 }, { at: 0.47, lane: -0.4 }, { at: 0.68, lane: 0 }],
    ramps: [0.36], ice: [],
    launches: [{ at: 0.902, power: 10, glide: true }],
    gaps: [[0.905, 0.945]],
    bridges: [{ from: 0.575, to: 0.745, rails: true }],
    offroad: [{ from: 0.3, to: 0.33, lanes: [0.35, 1] }],
    banked: [[0.42, 0.52]],
    hills: 10, theme: 'lagoon',
    liquid: { level: 0, kind: 'water' },
    pools: [[0, 0, 230], [-410, -10, 105], [445, -40, 95]],
    branches: [{
      name: 'Boardwalk', from: 0.36, to: 0.535, shortcut: true, cpu: [0.1, 0.35, 0.6],
      width: 10, edge: 'grass', outOfBounds: 8,
      points: [[-470, -110, 3], [-440, -30, 2.5], [-420, 60, 2.5], [-395, 140, 3]],
      bridges: [{ from: 0.16, to: 0.86, rails: false }],
      boostPads: [{ at: 0.42, lane: 0 }],
      itemRows: [0.62],
    }],
  },
  volcano: {
    id: 'volcano', name: 'Magma Peak', blurb: 'Jump the lava river, corkscrew up the cinder tower, dodge the crater-tunnel crushers and glide off the summit. Rail-less ridge shortcut for the brave.',
    points: [
      [300, 120, 1], [300, 40, 2], [305, -20, 3],
      // Lava river: boulders roll across the approach, then a launch ramp over the flow.
      [315, -80, 4], [320, -140, 5], [320, -200, 5], [320, -250, 6],
      // Cinder tower: a banked corkscrew climbing two and a quarter turns.
      ...arc(275, -300, 45, 0, -810, 7, 37, 30),
      // Ridge to the crater rim, crusher tunnel through it, out onto the summit.
      [200, -350, 39], [130, -330, 42], [70, -290, 44], [10, -240, 45], [-40, -190, 45], [-80, -130, 45], [-105, -70, 45],
      // Summit launch: glide down across the lava fields.
      [-120, -20, 44], [-145, 50, 30], [-175, 110, 14], [-210, 160, 6],
      // Lava lake: the main road loops round it, the ridge cuts straight over.
      [-250, 210, 4], [-310, 260, 4], [-330, 330, 4], [-290, 395, 4], [-210, 420, 4], [-130, 400, 4], [-60, 360, 3],
      [20, 330, 2], [110, 300, 1], [200, 250, 1], [270, 190, 1],
    ],
    width: 16, edge: 'grass', outOfBounds: 14,
    itemRows: [0.03, 0.3, 0.42, 0.86], boostPads: [{ at: 0.06, lane: 0 }, { at: 0.36, lane: 0.35 }, { at: 0.7, lane: -0.35 }, { at: 0.9, lane: 0.4 }],
    ramps: [0.75], ice: [],
    launches: [{ at: 0.075, power: 14 }, { at: 0.555, power: 8, glide: true }],
    gaps: [[0.078, 0.088], [0.558, 0.63]],
    tunnels: [{ from: 0.45, to: 0.53 }],
    banked: [[0.11, 0.33]],
    obstacles: [
      { at: 0.105, kind: 'roller', period: 3.4 }, { at: 0.125, kind: 'roller', period: 2.9, phase: 0.45 },
      { at: 0.465, kind: 'crusher', period: 3.2, lane: -0.55 }, { at: 0.485, kind: 'crusher', period: 3.2, phase: 0.33, lane: 0.55 }, { at: 0.505, kind: 'crusher', period: 3.2, phase: 0.66, lane: -0.55 },
    ],
    hills: 14, theme: 'volcano',
    liquid: { level: -1, kind: 'lava' },
    pools: [[-230, 320, 95], [330, -110, 60]],
    branches: [{
      name: 'Obsidian Ridge', from: 0.65, to: 0.82, shortcut: true, cpu: [0.05, 0.3, 0.55],
      width: 9, edge: 'grass', outOfBounds: 8,
      points: [[-225, 235, 4], [-185, 285, 4], [-215, 335, 4], [-175, 385, 4]],
      bridges: [{ from: 0.1, to: 0.9, rails: false }],
    }],
  },
  skyway: {
    id: 'skyway', name: 'Neon Skyway', blurb: 'Sunset city loop: a broken-freeway jump, a corkscrew garage ramp, an old-town alley shortcut and a glass tunnel under the harbour.',
    points: [
      [360, 96, 1], [360, 0, 1], [358, -72, 3],
      // Freeway on-ramp and the elevated freeway with its broken section.
      [342, -156, 7], [294, -234, 11], [210, -270, 12], [120, -276, 12], [36, -276, 12], [-48, -274, 12], [-120, -270, 12],
      // Corkscrew garage ramp down to the old town.
      ...arc(-180, -210, 50, -90, -540, 12, 1, 30),
      // Old town S-bends (the back alley cuts straight through).
      [-240, -130, 1], [-300, -80, 1], [-335, 0, 1], [-310, 80, 1], [-255, 125, 1],
      // Glass tunnel under the harbour.
      [-195, 150, -1], [-130, 190, -6], [-50, 220, -9], [40, 225, -9], [120, 210, -6], [190, 190, -1],
      [270, 192, 1], [335, 160, 1],
    ],
    width: 16, edge: 'wall', outOfBounds: 0,
    itemRows: [0.03, 0.3, 0.6, 0.83], boostPads: [{ at: 0.06, lane: -0.4 }, { at: 0.19, lane: 0 }, { at: 0.72, lane: 0 }, { at: 0.95, lane: 0.4 }],
    ramps: [0.66], ice: [],
    launches: [{ at: 0.232, power: 16 }],
    gaps: [[0.236, 0.252]],
    tunnels: [{ from: 0.73, to: 0.9, glass: true }],
    banked: [[0.3, 0.46]],
    hills: 0, theme: 'dusk',
    liquid: { level: 0, kind: 'water' },
    pools: [[-20, 250, 170]],
    branches: [{
      name: 'Back Alley', from: 0.555, to: 0.715, shortcut: true, cpu: [0.15, 0.4, 0.65],
      width: 10, edge: 'wall',
      points: [[-240, -70, 1], [-243, 0, 1], [-250, 70, 1]],
      offroad: [{ from: 0.2, to: 0.8 }],
      boostPads: [{ at: 0.08, lane: 0 }],
      obstacles: [{ at: 0.4, kind: 'roller', period: 2.6 }, { at: 0.62, kind: 'roller', period: 3.0, phase: 0.5 }],
    }],
  },
};
export const TRACK_IDS = Object.keys(TRACK_DEFS) as TrackId[];

// ── Sample flags ────────────────────────────────────────────────────────────
export const F_BRIDGE = 1, F_RAILS = 2, F_GAP = 4, F_TUNNEL = 8, F_UNDER = 16, F_ICE = 32, F_OFFROAD = 64, F_BANK = 128, F_GLASS = 256;
/** Below a bridge or gap the ground drops this far (no liquid). */
export const CHASM = 18;
/** Tunnel ceiling above the road. */
export const TUNNEL_HEIGHT = 7;
/** Railed roads (bridges, tunnels) carry a hard shoulder this wide out to the rail. */
export const RAIL_SHOULDER = 1.6;

export interface Path {
  /** 0 = main loop, k = branch k (1-based). */
  index: number; name: string; closed: boolean;
  count: number; length: number;
  /** Per sample: position, unit tangent (xz), distance along this path. */
  px: Float64Array; py: Float64Array; pz: Float64Array;
  tx: Float64Array; tz: Float64Array; dist: Float64Array;
  /** Signed curvature per sample (rad/m, + = left turn). */
  curve: Float64Array;
  /** Road cross slope per sample: height change per metre of lateral offset is `-bank` (+ raises the right-hand side). */
  bank: Float64Array;
  /** 0..1: how far the ground beside the road falls away (bridges, gaps), eased at the ends. */
  drop: Float32Array;
  flags: Uint16Array;
  halfWidth: number; edge: EdgeKind; outOfBounds: number;
  /** Main-loop distances where this path forks off and merges back (main loop: 0 and its length). */
  from: number; to: number;
  features: Features;
  shortcut: boolean;
  cpu: [number, number, number];
}
export interface Obstacle { seg: number; s: number; kind: ObstacleKind; period: number; phase: number; lane: number }
export interface Track extends Path {
  def: TrackDef;
  /** paths[0] is the track itself (the main loop); then one per branch. */
  paths: Path[];
  liquid: number | null;
  lava: boolean;
  obstacles: Obstacle[];
  /** Height a racer drives on at (x, z), measured from its nearest centre-line point. */
  ground: (x: number, z: number, near: Near) => number;
  /** Landscape height (for the renderer): the road bed on the road, the land below bridge decks and gaps. */
  terrain: (x: number, z: number, near: Near) => number;
}
export interface Near { seg: number; index: number; t: number; s: number; lateral: number; y: number; tx: number; tz: number; distance: number }
export const newNear = (): Near => ({ seg: 0, index: 0, t: 0, s: 0, lateral: 0, y: 0, tx: 0, tz: 1, distance: 0 });

/** Centripetal Catmull-Rom point between p1 and p2. */
function catmull(p0: number[], p1: number[], p2: number[], p3: number[], t: number, out: number[]): void {
  const d = (a: number[], b: number[]) => Math.max(1e-4, Math.sqrt(Math.hypot(b[0] - a[0], b[1] - a[1], b[2] - a[2])));
  const t0 = 0, t1 = t0 + d(p0, p1), t2 = t1 + d(p1, p2), t3 = t2 + d(p2, p3);
  const u = t1 + (t2 - t1) * t;
  for (let k = 0; k < 3; k++) {
    const a1 = ((t1 - u) * p0[k] + (u - t0) * p1[k]) / (t1 - t0);
    const a2 = ((t2 - u) * p1[k] + (u - t1) * p2[k]) / (t2 - t1);
    const a3 = ((t3 - u) * p2[k] + (u - t2) * p3[k]) / (t3 - t2);
    const b1 = ((t2 - u) * a1 + (u - t0) * a2) / (t2 - t0);
    const b2 = ((t3 - u) * a2 + (u - t1) * a3) / (t3 - t1);
    out[k] = ((t2 - u) * b1 + (u - t1) * b2) / (t2 - t1);
  }
}

/** Jump ramps rise over the RAMP_LENGTH metres before their lip. */
export const RAMP_LENGTH = 8;
export const RAMP_HEIGHT = 1.4;
/** Launch ramps (and glider pads) are longer and taller. */
export const LAUNCH_LENGTH = 14;
export const LAUNCH_HEIGHT = 2.6;
export function rampHeight(def: TrackDef, length: number, s: number): number {
  for (const at of def.ramps) {
    const d = at * length - s;
    if (d >= 0 && d < RAMP_LENGTH) return RAMP_HEIGHT * (1 - d / RAMP_LENGTH);
  }
  return 0;
}
/** Kicker or launch ramp height on any path at local distance `s`. */
export function rampOn(path: Path, s: number): number {
  for (const at of path.features.ramps) {
    const d = at * path.length - s;
    if (d >= 0 && d < RAMP_LENGTH) return RAMP_HEIGHT * (1 - d / RAMP_LENGTH);
  }
  for (const l of path.features.launches) {
    const d = l.at * path.length - s;
    if (d >= 0 && d < LAUNCH_LENGTH) return LAUNCH_HEIGHT * (1 - d / LAUNCH_LENGTH);
  }
  return 0;
}

interface Line { count: number; length: number; px: Float64Array; py: Float64Array; pz: Float64Array; tx: Float64Array; tz: Float64Array; dist: Float64Array; curve: Float64Array }
/**
 * Samples a centre line every ~2 m. `ctrl` is [x, y, z]; a closed line wraps,
 * an open one runs from ctrl[1] to ctrl[n-2] (the ends only guide the tangent).
 */
function sampleLine(ctrl: number[][], closed: boolean): Line {
  const m = ctrl.length;
  const dense: number[][] = [];
  const out = [0, 0, 0];
  const first = closed ? 0 : 1, last = closed ? m : m - 2;
  for (let i = first; i < last; i++) {
    const at = (k: number) => ctrl[closed ? (k + m) % m : clamp(k, 0, m - 1)];
    for (let k = 0; k < 60; k++) { catmull(at(i - 1), at(i), at(i + 1), at(i + 2), k / 60, out); dense.push([out[0], out[1], out[2]]); }
  }
  if (!closed) dense.push([...ctrl[m - 2]]);
  const segs = closed ? dense.length : dense.length - 1;
  const cum = [0];
  for (let i = 1; i <= segs; i++) {
    const a = dense[i - 1], b = dense[i % dense.length];
    cum.push(cum[i - 1] + Math.hypot(b[0] - a[0], b[2] - a[2]));
  }
  const length = cum[segs];
  const count = closed ? Math.round(length / 2) : Math.round(length / 2) + 1;
  const spacing = closed ? length / count : length / (count - 1);
  const px = new Float64Array(count), py = new Float64Array(count), pz = new Float64Array(count);
  const tx = new Float64Array(count), tz = new Float64Array(count), dist = new Float64Array(count), curve = new Float64Array(count);
  let j = 0;
  for (let i = 0; i < count; i++) {
    const s = Math.min(length, i * spacing);
    while (j < segs - 1 && cum[j + 1] < s) j++;
    const a = dense[j], b = dense[(j + 1) % dense.length];
    const f = clamp((s - cum[j]) / Math.max(1e-6, cum[j + 1] - cum[j]), 0, 1);
    px[i] = a[0] + (b[0] - a[0]) * f; py[i] = a[1] + (b[1] - a[1]) * f; pz[i] = a[2] + (b[2] - a[2]) * f;
    dist[i] = s;
  }
  const idx = (i: number) => (closed ? (i + count) % count : clamp(i, 0, count - 1));
  for (let i = 0; i < count; i++) {
    const n = idx(i + 1), p = idx(i - 1);
    const dx = px[n] - px[p], dz = pz[n] - pz[p], l = Math.hypot(dx, dz) || 1;
    tx[i] = dx / l; tz[i] = dz / l;
  }
  for (let i = 0; i < count; i++) {
    const n = idx(i + 3), p = idx(i - 3), span = closed ? 6 : n - p;
    const a0 = Math.atan2(tx[p], tz[p]), a1 = Math.atan2(tx[n], tz[n]);
    let d = a1 - a0;
    while (d > Math.PI) d -= Math.PI * 2;
    while (d < -Math.PI) d += Math.PI * 2;
    curve[i] = span > 0 ? d / (span * spacing) : 0;
  }
  return { count, length, px, py, pz, tx, tz, dist, curve };
}

const FEATURE_KEYS = ['itemRows', 'boostPads', 'ramps', 'ice', 'launches', 'offroad', 'bridges', 'gaps', 'tunnels', 'banked', 'obstacles'] as const;
function features(src: Partial<Features>): Features {
  const f = {} as Record<(typeof FEATURE_KEYS)[number], unknown>;
  for (const k of FEATURE_KEYS) f[k] = src[k] ?? [];
  return f as unknown as Features;
}

function makePath(index: number, name: string, line: Line, closed: boolean, opts: { width: number; edge: EdgeKind; outOfBounds: number; features: Features; from: number; to: number; cpu: [number, number, number]; shortcut: boolean }): Path {
  const { count } = line;
  const flags = new Uint16Array(count);
  const f = opts.features;
  const inside = (frac: number, a: number, b: number) => frac >= a && frac <= b;
  for (let i = 0; i < count; i++) {
    const frac = line.dist[i] / line.length;
    let v = 0;
    for (const [a, b] of f.ice) if (inside(frac, a, b)) v |= F_ICE;
    for (const o of f.offroad) if (inside(frac, o.from, o.to)) v |= F_OFFROAD;
    for (const b of f.bridges) if (inside(frac, b.from, b.to)) v |= F_BRIDGE | (b.rails ? F_RAILS : 0);
    for (const [a, b] of f.gaps) if (inside(frac, a, b)) v |= F_GAP;
    for (const t of f.tunnels) if (inside(frac, t.from, t.to)) v |= F_TUNNEL | F_RAILS | (t.glass ? F_GLASS : 0);
    for (const [a, b] of f.banked) if (inside(frac, a, b)) v |= F_BANK;
    flags[i] = v;
  }
  const bank = new Float64Array(count), drop = new Float32Array(count);
  return {
    index, name, closed, ...line, bank, drop, flags,
    halfWidth: opts.width / 2, edge: opts.edge, outOfBounds: opts.outOfBounds,
    from: opts.from, to: opts.to, features: f, shortcut: opts.shortcut, cpu: opts.cpu,
  };
}

/** Smooths a per-sample array with a box filter of radius `r` (wrapping on closed paths). */
function blur(path: Path, src: Float64Array, r: number): Float64Array {
  const n = path.count, out = new Float64Array(n);
  for (let i = 0; i < n; i++) {
    let sum = 0, w = 0;
    for (let k = -r; k <= r; k++) {
      const j = path.closed ? (i + k + n) % n : i + k;
      if (j < 0 || j >= n) continue;
      sum += src[j]; w++;
    }
    out[i] = sum / w;
  }
  return out;
}

/** Marks stretches that pass above another stretch (overpasses, stacked spiral turns) as railed bridges. */
function markOverpasses(paths: Path[]): void {
  for (const p of paths) for (const q of paths) {
    // Roads overlapping (or nearly touching) in plan, one well above the other.
    const reach = p.halfWidth + q.halfWidth + 3;
    for (let i = 0; i < p.count; i++) {
      const x = p.px[i], z = p.pz[i], y = p.py[i];
      for (let j = 0; j < q.count; j++) {
        if (y - q.py[j] < 4.5) continue;
        const dx = q.px[j] - x;
        if (dx > reach || dx < -reach) continue;
        const dz = q.pz[j] - z;
        if (dz > reach || dz < -reach || dx * dx + dz * dz > reach * reach) continue;
        if (p === q) {
          const gap = Math.abs(i - j);
          if (Math.min(gap, p.closed ? p.count - gap : gap) < 30) continue;
        }
        p.flags[i] |= F_BRIDGE | F_RAILS;
        break;
      }
    }
  }
}

const cache = new Map<TrackId, Track>();
export function trackFor(id: string): Track {
  const def = TRACK_DEFS[(id in TRACK_DEFS ? id : 'meadow') as TrackId];
  const hit = cache.get(def.id);
  if (hit) return hit;
  const mainLine = sampleLine(def.points.map(([x, z, y]) => [x, y, z]), true);
  const main = makePath(0, def.name, mainLine, true, {
    width: def.width, edge: def.edge, outOfBounds: def.outOfBounds, features: features(def),
    from: 0, to: mainLine.length, cpu: [1, 1, 1], shortcut: false,
  });
  const paths: Path[] = [main];
  const pt = [0, 0, 0, 0];
  for (const b of def.branches ?? []) {
    const F = b.from * main.length, T = b.to * main.length;
    const at = (s: number): number[] => { pointAt(main, s, 0, pt); return [pt[0], pt[1], pt[2]]; };
    const ctrl = [at(F - 16), at(F), ...b.points.map(([x, z, y]) => [x, y, z]), at(T), at(T + 16)];
    paths.push(makePath(paths.length, b.name, sampleLine(ctrl, false), false, {
      width: b.width ?? def.width, edge: b.edge ?? def.edge, outOfBounds: b.outOfBounds ?? def.outOfBounds, features: features(b),
      from: F, to: T, cpu: b.cpu, shortcut: b.shortcut === true,
    }));
  }
  const liquid = def.liquid ? def.liquid.level : null, lava = def.liquid?.kind === 'lava';
  markOverpasses(paths);
  for (const p of paths) {
    const raw = new Float64Array(p.count), dropRaw = new Float64Array(p.count);
    for (let i = 0; i < p.count; i++) {
      // ~15° on a 60 m radius, steeper on the spirals.
      if (p.flags[i] & F_BANK) raw[i] = clamp(p.curve[i] * 16, -0.42, 0.42);
      if (p.flags[i] & (F_BRIDGE | F_GAP)) dropRaw[i] = 1;
      if (liquid !== null && !lava && p.py[i] < liquid + 0.5) p.flags[i] |= F_UNDER;
    }
    p.bank.set(blur(p, raw, 6));
    // The ground eases down over ~12 m before a bridge or gap.
    for (let i = 0; i < p.count; i++) {
      let d = 0;
      for (let k = -6; k <= 6; k++) {
        const j = p.closed ? (i + k + p.count) % p.count : i + k;
        if (j >= 0 && j < p.count && dropRaw[j] > 0) d = Math.max(d, 1 - Math.abs(k) / 7);
      }
      p.drop[i] = d;
    }
  }
  const obstacles: Obstacle[] = [];
  for (const p of paths) for (const o of p.features.obstacles) obstacles.push({ seg: p.index, s: o.at * p.length, kind: o.kind, period: o.period, phase: o.phase ?? 0, lane: o.lane ?? 0 });
  const noise = makeNoise(hashString(def.id));
  const pools = def.pools ?? [];
  const poolSink = (x: number, z: number): number => {
    let sink = 0;
    for (const [cx, cz, r] of pools) {
      const d = Math.hypot(x - cx, z - cz);
      if (d < r) sink = Math.max(sink, 1 - smoothstep(r * 0.55, r, d));
    }
    return sink;
  };
  const track = main as Track;
  track.def = def; track.paths = paths; track.liquid = liquid; track.lava = lava; track.obstacles = obstacles;
  /**
   * Ground beside the road `off` metres beyond its edge: the road edge height,
   * rising into themed hills (walls on walled tracks, rock either side of a
   * tunnel); in pools it sinks under the surface, and beside bridges and gaps
   * it falls away to the water, lava or a ravine.
   */
  const beside = (x: number, z: number, near: Near, p: Path, off: number): number => {
    const flags = p.flags[near.index] ?? 0, drop = p.drop[near.index] ?? 0;
    // Land meets the road edge on that side (lower or higher on a banked turn).
    const edge = near.y - Math.sign(near.lateral) * p.halfWidth * (p.bank[near.index] ?? 0);
    let natural: number;
    if (off <= 1) natural = edge;
    else {
      const rise = smoothstep(1, 70, off);
      const hill = (fbm(noise, x * 0.012, z * 0.012, 4) * 0.5 + 0.5) * def.hills;
      const wall = p.edge === 'wall' ? smoothstep(3, 14, off) * (def.theme === 'city' || def.theme === 'dusk' ? 0 : 10 + def.hills * 0.6) : 0;
      const cut = flags & F_TUNNEL && !(flags & F_GLASS) ? smoothstep(1, 8, off) * (TUNNEL_HEIGHT + 2) : 0;
      natural = edge + rise * hill + wall + cut;
    }
    // Lagoons and lava lakes: the land sinks below the surface a few metres off the road.
    const sink = poolSink(x, z);
    const wet = liquid !== null ? Math.min(near.y - 4, liquid - 5) : near.y - CHASM;
    if (sink > 0 && off > 2) natural += (Math.min(natural, wet) - natural) * sink * smoothstep(2, 7, off);
    if (drop <= 0) return natural;
    // Beside a bridge or gap: down to the water/lava inside a pool, else a ravine.
    const low = sink > 0.5 ? wet : near.y - CHASM;
    return natural + (low - natural) * drop;
  };
  track.ground = (x, z, near) => {
    const p = paths[near.seg] ?? main;
    const off = Math.abs(near.lateral) - p.halfWidth;
    const flags = p.flags[near.index] ?? 0;
    // On the road, and on a railed deck's shoulder out to the rail.
    if (off <= 0 || (off <= RAIL_SHOULDER && flags & F_RAILS && !(flags & F_GAP))) {
      if (flags & F_GAP) return near.y - CHASM * 3;
      return near.y + (off <= 0 ? rampOn(p, near.s) : 0) - near.lateral * (p.bank[near.index] ?? 0);
    }
    return beside(x, z, near, p, off);
  };
  track.terrain = (x, z, near) => {
    const p = paths[near.seg] ?? main;
    const off = Math.abs(near.lateral) - p.halfWidth;
    // Under a bridge deck or over a gap the land is what lies beside it.
    if ((p.flags[near.index] ?? 0) & (F_BRIDGE | F_GAP)) return beside(x, z, near, p, Math.max(off, 2.5));
    if (off <= 1) return near.y - clamp(near.lateral, -p.halfWidth, p.halfWidth) * (p.bank[near.index] ?? 0) - 0.35;
    return beside(x, z, near, p, off);
  };
  cache.set(def.id, track);
  return track;
}

const _near: Near = newNear();
const _cand: Near = newNear();
/**
 * Nearest centre-line point of `path`. With `hint >= 0` only a ±window of
 * samples is searched (racers move < 2 samples per tick); otherwise the whole
 * path. A finite `y` prefers stretches at that height (overpasses, spirals).
 * Writes into `out` (allocation-free in the hot loop).
 */
export function nearest(path: Path, x: number, z: number, hint = -1, out: Near = _near, window = 24, y = NaN): Near {
  const n = path.count, closed = path.closed;
  let from: number, to: number;
  if (hint >= 0) {
    from = hint - window; to = hint + window;
    if (!closed) { from = Math.max(0, from); to = Math.min(n - 2, to); }
  } else { from = 0; to = closed ? n - 1 : n - 2; }
  let best = Infinity, bestXZ = Infinity, bi = 0, bt = 0;
  for (let k = from; k <= to; k++) {
    const i = closed ? ((k % n) + n) % n : k, j = closed ? (i + 1) % n : i + 1;
    const ax = path.px[i], az = path.pz[i], bx = path.px[j] - ax, bz = path.pz[j] - az;
    const l2 = bx * bx + bz * bz || 1;
    const t = clamp(((x - ax) * bx + (z - az) * bz) / l2, 0, 1);
    const dx = x - (ax + bx * t), dz = z - (az + bz * t), xz = dx * dx + dz * dz;
    let d = xz;
    if (y === y) {
      const dy = Math.abs(y - (path.py[i] + (path.py[j] - path.py[i]) * t));
      if (dy > 4) d += (dy - 4) * (dy - 4) * 4;
    }
    if (d < best) { best = d; bestXZ = xz; bi = i; bt = t; }
  }
  const j = closed ? (bi + 1) % n : Math.min(n - 1, bi + 1);
  const cx = path.px[bi] + (path.px[j] - path.px[bi]) * bt, cz = path.pz[bi] + (path.pz[j] - path.pz[bi]) * bt;
  const tx = path.tx[bi], tz = path.tz[bi];
  out.seg = path.index; out.index = bi; out.t = bt;
  out.s = path.dist[bi] + (j === 0 ? path.length - path.dist[bi] : path.dist[j] - path.dist[bi]) * bt;
  // Facing (tx, tz) with yaw 0 = +Z, the left-hand side is (tz, -tx); lateral > 0 = left of travel.
  out.lateral = (x - cx) * tz - (z - cz) * tx;
  out.y = path.py[bi] + (path.py[j] - path.py[bi]) * bt;
  out.tx = tx; out.tz = tz; out.distance = Math.sqrt(bestXZ);
  return out;
}

/**
 * Selection score: how far outside the road edge (0 anywhere on the tarmac, so
 * a racer stays on its path while two roads overlap at a fork) plus a penalty
 * for being far above/below the road.
 */
const score = (n: Near, y: number, hw: number): number => Math.max(0, n.distance - hw) + (y === y ? Math.max(0, Math.abs(y - n.y) - 4) * 2 : 0);
const copyNear = (a: Near, b: Near): Near => {
  b.seg = a.seg; b.index = a.index; b.t = a.t; b.s = a.s; b.lateral = a.lateral; b.y = a.y; b.tx = a.tx; b.tz = a.tz; b.distance = a.distance;
  return b;
};
/** Metres either side of a fork or merge where a racer may switch between paths. */
export const FORK_ZONE = 40;
/**
 * Where a racer is: nearest point on its current path `seg`, switching to a
 * branch (or back to the main loop) inside fork/merge zones when that centre
 * line is clearly closer. `out.seg` names the path.
 */
export function locate(track: Track, x: number, y: number, z: number, seg: number, hint: number, out: Near = _near): Near {
  const paths = track.paths;
  const path = paths[seg] ?? track;
  nearest(path, x, z, hint, out, hint < 0 ? 0 : 24, y);
  if (paths.length === 1) return out;
  let best = score(out, y, path.halfWidth);
  const consider = (cand: Near) => {
    const sc = score(cand, y, paths[cand.seg].halfWidth);
    if (sc + 0.3 < best) { best = sc; copyNear(cand, out); }
  };
  if (path.index === 0) {
    const s = out.s, L = track.length;
    for (let b = 1; b < paths.length; b++) {
      const bp = paths[b];
      const df = circ(s - bp.from, L), dt = circ(s - bp.to, L);
      if (df > -6 && df < FORK_ZONE) consider(nearest(bp, x, z, 10, _cand, 12, y));
      if (dt > -FORK_ZONE && dt < 6) consider(nearest(bp, x, z, bp.count - 12, _cand, 12, y));
    }
  } else {
    const s = out.s;
    // Past either end of a branch the racer is back on the main loop, whatever the scores say.
    if (s <= 0.05 || s >= path.length - 0.05) best = Infinity;
    if (s < FORK_ZONE) consider(nearest(track, x, z, sampleAt(track, path.from), _cand, 22, y));
    if (s > path.length - FORK_ZONE) consider(nearest(track, x, z, sampleAt(track, path.to), _cand, 22, y));
  }
  return out;
}
/** Circular difference into (-L/2, L/2]. */
export function circ(d: number, L: number): number {
  d %= L;
  if (d > L / 2) d -= L;
  else if (d <= -L / 2) d += L;
  return d;
}

/**
 * Lap progress (main-loop distance) of local distance `s` on path `seg`. On a
 * branch it runs metre for metre through the fork and merge zones (where a
 * racer may switch paths, so progress never jumps) and stretches or squeezes
 * linearly in between.
 */
export function progressOf(track: Track, seg: number, s: number): number {
  if (seg === 0) return s;
  const p = track.paths[seg];
  if (!p) return s;
  const L = p.length, span = p.to - p.from, z = Math.min(FORK_ZONE, L / 3, span / 3);
  s = clamp(s, 0, L);
  if (s <= z) return p.from + s;
  if (s >= L - z) return p.to - (L - s);
  return p.from + z + (s - z) / (L - 2 * z) * (span - 2 * z);
}

export interface Spot { seg: number; s: number }
/**
 * Walks `d` metres ahead along a route: on the main loop, every branch whose
 * bit is set in `route` (bit k-1 = branch k) is taken at its fork; a branch
 * always continues onto the main loop at its merge.
 */
export function advance(track: Track, seg: number, s: number, d: number, route: number, out: Spot): Spot {
  let rest = d;
  for (let guard = 0; guard < 4; guard++) {
    if (seg !== 0) {
      const p = track.paths[seg];
      if (s + rest <= p.length) { s += rest; rest = 0; break; }
      rest -= p.length - s; seg = 0; s = p.to;
      continue;
    }
    let fork = -1, gap = Infinity;
    for (let b = 1; b < track.paths.length; b++) {
      if (!(route & (1 << (b - 1)))) continue;
      const g = circ(track.paths[b].from - s, track.length);
      if (g > -FORK_ZONE && g <= rest && g < gap) { gap = g; fork = b; }
    }
    if (fork < 0) { s = (((s + rest) % track.length) + track.length) % track.length; rest = 0; break; }
    // (A racer still counted on the main loop may already be up to FORK_ZONE metres into the branch.)
    rest -= gap; seg = fork; s = 0;
  }
  out.seg = seg; out.s = s;
  return out;
}

/** Point at distance `s` plus lateral offset; writes [x, y, z, yaw]. Open paths clamp `s` to their ends. */
export function pointAt(path: Path, s: number, lateral: number, out: number[] = [0, 0, 0, 0]): number[] {
  const L = path.length, n = path.count;
  let i: number, j: number, t: number;
  if (path.closed) {
    s = ((s % L) + L) % L;
    const f = (s / L) * n;
    i = Math.floor(f) % n; j = (i + 1) % n; t = f - Math.floor(f);
  } else {
    const f = (clamp(s, 0, L) / L) * (n - 1);
    i = Math.min(n - 2, Math.floor(f)); j = i + 1; t = f - i;
  }
  const x = path.px[i] + (path.px[j] - path.px[i]) * t, z = path.pz[i] + (path.pz[j] - path.pz[i]) * t;
  const tx = path.tx[i], tz = path.tz[i];
  out[0] = x + tz * lateral; out[2] = z - tx * lateral;
  out[1] = path.py[i] + (path.py[j] - path.py[i]) * t;
  out[3] = Math.atan2(tx, tz);
  return out;
}
export const sampleAt = (path: Path, s: number): number => {
  const L = path.length;
  if (!path.closed) return clamp(Math.floor((clamp(s, 0, L) / L) * (path.count - 1)), 0, path.count - 1);
  return Math.floor((((s % L) + L) % L) / L * path.count) % path.count;
};
/** Run-up before a launch ramp: rescues never drop a racer here (it could not reach take-off speed). */
export const RUN_UP = 60;
/** Whether local distance `s` lies in the run-up, on the ramp or over the gap of one of the path's launches. */
export function inLaunchZone(path: Path, s: number): boolean {
  for (const l of path.features.launches) {
    const d = l.at * path.length - s;
    if (d > -16 && d < RUN_UP) return true;
  }
  return (path.flags[sampleAt(path, s)] & F_GAP) !== 0;
}
/** Whether lateral offset `lateral` at local distance `s` lies in one of the path's off-road patches. */
export function inOffroad(path: Path, s: number, lateral: number): boolean {
  const frac = s / path.length;
  for (const o of path.features.offroad) {
    if (frac < o.from || frac > o.to) continue;
    if (!o.lanes) return true;
    const lane = lateral / path.halfWidth;
    if (lane >= o.lanes[0] && lane <= o.lanes[1]) return true;
  }
  return false;
}

/** Roller radius / crusher half-size and lift. */
export const ROLLER_RADIUS = 1.7;
export const CRUSHER_HALF = 2.4;
/** Raised crushers clear a kart and stay under a tunnel roof. */
export const CRUSHER_RISE = 3.8;
/**
 * Obstacle pose at race time `time`: writes [x, y, z, k] where k is the
 * crusher's lift above the road (0 = slammed down) or the roller's lateral
 * position (for spin). Pure: the host and every guest agree from the clock.
 */
export function obstacleAt(track: Track, o: Obstacle, time: number, out: number[]): number[] {
  const path = track.paths[o.seg] ?? track;
  const u = ((time / o.period + o.phase) % 1 + 1) % 1;
  if (o.kind === 'roller') {
    const lat = Math.sin(u * Math.PI * 2) * (path.halfWidth - 2);
    pointAt(path, o.s, lat, out);
    out[1] += ROLLER_RADIUS - lat * (path.bank[sampleAt(path, o.s)] ?? 0);
    out[3] = lat;
    return out;
  }
  // Crusher: waits up, slams down fast, rests, rises slowly.
  const lift = u < 0.6 ? CRUSHER_RISE : u < 0.66 ? CRUSHER_RISE * (1 - ((u - 0.6) / 0.06) ** 2) : u < 0.82 ? 0 : CRUSHER_RISE * smoothstep(0.82, 1, u);
  pointAt(path, o.s, o.lane * path.halfWidth * 0.5, out);
  out[3] = lift;
  return out;
}
