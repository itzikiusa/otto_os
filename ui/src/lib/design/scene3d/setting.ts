// A Smart mode SETTING (environment.sky / fog / ground / room / weather and the
// point lights' glow, written by the productions scene writer) as plain
// renderer parameters: the ground's look by kind, the fog, the two walls of a
// room, the falling rain or snow, the halos. Pure (no three import), so the
// unit tests pin what a setting draws; `settingView.ts` builds it in three.
// Scene colours are scene data (the ground's paint), not UI chrome.
import type { GroundKind, Scene3dEnvironment, Scene3dLight, Vec3 } from './types';

export interface GroundLook {
  color: string;
  /** The texture's second colour: speckle, plank seams, grout, the road's verge. */
  detail: string;
  roughness: number;
  metalness: number;
  pattern: 'water' | 'speckle' | 'planks' | 'tiles' | 'road';
  /** Metres one repeat of the texture covers. */
  tile: number;
}

export const GROUND_LOOKS: Record<GroundKind, GroundLook> = {
  water: { color: '#1d4b5c', detail: '#2f6f80', roughness: 0.15, metalness: 0, pattern: 'water', tile: 12 },
  grass: { color: '#5f8a3a', detail: '#4a7330', roughness: 0.95, metalness: 0, pattern: 'speckle', tile: 2 },
  sand: { color: '#d8c39a', detail: '#c2ab82', roughness: 0.95, metalness: 0, pattern: 'speckle', tile: 2 },
  soil: { color: '#6e5a43', detail: '#57452f', roughness: 1, metalness: 0, pattern: 'speckle', tile: 2 },
  stone: { color: '#8d8a84', detail: '#74716b', roughness: 0.9, metalness: 0, pattern: 'tiles', tile: 2.4 },
  snow: { color: '#eef2f6', detail: '#d9e2ec', roughness: 0.85, metalness: 0, pattern: 'speckle', tile: 2 },
  wood_floor: { color: '#9a6b42', detail: '#7a522f', roughness: 0.7, metalness: 0, pattern: 'planks', tile: 1.6 },
  tiles: { color: '#dcd8cf', detail: '#aaa49a', roughness: 0.4, metalness: 0, pattern: 'tiles', tile: 1.2 },
  road: { color: '#b79f7a', detail: '#3b3d40', roughness: 0.95, metalness: 0, pattern: 'road', tile: 2 },
  meadow: { color: '#6f9a3e', detail: '#e8d84a', roughness: 0.95, metalness: 0, pattern: 'speckle', tile: 2 },
  moon_dust: { color: '#9a9894', detail: '#6f6d69', roughness: 1, metalness: 0, pattern: 'speckle', tile: 3 },
  sea_floor: { color: '#c9b98f', detail: '#a89772', roughness: 1, metalness: 0, pattern: 'speckle', tile: 2 },
  carpet: { color: '#8a5a6a', detail: '#74485a', roughness: 1, metalness: 0, pattern: 'speckle', tile: 1 },
  stage: { color: '#6b4426', detail: '#4f3019', roughness: 0.6, metalness: 0, pattern: 'planks', tile: 2.4 },
};

/** How a body in the sky is drawn: its colours (centre, edge), its size (degrees across), and whether it shines. */
export const SKY_BODY_LOOKS = {
  sun: { color: '#fff6d8', edge: '#ffd27a', size: 6, shines: true },
  moon: { color: '#eceae2', edge: '#b9b6ad', size: 6, shines: false },
  earth: { color: '#4f8fe0', edge: '#e8f2ff', size: 9, shines: false },
  planet: { color: '#d9a066', edge: '#a86a3c', size: 7, shines: false },
} as const;

/** The asphalt strip of a road ground: this wide, along x through the ground's centre. */
export const ROAD_WIDTH = 7;
/** A room's walls: plaster, or a theatre's red curtains round a stage. */
export function wallColor(kind: GroundKind | undefined): string {
  return kind === 'stage' ? '#6a1420' : '#e6dccb';
}

export function groundLook(kind: GroundKind): GroundLook {
  return GROUND_LOOKS[kind] ?? GROUND_LOOKS.soil;
}

/** The fog three draws (FogExp2), none when the setting has none. */
export function fogOf(env: Scene3dEnvironment | undefined): { color: string; density: number } | null {
  return env?.fog && env.fog.density > 0 ? env.fog : null;
}

export interface Wall {
  /** Its centre. */
  position: Vec3;
  /** Its turn about y, radians (a plane faces +z unturned). */
  rotationY: number;
  width: number;
  height: number;
}

/** A room's two walls: on the floor's far sides from the camera, facing it. */
export function wallsOf(env: Scene3dEnvironment | undefined, camera: Vec3): Wall[] {
  const g = env?.ground;
  const h = env?.room?.height;
  if (!g || !h) return [];
  const [cx, cy, cz] = g.center;
  const [sx, sz] = g.size;
  const zSide = camera[2] >= cz ? -1 : 1;
  const xSide = camera[0] >= cx ? -1 : 1;
  return [
    { position: [cx, cy + h / 2, cz + (zSide * sz) / 2], rotationY: zSide < 0 ? 0 : Math.PI, width: sx, height: h },
    { position: [cx + (xSide * sx) / 2, cy + h / 2, cz], rotationY: xSide < 0 ? Math.PI / 2 : -Math.PI / 2, width: sz, height: h },
  ];
}

export interface Halo {
  id: string;
  position: Vec3;
  color: string;
  /** Radius, metres. */
  radius: number;
}

/** The halos: one at each visible point light that glows. */
export function halosOf(lights: Scene3dLight[]): Halo[] {
  return lights
    .filter((l) => l.type === 'point' && (l.glow ?? 0) > 0 && l.position && l.visible !== false)
    .map((l) => ({ id: l.id, position: l.position!, color: l.color ?? '#ffffff', radius: l.glow! }));
}

export interface Precipitation {
  kind: 'rain' | 'snow';
  count: number;
  /** Where it falls: a box around the group. */
  center: Vec3;
  size: Vec3;
  /** Metres a second downward. */
  speed: number;
}

/** The rain or snow falling round the group (at most 40 m across), none indoors or dry. */
export function precipitationOf(env: Scene3dEnvironment | undefined, target: Vec3): Precipitation | null {
  if (!env?.weather || env.room) return null;
  const across = Math.min(40, Math.max(10, env.ground ? Math.min(...env.ground.size) / 10 : 20));
  const center: Vec3 = [target[0], (env.ground?.center[1] ?? 0) + 7, target[2]];
  return env.weather === 'rain'
    ? { kind: 'rain', count: 1600, center, size: [across, 14, across], speed: 9 }
    : { kind: 'snow', count: 1400, center, size: [across, 14, across], speed: 0.9 };
}

/** What a rebuild depends on (everything a setting draws). */
export function settingKey(env: Scene3dEnvironment | undefined, lights: Scene3dLight[], camera: Vec3, target: Vec3): string {
  if (!env?.sky && !env?.ground && !env?.fog && !env?.local_effects?.length && !halosOf(lights).length) return '';
  const side = (k: number) => Math.sign(camera[k] - (env?.ground?.center[k] ?? 0));
  return JSON.stringify([env?.sky, env?.fog, env?.ground, env?.shore, env?.room, env?.weather, env?.local_effects, env?.sky_bodies, env?.stars, halosOf(lights), side(0), side(2), env?.weather ? target : null]);
}
