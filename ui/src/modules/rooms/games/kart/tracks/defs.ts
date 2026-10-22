// Kart track definitions: the authored control points and course furniture
// of every track (re-exported by ../tracks.ts).
import type { TrackId, P3, TrackDef } from './types.ts';

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
