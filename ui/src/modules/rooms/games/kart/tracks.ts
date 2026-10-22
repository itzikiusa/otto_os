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
 * glide), tunnels, rails, ice, off-road patches, banking, drawbridges,
 * half-pipes and stretches that run under the water surface. Overpasses (a
 * stretch passing above another) are detected and railed automatically.
 * Everything that moves (rollers, crushers, drawbridge leaves) is a pure
 * function of the race clock, so guests animate it with no extra net data.
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
/**
 * Bascule drawbridge over [from, to]: two leaves hinged at the ends tilt up
 * together on a cycle. Up, the pair is a jump (crest it at speed); a racer
 * stopped in the opening between the tips drops into the water.
 */
export interface Drawbridge { from: number; to: number; period: number; phase?: number }
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
  drawbridges: Drawbridge[];
  /** Half-pipes: both edges curve up into riding walls that end in a trick lip at the range's end. */
  halfpipes: Range[];
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
  /** Only pays off with a boost: deep off-road a mushroom (or star) blasts through. CPUs take it only holding one. */
  boost?: boolean;
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
    id: 'meadow', name: 'Sunny Meadow', blurb: 'Rolling farmland, a hilltop jump and a lake crossing: race the drawbridge over the water or splash through the ford beneath it.',
    points: [
      [0, 0, 0], [100, 0, 0], [150, 20, 1], [170, 60, 3], [150, 100, 5], [110, 110, 6], [80, 140, 7], [90, 185, 8], [140, 200, 7], [200, 190, 5], [250, 210, 4],
      // South to the lake: the main road bows north over the drawbridge, the ford bows south under the water.
      [285, 262, 4], [295, 325, 3], [275, 380, 2], [230, 402, 1.5], [175, 385, 1.5], [120, 374, 1.5], [65, 378, 1.5], [15, 395, 1.5], [-20, 405, 2],
      [-60, 385, 3], [-70, 330, 5], [-55, 270, 6], [-35, 215, 7], [-20, 160, 8], [-50, 110, 6], [-100, 100, 4], [-130, 60, 2], [-100, 15, 0], [-50, 0, 0],
    ],
    width: 17, edge: 'grass', outOfBounds: 16, itemRows: [0.131, 0.44, 0.804], boostPads: [{ at: 0.253, lane: 0.35 }, { at: 0.668, lane: -0.35 }, { at: 0.918, lane: 0 }],
    ramps: [0.785], ice: [], hills: 9, theme: 'meadow',
    liquid: { level: -2.5, kind: 'water' },
    pools: [[105, 410, 80]],
    bridges: [{ from: 0.515, to: 0.598, rails: true }],
    drawbridges: [{ from: 0.545, to: 0.567, period: 9 }],
    branches: [{
      name: 'Ford', from: 0.47, to: 0.635, cpu: [0.3, 0.45, 0.5],
      points: [[180, 435, 0], [125, 448, -6], [70, 446, -6], [15, 424, -1]],
      itemRows: [0.5],
    }],
  },
  downtown: {
    id: 'downtown', name: 'Downtown Dash', blurb: 'Tight city blocks between towers, barrier-lined corners and a boulevard boost lane.',
    points: [
      [0, 0, 0], [120, 0, 0], [150, 15, 0], [160, 45, 0], [160, 90, 1], [140, 115, 2], [100, 120, 3], [80, 140, 4], [80, 190, 5], [100, 215, 5], [150, 220, 4], [175, 245, 3], [170, 290, 2], [130, 305, 1], [40, 305, 0], [0, 290, 0], [-20, 260, 0], [-20, 215, 1],
      // West side: the interchange. A climbing 270° loop flies back over its own entry road.
      [-50, 185, 1], [-90, 180, 1], [-130, 180, 1], ...arc(-150, 220, 40, -90, -360, 1, 12, 45).slice(1), [-110, 180, 12], [-112, 140, 9], [-114, 100, 4],
      [-110, 60, 1], [-90, 20, 0], [-50, 0, 0],
    ],
    width: 16, edge: 'wall', outOfBounds: 0, itemRows: [0.113, 0.34, 0.565], boostPads: [{ at: 0.04, lane: -0.4 }, { at: 0.291, lane: 0.3 }, { at: 0.48, lane: 0 }, { at: 0.887, lane: 0.4 }],
    ramps: [0.219], ice: [], hills: 0, theme: 'city',
    // The skate park on the boulevard: ride a wall off the lip for a trick.
    halfpipes: [[0.405, 0.455]],
    branches: [{
      name: 'Market Arcade', from: 0.145, to: 0.365, shortcut: true, boost: true, cpu: [0.4, 0.7, 0.9],
      width: 10, edge: 'wall',
      points: [[170, 120, 1.5], [185, 170, 2.5], [190, 215, 3]],
      offroad: [{ from: 0.35, to: 0.65 }],
    }],
  },
  canyon: {
    id: 'canyon', name: 'Canyon Run', blurb: 'Red-rock switchbacks, steep climbs and a big jump over the dry riverbed.',
    points: [
      [0, 0, 0], [90, -10, 2], [140, 10, 5], [160, 50, 8], [130, 90, 10], [80, 100, 11], [60, 130, 12], [80, 170, 13], [140, 175, 15], [190, 150, 17], [230, 170, 18],
      // East rim: out along the mesa, then the gorge launch over the dry river and down the far side.
      [285, 175, 18], [322, 205, 17], [332, 245, 16], [332, 300, 11], [312, 338, 9], [255, 348, 9], [195, 312, 10],
      [130, 275, 10], [70, 250, 9], [20, 270, 10], [-40, 260, 11], [-70, 220, 9], [-60, 170, 7], [-100, 140, 6], [-110, 90, 4], [-80, 40, 2], [-50, 10, 0],
    ],
    width: 16, edge: 'wall', outOfBounds: 0, itemRows: [0.148, 0.53, 0.81], boostPads: [{ at: 0.272, lane: 0 }, { at: 0.703, lane: 0.4 }, { at: 0.934, lane: -0.3 }],
    ramps: [0.338, 0.868], ice: [], hills: 22, theme: 'canyon',
    launches: [{ at: 0.452, power: 11 }],
    gaps: [[0.456, 0.476]],
    // The dry gulch: ride the walls off the end for a trick.
    halfpipes: [[0.6, 0.655]],
    branches: [{
      name: 'Mine Shaft', from: 0.755, to: 0.9, cpu: [0.3, 0.45, 0.55],
      width: 11, edge: 'wall',
      points: [[-75, 258, 10], [-118, 222, 8], [-135, 172, 7], [-132, 125, 5]],
      tunnels: [{ from: 0.14, to: 0.86 }],
      boostPads: [{ at: 0.22, lane: 0 }, { at: 0.8, lane: 0 }],
      obstacles: [
        { at: 0.36, kind: 'crusher', period: 2.8, lane: -0.5 }, { at: 0.5, kind: 'crusher', period: 2.8, phase: 0.4, lane: 0.5 }, { at: 0.64, kind: 'crusher', period: 2.8, phase: 0.75, lane: -0.5 },
      ],
    }],
  },
  frost: {
    id: 'frost', name: 'Frostbite Pass', blurb: 'Snowy pines, an icy lake crossing and a mountain-pass descent. Watch your grip.',
    points: [
      [0, 0, 0], [110, 0, 1], [160, -30, 3], [220, -30, 5], [260, 0, 7], [270, 50, 9], [240, 90, 11], [190, 100, 12], [160, 130, 12], [170, 180, 13], [220, 200, 14],
      // Glacier spiral: a banked helix winds down the mountain, passing under its own first turn.
      ...arc(275, 245, 40, -90, 360, 16, 3.5, 30),
      [312, 295, 3], [285, 335, 3.5], [225, 345, 5], [170, 318, 6], [110, 288, 5], [65, 262, 4], [35, 236, 3], [0, 240, 3],
      // West: the road skirts the frozen lake; the Thin Ice shortcut slides straight across it.
      [-50, 262, 3], [-110, 250, 4], [-150, 195, 4], [-155, 125, 3], [-135, 60, 2], [-95, 18, 1], [-50, 3, 0],
    ],
    width: 17, edge: 'grass', outOfBounds: 15, itemRows: [0.108, 0.318, 0.75], boostPads: [{ at: 0.202, lane: -0.3 }, { at: 0.653, lane: 0.3 }, { at: 0.94, lane: 0 }],
    ramps: [0.717], ice: [[0.62, 0.69]], hills: 16, theme: 'frost',
    banked: [[0.345, 0.54]],
    // The bobsled run out of the spiral: ride the ice walls off the lip.
    halfpipes: [[0.556, 0.6]],
    liquid: { level: -2.5, kind: 'water' },
    pools: [[-80, 150, 62]],
    branches: [{
      name: 'Thin Ice', from: 0.765, to: 0.93, shortcut: true, cpu: [0.15, 0.35, 0.55],
      width: 12, outOfBounds: 8,
      points: [[-75, 222, 2.5], [-82, 165, 2], [-84, 105, 2]],
      ice: [[0.42, 0.95]],
      bridges: [{ from: 0.18, to: 0.84, rails: false }],
      itemRows: [0.5],
    }],
  },
  lagoon: {
    id: 'lagoon', name: 'Coral Lagoon', blurb: 'A figure-eight over and under a tropical lagoon: a seabed run beneath the bridge, then the sea-cliff glider or the glass Reef Tunnel, and a rail-less boardwalk shortcut.',
    points: [
      [460, 175, 2], [380, 195, 2], [300, 200, 2], [235, 175, 2],
      // Dive: the coast road slopes under the lagoon and crosses the seabed beneath the bridge.
      [175, 125, 0], [110, 75, -5], [40, 22, -9], [-30, -30, -9], [-100, -82, -5], [-160, -128, 0], [-215, -170, 2],
      // West lobe: dune jump, banked jungle bend; the boardwalk cuts across the cove.
      [-280, -205, 3], [-360, -215, 4], [-440, -190, 5], [-505, -130, 6], [-535, -50, 6], [-525, 35, 5], [-485, 110, 4], [-415, 165, 3], [-335, 190, 3], [-260, 175, 4],
      // The bridge back across the lagoon, over the seabed road.
      [-200, 140, 6], [-120, 85, 8], [-40, 28, 9], [40, -30, 9], [120, -88, 8], [190, -135, 6],
      // East lobe: climb onto the sea cliffs, glide across the cove, sweep home.
      [255, -175, 8], [335, -205, 13], [420, -200, 18], [490, -160, 22], [535, -95, 23], [555, -20, 15], [560, 55, 5], [562, 125, 3], [530, 168, 2.5],
    ],
    width: 17, edge: 'grass', outOfBounds: 15,
    itemRows: [0.099, 0.396, 0.614, 0.851], boostPads: [{ at: 0.049, lane: 0 }, { at: 0.198, lane: 0.4 }, { at: 0.465, lane: -0.4 }, { at: 0.673, lane: 0 }],
    ramps: [0.338], ice: [],
    launches: [{ at: 0.893, power: 10, glide: true }],
    gaps: [[0.896, 0.935]],
    bridges: [{ from: 0.569, to: 0.737, rails: true }],
    offroad: [{ from: 0.297, to: 0.327, lanes: [0.35, 1] }],
    banked: [[0.416, 0.515]],
    hills: 10, theme: 'lagoon',
    liquid: { level: 0, kind: 'water' },
    pools: [[0, 0, 230], [-410, -10, 105], [445, -40, 110]],
    branches: [{
      name: 'Boardwalk', from: 0.356, to: 0.53, shortcut: true, cpu: [0.1, 0.35, 0.6],
      width: 10, edge: 'grass', outOfBounds: 8,
      points: [[-470, -110, 3], [-440, -30, 2.5], [-420, 60, 2.5], [-395, 140, 3]],
      bridges: [{ from: 0.16, to: 0.86, rails: false }],
      boostPads: [{ at: 0.42, lane: 0 }],
      itemRows: [0.62],
    }, {
      // Under the east cove in a glass tube, dodging rolling sea mines, instead of the cliff climb and the glider.
      name: 'Reef Tunnel', from: 0.76, to: 0.95, cpu: [0.3, 0.45, 0.5],
      width: 11, edge: 'wall',
      points: [[283, -161, 5], [345, -112, 1], [410, -60, -7], [470, -15, -8], [530, 60, -2]],
      tunnels: [{ from: 0.22, to: 0.78, glass: true }],
      obstacles: [{ at: 0.38, kind: 'roller', period: 3.1 }, { at: 0.52, kind: 'roller', period: 2.7, phase: 0.5 }, { at: 0.66, kind: 'roller', period: 3.4, phase: 0.25 }],
      itemRows: [0.2],
    }],
  },
  volcano: {
    id: 'volcano', name: 'Magma Peak', blurb: 'Jump the lava river, corkscrew up the cinder tower, dodge the crater-tunnel crushers, then glide off the summit or take the Switchback Trail down. Rail-less ridge shortcut for the brave.',
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
    // The crater tunnel ends well before the Switchback Trail forks off (its tube never spans the fork's mouth).
    tunnels: [{ from: 0.435, to: 0.488 }],
    banked: [[0.11, 0.33]],
    obstacles: [
      { at: 0.105, kind: 'roller', period: 3.4 }, { at: 0.125, kind: 'roller', period: 2.9, phase: 0.45 },
      { at: 0.447, kind: 'crusher', period: 3.2, lane: -0.55 }, { at: 0.462, kind: 'crusher', period: 3.2, phase: 0.33, lane: 0.55 }, { at: 0.477, kind: 'crusher', period: 3.2, phase: 0.66, lane: -0.55 },
    ],
    hills: 14, theme: 'volcano',
    liquid: { level: -1, kind: 'lava' },
    pools: [[-230, 320, 95], [330, -110, 60]],
    branches: [{
      // Off the summit by road instead of the glider: banked serpentine bends down the cinder slope, boulders rolling across.
      name: 'Switchback Trail', from: 0.5, to: 0.645, cpu: [0.3, 0.45, 0.5],
      width: 11, edge: 'grass', outOfBounds: 9,
      points: [[-31, -144, 45], [-42, -92, 43], [-62, -42, 39], [-76, 6, 34], [-82, 55, 27], [-120, 100, 16], [-165, 133, 8]],
      banked: [[0.1, 0.9]],
      boostPads: [{ at: 0.3, lane: 0 }, { at: 0.62, lane: 0 }],
      obstacles: [{ at: 0.46, kind: 'roller', period: 3.0 }, { at: 0.78, kind: 'roller', period: 3.3, phase: 0.4 }],
      itemRows: [0.15],
    }, {
      name: 'Obsidian Ridge', from: 0.665, to: 0.82, shortcut: true, cpu: [0.05, 0.3, 0.55],
      width: 9, edge: 'grass', outOfBounds: 8,
      points: [[-262, 262, 4], [-215, 300, 4], [-218, 345, 4], [-178, 388, 4]],
      // The deck starts once the ridge has left the main road's tarmac (and ends before it rejoins).
      bridges: [{ from: 0.17, to: 0.83, rails: false }],
    }],
  },
  skyway: {
    id: 'skyway', name: 'Neon Skyway', blurb: 'Sunset city loop: a broken-freeway jump, a corkscrew garage ramp, an old-town alley shortcut, then a glass tunnel under the harbour or the drawbridge over it.',
    points: [
      [360, 96, 1], [360, 0, 1], [358, -72, 3],
      // Freeway on-ramp and the elevated freeway with its broken section.
      [342, -156, 7], [294, -234, 11], [210, -270, 12], [120, -276, 12], [36, -276, 12], [-48, -274, 12], [-120, -270, 12],
      // Corkscrew garage ramp down to the old town.
      ...arc(-180, -210, 50, -90, -540, 12, 1, 30),
      // Old town S-bends (the back alley cuts straight through).
      [-240, -130, 1], [-300, -80, 1], [-335, 0, 1], [-310, 80, 1], [-255, 125, 1],
      // Glass tunnel under the harbour: the road holds its level until the Harbour Bridge has split off, then dives.
      [-195, 150, 1], [-160, 170, 0.3], [-120, 195, -6.2], [-50, 220, -9], [40, 225, -9], [120, 210, -6], [190, 190, -1],
      [270, 192, 1], [335, 160, 1],
    ],
    width: 16, edge: 'wall', outOfBounds: 0,
    itemRows: [0.03, 0.3, 0.6, 0.83], boostPads: [{ at: 0.06, lane: -0.4 }, { at: 0.19, lane: 0 }, { at: 0.72, lane: 0 }, { at: 0.95, lane: 0.4 }],
    ramps: [0.66], ice: [],
    launches: [{ at: 0.232, power: 16 }],
    gaps: [[0.236, 0.252]],
    tunnels: [{ from: 0.745, to: 0.9, glass: true }],
    banked: [[0.3, 0.46]],
    hills: 0, theme: 'dusk',
    liquid: { level: 0, kind: 'water' },
    // The harbour, and the inlet where the tunnel road dives and the bridge road leaves.
    pools: [[-20, 250, 170], [-160, 178, 45]],
    branches: [{
      name: 'Back Alley', from: 0.555, to: 0.715, shortcut: true, cpu: [0.15, 0.4, 0.65],
      width: 10, edge: 'wall',
      points: [[-240, -70, 1], [-243, 0, 1], [-250, 70, 1]],
      offroad: [{ from: 0.2, to: 0.8 }],
      boostPads: [{ at: 0.08, lane: 0 }],
      obstacles: [{ at: 0.4, kind: 'roller', period: 2.6 }, { at: 0.62, kind: 'roller', period: 3.0, phase: 0.5 }],
    }, {
      // Over the harbour instead of under it: time the drawbridge (or jump it as it lifts).
      name: 'Harbour Bridge', from: 0.728, to: 0.93, cpu: [0.35, 0.45, 0.5],
      width: 14, edge: 'wall',
      points: [[-168, 202, 1.5], [-100, 268, 6], [0, 300, 7], [100, 282, 7], [185, 232, 3]],
      bridges: [{ from: 0.14, to: 0.86, rails: true }],
      drawbridges: [{ from: 0.47, to: 0.535, period: 10, phase: 0.3 }],
      boostPads: [{ at: 0.25, lane: 0 }],
      itemRows: [0.7],
    }],
  },
};
export const TRACK_IDS = Object.keys(TRACK_DEFS) as TrackId[];

