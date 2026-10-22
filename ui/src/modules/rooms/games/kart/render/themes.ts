/**
 * Per-theme look of a kart course: sky, sun and fog (GameView's Theme) plus
 * the ground palette, road tint, curb / barrier / banner colours, the liquid,
 * the off-road surface, clouds and stars, the crowd's shirts and (on the
 * neon skyway) the glow along the barrier tops.
 */
import type { Theme } from '../../common/view';
import type { SurfaceKind } from './surfaces';
import type { ThemeId } from '../tracks';

export interface KartTheme extends Theme {
  ground: number[]; water: number | null; road: string; edge: number;
  /** Colour of the track's liquid (lagoon water, lava) and of the land just below it. */
  liquid: number; bed: number;
  wall: [string, string]; obstacle: number;
  /** Sun position relative to the shadow focus (low and long-shadowed at dusk). */
  sunOffset: [number, number, number];
  /** Cloud tint, how much of the sky they cover (0..1), star strength, and the sky below the horizon. */
  cloud: number; cover: number; stars: number; skyGround: number;
  /** Shallow-water tint (the deep tint is `liquid`). */
  shallow: number;
  /** What off-road patches are made of. */
  offroad: SurfaceKind;
  /** Rumble curbs: plain and on banked bends. */
  curb: [string, string]; bankCurb: [string, string];
  /** Barrier banners: background, lettering, accent panel. */
  banner: [string, string, string];
  /** Glowing strip along the barrier tops, or null. */
  neon: number | null;
  /** Spectator shirt colours. */
  crowd: number[];
}

