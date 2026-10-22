/**
 * Time of day and weather presets for Storm Royale: clear day, dusk, night
 * (flashlights), rain, fog and ash (Dead City / the toxic cloud). Pure data:
 * `weatherTheme` turns a map's base stage theme into the preset's sky, fog,
 * sun and ambient light, and `weatherFor` picks the default for a map (or a
 * valid override). The renderer adds the particles and lights.
 */
/** The stage colours a preset overrides (GameView's Theme, kept structural so this module stays DOM-free for tests). */
export interface Theme {
  skyTop: number; skyHorizon: number; fog: number; fogNear: number; fogFar: number;
  sun: number; sunIntensity: number; ambientSky: number; ambientGround: number; ambient: number;
}

export type WeatherId = 'day' | 'dusk' | 'night' | 'rain' | 'fog' | 'ash';
export const WEATHERS: { id: WeatherId; name: string }[] = [
  { id: 'day', name: 'Clear day' }, { id: 'dusk', name: 'Dusk' }, { id: 'night', name: 'Night' },
  { id: 'rain', name: 'Rain' }, { id: 'fog', name: 'Fog' }, { id: 'ash', name: 'Ash storm' },
];
export interface WeatherPreset {
  /** Overrides on the map theme (unset: the map's own). */
  theme: Partial<Theme>;
  /** Sun direction (offset from the shadow focus), tone-mapping exposure. */
  sun: [number, number, number]; exposure: number;
  /** Particle densities 0..1, the local flashlight, lightning flashes. */
  rain: number; ash: number; flashlight: boolean; lightning: boolean;
  /** Storm / toxic wall tint (rgb 0..1). */
  wall: [number, number, number];
}
export const WEATHER: Record<WeatherId, WeatherPreset> = {
  day: { theme: {}, sun: [60, 110, 40], exposure: 1.05, rain: 0, ash: 0, flashlight: false, lightning: false, wall: [0.55, 0.3, 0.95] },
  dusk: {
    theme: { skyTop: 0x27365e, skyHorizon: 0xf2a066, fog: 0xc4876a, fogNear: 90, fogFar: 560, sun: 0xffa45e, sunIntensity: 2.2, ambientSky: 0xc4a0b0, ambientGround: 0x5a4638, ambient: 1.05 },
    sun: [120, 34, -30], exposure: 1.1, rain: 0, ash: 0, flashlight: false, lightning: false, wall: [0.75, 0.3, 0.7],
  },
  night: {
    theme: { skyTop: 0x03060f, skyHorizon: 0x172036, fog: 0x0d1322, fogNear: 25, fogFar: 240, sun: 0x8ea8dc, sunIntensity: 0.55, ambientSky: 0x3a4c7c, ambientGround: 0x10140f, ambient: 0.5 },
    sun: [-50, 120, 70], exposure: 1.25, rain: 0, ash: 0, flashlight: true, lightning: false, wall: [0.45, 0.35, 1],
  },
  rain: {
    theme: { skyTop: 0x46515e, skyHorizon: 0x8b959e, fog: 0x7c8790, fogNear: 35, fogFar: 300, sun: 0xd6dde6, sunIntensity: 0.95, ambientSky: 0xa9b4be, ambientGround: 0x3a3e3a, ambient: 1.0 },
    sun: [40, 120, 60], exposure: 1.05, rain: 1, ash: 0, flashlight: false, lightning: true, wall: [0.5, 0.35, 0.85],
  },
  fog: {
    theme: { skyTop: 0x98a2aa, skyHorizon: 0xc6cbcd, fog: 0xbfc5c7, fogNear: 10, fogFar: 150, sun: 0xf2f2ee, sunIntensity: 1.2, ambientSky: 0xdfe4e6, ambientGround: 0x6a6e66, ambient: 1.2 },
    sun: [60, 110, 40], exposure: 1.0, rain: 0, ash: 0, flashlight: false, lightning: false, wall: [0.55, 0.4, 0.85],
  },
  ash: {
    theme: { skyTop: 0x4a3a33, skyHorizon: 0xc39472, fog: 0x9e8270, fogNear: 40, fogFar: 360, sun: 0xffb27a, sunIntensity: 2.1, ambientSky: 0xdcc2ae, ambientGround: 0x6e5a4c, ambient: 1.35 },
    sun: [110, 80, -40], exposure: 1.15, rain: 0, ash: 1, flashlight: false, lightning: false, wall: [0.35, 0.85, 0.3],
  },
};
export const isWeather = (v: unknown): v is WeatherId => typeof v === 'string' && v in WEATHER;
/** The map's default weather (Dead City burns under ash), or `wanted` when it is a known preset. */
export function weatherFor(mapId: string, wanted?: unknown): WeatherId {
  if (isWeather(wanted)) return wanted;
  return mapId === 'deadcity' ? 'ash' : 'day';
}
/** The stage theme for a map under a weather preset. */
export function weatherTheme<T extends Theme>(base: T, id: WeatherId): T { return { ...base, ...WEATHER[id].theme }; }