// ── Sample flags ────────────────────────────────────────────────────────────
export const F_BRIDGE = 1, F_RAILS = 2, F_GAP = 4, F_TUNNEL = 8, F_UNDER = 16, F_ICE = 32, F_OFFROAD = 64, F_BANK = 128, F_GLASS = 256, F_DRAW = 512, F_PIPE = 1024;
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
  /** Pays off only with a boost item (see BranchDef.boost). */
  boost: boolean;
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
  /** Height a racer drives on at (x, z), measured from its nearest centre-line point, at race time `time` (drawbridges). */
  ground: (x: number, z: number, near: Near, time?: number) => number;
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

/** Drawbridge leaves tilt up to this angle (radians). */
export const DRAW_ANGLE = 0.3;
/** Drawbridge cycle at race time `time`: 0 = down … 1 = fully up (down 45 %, rising 15 %, up 25 %, lowering 15 % of the period). */
export function drawLift(d: Drawbridge, time: number): number {
  const u = (((time / d.period + (d.phase ?? 0)) % 1) + 1) % 1;
  return u < 0.45 ? 0 : u < 0.6 ? smoothstep(0.45, 0.6, u) : u < 0.85 ? 1 : 1 - smoothstep(0.85, 1, u);
}
/**
 * Height of a drawbridge leaf above the road at local distance `s` and time
 * `time`: 0 off the bridge or while it is down, NaN in the opening between the
 * raised tips (each leaf, rotating about its hinge, pulls back from the middle).
 */
