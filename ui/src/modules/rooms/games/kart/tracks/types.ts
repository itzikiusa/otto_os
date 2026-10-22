// Kart track authoring types: track and branch definitions and their course
// features (re-exported by ../tracks.ts).

export type TrackId = 'meadow' | 'downtown' | 'canyon' | 'frost' | 'lagoon' | 'volcano' | 'skyway';
export type EdgeKind = 'grass' | 'wall';
export type ThemeId = 'meadow' | 'city' | 'canyon' | 'frost' | 'lagoon' | 'volcano' | 'dusk';
/** [from, to] as fractions of a path's length. */
export type Range = [number, number];
export type P3 = [number, number, number];
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