const CROWD = [0xd8322f, 0x2f6fd8, 0xf2c230, 0x3aa65a, 0xffffff, 0xff7a2a, 0x8a4fd8, 0x26b5c9, 0xf26fa8];
export const THEMES: Record<ThemeId, KartTheme> = {
  meadow: {
    skyTop: 0x2f7fd8, skyHorizon: 0xcfe9ff, fog: 0xd6ecff, fogNear: 200, fogFar: 760, sun: 0xfff3dc, sunIntensity: 2.7, ambientSky: 0xcfe6ff, ambientGround: 0x5d7a3a, ambient: 1.05,
    ground: [0x6cbf4a, 0x58a93c, 0x8fcd5c, 0x7bb84f], water: 0x3d9bd6, road: '#62666e', edge: 0x9bd36a, liquid: 0x1d78b8, bed: 0xd8c48a, wall: ['#d8322f', '#f4f4f4'], obstacle: 0x8a7a66,
    sunOffset: [70, 105, 30], cloud: 0xffffff, cover: 0.42, stars: 0, skyGround: 0x9fc87a, shallow: 0x55d6d0, offroad: 'mud',
    curb: ['#d8322f', '#f4f4f4'], bankCurb: ['#2f6fd8', '#f4f4f4'], banner: ['#1f5fbf', '#ffffff', '#f2c230'], neon: null, crowd: CROWD,
  },
  city: {
    skyTop: 0x3f86d6, skyHorizon: 0xd7e7f5, fog: 0xd9e4ef, fogNear: 170, fogFar: 680, sun: 0xfff1d6, sunIntensity: 2.6, ambientSky: 0xdde8f5, ambientGround: 0x666a70, ambient: 1.1,
    ground: [0x9a9ea6, 0x8e939b, 0xa4a8ae, 0x7fae5a], water: null, road: '#555a62', edge: 0xb9bdc4, liquid: 0x3d8fc0, bed: 0x6f7378, wall: ['#d8322f', '#f4f4f4'], obstacle: 0xd8322f,
    sunOffset: [60, 110, 40], cloud: 0xffffff, cover: 0.3, stars: 0, skyGround: 0x8e939b, shallow: 0x5fc6d0, offroad: 'mud',
    curb: ['#d8322f', '#f4f4f4'], bankCurb: ['#2f6fd8', '#f4f4f4'], banner: ['#d8322f', '#ffffff', '#202020'], neon: null, crowd: CROWD,
  },
  canyon: {
    skyTop: 0x2f6fc0, skyHorizon: 0xf7dcb4, fog: 0xf0d2a8, fogNear: 190, fogFar: 760, sun: 0xffe2b8, sunIntensity: 2.9, ambientSky: 0xffe7c9, ambientGround: 0x9a5a35, ambient: 1.0,
    ground: [0xd9955a, 0xc8763f, 0xe0ac70, 0xb86534], water: null, road: '#6e625b', edge: 0xe7b47a, liquid: 0x3d8fc0, bed: 0xb86534, wall: ['#b5562c', '#e8c79a'], obstacle: 0x9a5a35,
    sunOffset: [-80, 95, 50], cloud: 0xfff6ea, cover: 0.22, stars: 0, skyGround: 0xd9955a, shallow: 0x5fc6d0, offroad: 'sand',
    curb: ['#b5402c', '#f4ead8'], bankCurb: ['#e0a030', '#f4ead8'], banner: ['#b5562c', '#fff3dc', '#2f6fc0'], neon: null, crowd: CROWD,
  },
  frost: {
    skyTop: 0x5f97d6, skyHorizon: 0xe8f2fc, fog: 0xe6eff9, fogNear: 160, fogFar: 660, sun: 0xffffff, sunIntensity: 2.3, ambientSky: 0xeaf3ff, ambientGround: 0x9fb3c8, ambient: 1.2,
    ground: [0xf3f7fc, 0xe2ebf5, 0xffffff, 0xd2deeb], water: null, road: '#646d78', edge: 0xffffff, liquid: 0x7fb6d9, bed: 0xd2deeb, wall: ['#2f6fd8', '#f4f4f4'], obstacle: 0xf4f8ff,
    sunOffset: [50, 70, -60], cloud: 0xf4f8ff, cover: 0.6, stars: 0, skyGround: 0xe2ebf5, shallow: 0xa8e0f0, offroad: 'snow',
    curb: ['#2f6fd8', '#f4f4f4'], bankCurb: ['#d8322f', '#f4f4f4'], banner: ['#2f6fd8', '#ffffff', '#d8322f'], neon: null, crowd: CROWD,
  },
  lagoon: {
    skyTop: 0x1f86e0, skyHorizon: 0xc9f1ff, fog: 0xc9eefc, fogNear: 220, fogFar: 860, sun: 0xfff1d0, sunIntensity: 2.9, ambientSky: 0xd2f3ff, ambientGround: 0x5f8f5a, ambient: 1.1,
    ground: [0x5fbf4f, 0x4aa844, 0x86d06a, 0xe9d7a0], water: null, road: '#6b6a68', edge: 0xf1e3b0, liquid: 0x0f7fb0, bed: 0xe8d49a, wall: ['#2aa7a1', '#f6efe0'], obstacle: 0x8b6a4a,
    sunOffset: [60, 115, 20], cloud: 0xffffff, cover: 0.35, stars: 0, skyGround: 0x7fd6e0, shallow: 0x46e0d0, offroad: 'sand',
    curb: ['#2aa7a1', '#f6efe0'], bankCurb: ['#ff7a5a', '#f6efe0'], banner: ['#2aa7a1', '#ffffff', '#ff7a5a'], neon: null, crowd: CROWD,
  },
  volcano: {
    skyTop: 0x2a1f30, skyHorizon: 0xd9744a, fog: 0x8a4a3a, fogNear: 150, fogFar: 620, sun: 0xffb27a, sunIntensity: 2.3, ambientSky: 0xffa07a, ambientGround: 0x3a2622, ambient: 1.0,
    ground: [0x4a3c3a, 0x3a2f2e, 0x5c4a44, 0x2c2423], water: null, road: '#44403f', edge: 0x6b5a52, liquid: 0xff5a1a, bed: 0x8a2a10, wall: ['#e0582a', '#2a2222'], obstacle: 0x4a3a34,
    sunOffset: [-90, 60, -40], cloud: 0x5a4040, cover: 0.65, stars: 0.25, skyGround: 0x3a2622, shallow: 0xff8a2a, offroad: 'cinder',
    curb: ['#e0582a', '#2a2222'], bankCurb: ['#ffb000', '#2a2222'], banner: ['#2a2222', '#ffb000', '#e0582a'], neon: 0xff6a1a, crowd: CROWD,
  },
  dusk: {
    skyTop: 0x1f2360, skyHorizon: 0xf29a72, fog: 0xc98a86, fogNear: 170, fogFar: 680, sun: 0xffa86a, sunIntensity: 2.1, ambientSky: 0xc8a8e8, ambientGround: 0x4a4458, ambient: 1.15,
    ground: [0x8e8a96, 0x7f7b88, 0x9c98a4, 0x6f9e5a], water: null, road: '#43454f', edge: 0xb0acb8, liquid: 0x2c6f9e, bed: 0x4a5a6a, wall: ['#ff4fa3', '#2f2f3a'], obstacle: 0xffb000,
    sunOffset: [-120, 38, 70], cloud: 0xf2a0a8, cover: 0.38, stars: 0.6, skyGround: 0x4a4458, shallow: 0x3fa0c0, offroad: 'mud',
    curb: ['#ff4fa3', '#f4f4f4'], bankCurb: ['#38b6ff', '#f4f4f4'], banner: ['#2f2f3a', '#ff4fa3', '#38b6ff'], neon: 0xff4fa3, crowd: CROWD,
  },
};