export function drawOn(path: Path, s: number, time: number): number {
  for (const d of path.features.drawbridges) {
    const a = d.from * path.length, b = d.to * path.length;
    if (s < a || s > b) continue;
    const lift = drawLift(d, time);
    if (lift <= 0) return 0;
    const th = DRAW_ANGLE * lift, half = (b - a) / 2;
    const dist = s < a + half ? s - a : b - s;
    if (dist > half * Math.cos(th)) return NaN;
    return dist * Math.tan(th);
  }
  return 0;
}
/** Half-pipe wall height at the road edge (the shoulder out to the rail stays at this height). */
export const PIPE_HEIGHT = 3.4;
/** Half-pipes ease in over this many metres and end abruptly in the lip. */
export const PIPE_ENTRY = 14;
/** Half-pipe wall height above the road at local distance `s`, lateral offset `lateral`. */
export function pipeOn(path: Path, s: number, lateral: number): number {
  for (const [f, t] of path.features.halfpipes) {
    const a = f * path.length, b = t * path.length;
    if (s < a || s > b) continue;
    const w = clamp((Math.abs(lateral) / path.halfWidth - 0.35) / 0.65, 0, 1);
    return PIPE_HEIGHT * w * w * smoothstep(a, a + PIPE_ENTRY, s);
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

const FEATURE_KEYS = ['itemRows', 'boostPads', 'ramps', 'ice', 'launches', 'offroad', 'bridges', 'gaps', 'tunnels', 'banked', 'obstacles', 'drawbridges', 'halfpipes'] as const;
function features(src: Partial<Features>): Features {
  const f = {} as Record<(typeof FEATURE_KEYS)[number], unknown>;
  for (const k of FEATURE_KEYS) f[k] = src[k] ?? [];
  return f as unknown as Features;
}

function makePath(index: number, name: string, line: Line, closed: boolean, opts: { width: number; edge: EdgeKind; outOfBounds: number; features: Features; from: number; to: number; cpu: [number, number, number]; shortcut: boolean; boost: boolean }): Path {
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
    for (const d of f.drawbridges) if (inside(frac, d.from, d.to)) v |= F_BRIDGE | F_RAILS | F_DRAW;
    for (const [a, b] of f.halfpipes) if (inside(frac, a, b)) v |= F_PIPE | F_RAILS;
    flags[i] = v;
  }
  const bank = new Float64Array(count), drop = new Float32Array(count);
  return {
    index, name, closed, ...line, bank, drop, flags,
    halfWidth: opts.width / 2, edge: opts.edge, outOfBounds: opts.outOfBounds,
    from: opts.from, to: opts.to, features: f, shortcut: opts.shortcut, boost: opts.boost, cpu: opts.cpu,
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

/**
 * Where a branch still shares tarmac with the main loop (the mouth of a fork
 * or merge), its road surface is the main road's: it may only climb or dive
 * once the two have pulled apart. Otherwise one strip runs under the other
 * and a racer swapping paths drops off (or into) a ledge.
 */
function matchForkHeights(main: Path, line: Line, from: number, to: number, halfWidth: number): void {
  const near = newNear(), touch = main.halfWidth + halfWidth;
  for (let i = 0; i < line.count; i++) {
    const s = line.dist[i];
    if (s > FORK_ZONE * 2.5 && s < line.length - FORK_ZONE * 2.5) continue;
    const hint = sampleAt(main, s < line.length / 2 ? from + s : to - (line.length - s));
    nearest(main, line.px[i], line.pz[i], hint, near, 40);
    const w = 1 - smoothstep(touch - 2, touch + 12, near.distance);
    if (w <= 0) continue;
    line.py[i] += (near.y - line.py[i]) * w;
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
    from: 0, to: mainLine.length, cpu: [1, 1, 1], shortcut: false, boost: false,
  });
  const paths: Path[] = [main];
  const pt = [0, 0, 0, 0];
  for (const b of def.branches ?? []) {
    const F = b.from * main.length, T = b.to * main.length;
    const at = (s: number): number[] => { pointAt(main, s, 0, pt); return [pt[0], pt[1], pt[2]]; };
    const ctrl = [at(F - 16), at(F), ...b.points.map(([x, z, y]) => [x, y, z]), at(T), at(T + 16)];
    const line = sampleLine(ctrl, false);
    matchForkHeights(main, line, F, T, (b.width ?? def.width) / 2);
    paths.push(makePath(paths.length, b.name, line, false, {
      width: b.width ?? def.width, edge: b.edge ?? def.edge, outOfBounds: b.outOfBounds ?? def.outOfBounds, features: features(b),
      from: F, to: T, cpu: b.cpu, shortcut: b.shortcut === true, boost: b.boost === true,
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
  track.ground = (x, z, near, time = 0) => {
    const p = paths[near.seg] ?? main;
    const off = Math.abs(near.lateral) - p.halfWidth;
    const flags = p.flags[near.index] ?? 0;
    // On the road, and on a railed deck's shoulder out to the rail.
    if (off <= 0 || (off <= RAIL_SHOULDER && flags & F_RAILS && !(flags & F_GAP))) {
      if (flags & F_GAP) return near.y - CHASM * 3;
      let lift = off <= 0 ? rampOn(p, near.s) : 0;
      if (flags & F_DRAW) {
        const leaf = drawOn(p, near.s, time);
        if (leaf !== leaf) return near.y - CHASM * 3; // the opening between raised leaves
        lift += leaf;
      }
      if (flags & F_PIPE) lift += pipeOn(p, near.s, near.lateral);
      return near.y + lift - near.lateral * (p.bank[near.index] ?? 0);
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
/** Whether local distance `s` lies in the run-up, on the ramp or over the gap of one of the path's launches (or on a drawbridge and its run-up). */
export function inLaunchZone(path: Path, s: number): boolean {
  for (const l of path.features.launches) {
    const d = l.at * path.length - s;
    if (d > -16 && d < RUN_UP) return true;
  }
  for (const d of path.features.drawbridges) if (s > d.from * path.length - RUN_UP && s < d.to * path.length + 8) return true;
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

/** Coins per line, metres between them, and the spacing of lines on the main loop / on branches. */
export const COIN_LINE = 5;
const COIN_GAP = 4, COIN_EVERY_MAIN = 150, COIN_EVERY_BRANCH = 70;
const LANES = [-0.45, 0.45, 0, -0.6, 0.6];
const coinCache = new Map<TrackId, Float64Array>();
/**
 * Coin positions, flat [x, y, z] per coin: lines of five down the road, one
 * every ~150 m of the main loop and every ~70 m of a branch (the riskier
 * routes pay better), clear of the start grid, forks and merges, jumps,
 * drawbridges, half-pipes, item rows and obstacles.
 */
export function coinSpots(track: Track): Float64Array {
  const hit = coinCache.get(track.def.id);
  if (hit) return hit;
  const out: number[] = [];
  const near = newNear(), pt = [0, 0, 0, 0];
  let line = 0;
  for (const p of track.paths) {
    const every = p.index === 0 ? COIN_EVERY_MAIN : COIN_EVERY_BRANCH, span = (COIN_LINE - 1) * COIN_GAP;
    const clear = (s: number): boolean => {
      if (p.index === 0) {
        if (s < 40 || s > p.length - 30) return false;
        for (const b of track.paths) if (b.index > 0 && (Math.abs(circ(s - b.from, p.length)) < FORK_ZONE + 12 || Math.abs(circ(s - b.to, p.length)) < FORK_ZONE + 12)) return false;
      } else if (s < FORK_ZONE + 8 || s > p.length - FORK_ZONE - 8) return false;
      if (inLaunchZone(p, s) || p.flags[sampleAt(p, s)] & (F_GAP | F_DRAW | F_PIPE)) return false;
      if (p.features.itemRows.some(a => Math.abs(a * p.length - s) < 14)) return false;
      if (p.features.ramps.some(a => Math.abs(a * p.length - s) < 12)) return false;
      return !track.obstacles.some(o => o.seg === p.index && Math.abs(o.s - s) < 14);
    };
    for (let s0 = every * 0.6; s0 + span < p.length; s0 += every) {
      let s = -1;
      for (let k = 0; k < 4 && s < 0; k++) { const c = s0 + k * 18; if (clear(c) && clear(c + span)) s = c; }
      if (s < 0) continue;
      const lane = LANES[line++ % LANES.length] * p.halfWidth;
      for (let k = 0; k < COIN_LINE; k++) {
        pointAt(p, s + k * COIN_GAP, lane, pt);
        nearest(p, pt[0], pt[2], sampleAt(p, s + k * COIN_GAP), near, 6);
        out.push(pt[0], track.ground(pt[0], pt[2], near) + 1.0, pt[2]);
      }
    }
  }
  const spots = Float64Array.from(out);
  coinCache.set(track.def.id, spots);
  return spots;
}
