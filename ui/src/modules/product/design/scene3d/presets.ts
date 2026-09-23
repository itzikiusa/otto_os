// scene3d v2 presets — physical materials and procedural environments, as
// DATA plus the pure mapping `doc material → every renderer field`. No three
// import here (and only type imports): the viewer, the GLB/USDZ exporters,
// the embed runtime and the unit tests all share it.
import type { EnvPresetId, MaterialPresetId, Scene3dMaterial } from './types';

// ── Materials ────────────────────────────────────────────────────────────────

export interface MaterialPresetDef {
  id: MaterialPresetId;
  label: string;
  /** One line for the preset grid's tooltip. */
  hint: string;
  /** The preset's defaults; explicit document fields win over these. */
  defaults: Omit<Scene3dMaterial, 'preset' | 'wireframe'>;
  /** Swatch colour for the grid when the object has no colour of its own. */
  swatch: string;
}

export const MATERIAL_PRESETS: readonly MaterialPresetDef[] = [
  {
    id: 'glossy-plastic',
    label: 'Glossy plastic',
    hint: 'Clear-coated, low roughness — cards, toys, product shells',
    defaults: { metalness: 0, roughness: 0.35, clearcoat: 1, clearcoat_roughness: 0.08 },
    swatch: '#6d5ef7',
  },
  {
    id: 'brushed-metal',
    label: 'Brushed metal',
    hint: 'Fully metallic with a soft sheen — chips, trims, hardware',
    defaults: { color: '#c7cad1', metalness: 1, roughness: 0.38 },
    swatch: '#b9bdc6',
  },
  {
    id: 'frosted-glass',
    label: 'Frosted glass',
    hint: 'Transmissive, blurred refraction — panels, bottles, UI glass',
    defaults: { color: '#f3f4ff', metalness: 0, roughness: 0.32, transmission: 1, ior: 1.45, thickness: 0.3 },
    swatch: '#dfe3f5',
  },
  {
    id: 'matte-paper',
    label: 'Matte paper',
    hint: 'Diffuse, no highlights — packaging, plinths, print',
    defaults: { metalness: 0, roughness: 0.95 },
    swatch: '#efe9dc',
  },
  {
    id: 'satin',
    label: 'Satin',
    hint: 'Soft sheen between matte and gloss — fabric, soft-touch',
    defaults: { metalness: 0, roughness: 0.55, sheen: 0.7, clearcoat: 0.15, clearcoat_roughness: 0.4 },
    swatch: '#c9b8e8',
  },
];

export function materialPreset(id: string | undefined | null): MaterialPresetDef | null {
  return MATERIAL_PRESETS.find((p) => p.id === id) ?? null;
}

/** Every field the renderer needs, filled. Colours are resolved `#rrggbb`. */
export interface ResolvedMaterial {
  color: string;
  metalness: number;
  roughness: number;
  opacity: number;
  transparent: boolean;
  emissive: string;
  emissiveIntensity: number;
  wireframe: boolean;
  clearcoat: number;
  clearcoatRoughness: number;
  transmission: number;
  ior: number;
  thickness: number;
  sheen: number;
  /** A physical material is only worth its cost when a physical field is on. */
  physical: boolean;
}

/** Renderer defaults for a material with no preset and no fields (the v1 look). */
export const MATERIAL_DEFAULTS = {
  color: '#94a3b8',
  metalness: 0.1,
  roughness: 0.7,
  emissive: '#000000',
} as const;

const clamp01 = (n: number): number => Math.min(1, Math.max(0, n));

/**
 * Doc material → renderer fields. Precedence: explicit field > preset default >
 * renderer default. `color(ref, fallback)` resolves hex / `token:` colours
 * (see `tokens.ts::resolveColor`).
 */
export function resolveMaterial(
  m: Scene3dMaterial | undefined,
  color: (ref: string | undefined, fallback: string) => string,
): ResolvedMaterial {
  const p = materialPreset(m?.preset)?.defaults ?? {};
  const pick = <K extends keyof Scene3dMaterial>(k: K): Scene3dMaterial[K] | undefined =>
    m?.[k] !== undefined ? m[k] : (p as Scene3dMaterial)[k];
  const opacity = clamp01(pick('opacity') ?? 1);
  const transmission = clamp01(pick('transmission') ?? 0);
  const clearcoat = clamp01(pick('clearcoat') ?? 0);
  const sheen = clamp01(pick('sheen') ?? 0);
  return {
    color: color(pick('color'), MATERIAL_DEFAULTS.color),
    metalness: clamp01(pick('metalness') ?? MATERIAL_DEFAULTS.metalness),
    roughness: clamp01(pick('roughness') ?? MATERIAL_DEFAULTS.roughness),
    opacity,
    transparent: opacity < 1,
    emissive: color(pick('emissive'), MATERIAL_DEFAULTS.emissive),
    emissiveIntensity: Math.min(100, Math.max(0, pick('emissive_intensity') ?? 1)),
    wireframe: m?.wireframe ?? false,
    clearcoat,
    clearcoatRoughness: clamp01(pick('clearcoat_roughness') ?? 0),
    transmission,
    ior: Math.min(2.333, Math.max(1, pick('ior') ?? 1.5)),
    thickness: Math.min(10, Math.max(0, pick('thickness') ?? 0)),
    sheen,
    physical: clearcoat > 0 || transmission > 0 || sheen > 0,
  };
}

/** Physical fields a preset owns — cleared when a preset is applied so its defaults show. */
const PRESET_OWNED: (keyof Scene3dMaterial)[] = [
  'metalness',
  'roughness',
  'clearcoat',
  'clearcoat_roughness',
  'transmission',
  'ior',
  'thickness',
  'sheen',
];

/**
 * Apply a preset to a material: set `preset`, drop the physical overrides the
 * preset owns (so clicking "Brushed metal" really looks like brushed metal),
 * keep colour / opacity / emissive / notes-like fields. `null` removes the
 * preset (back to plain PBR) and keeps everything else.
 */
export function applyMaterialPreset(m: Scene3dMaterial | undefined, preset: MaterialPresetId | null): Scene3dMaterial {
  const next: Scene3dMaterial = { ...(m ?? {}) };
  if (preset === null) {
    delete next.preset;
    return next;
  }
  for (const k of PRESET_OWNED) delete next[k];
  next.preset = preset;
  return next;
}

// ── Environments ─────────────────────────────────────────────────────────────

/** A glowing rectangle in the procedural environment (a softbox / the sun / a window). */
export interface EnvPanel {
  /** Direction from the origin (normalized by the builder). */
  dir: [number, number, number];
  /** Panel colour (linear-ish; the builder multiplies by `intensity`). */
  color: string;
  intensity: number;
  /** Width × height in metres at radius 10. */
  size: [number, number];
}

export interface EnvPresetDef {
  id: EnvPresetId;
  label: string;
  /** Sky gradient for the environment sphere: zenith → horizon → ground. */
  sky: [string, string, string];
  panels: EnvPanel[];
  /** The backdrop the studio shows (CSS/three gradient: top → bottom). */
  backdrop: [string, string];
  /** Renderer exposure that suits the preset. */
  exposure: number;
}

export const ENV_PRESETS: readonly EnvPresetDef[] = [
  {
    id: 'studio-soft',
    label: 'Studio soft',
    sky: ['#f4f3f8', '#dcdbe4', '#b9b8c2'],
    panels: [
      { dir: [0.6, 0.7, 0.4], color: '#ffffff', intensity: 5, size: [6, 4] },
      { dir: [-0.8, 0.4, 0.3], color: '#f1efff', intensity: 2.5, size: [5, 5] },
      { dir: [0, 0.3, -1], color: '#ffffff', intensity: 3, size: [8, 3] },
      { dir: [0, 1, 0], color: '#ffffff', intensity: 1.6, size: [10, 10] },
    ],
    backdrop: ['#f5f4fa', '#dedce8'],
    exposure: 1,
  },
  {
    id: 'sunset',
    label: 'Sunset',
    sky: ['#3b3a6e', '#f29e6b', '#4a2f3a'],
    panels: [
      { dir: [0.9, 0.15, 0.2], color: '#ffb36b', intensity: 9, size: [3, 3] },
      { dir: [-0.6, 0.5, -0.4], color: '#8f7cf0', intensity: 1.5, size: [8, 5] },
    ],
    backdrop: ['#5a4b8f', '#f0a574'],
    exposure: 1.05,
  },
  {
    id: 'night',
    label: 'Night',
    sky: ['#070b1a', '#141c38', '#05060d'],
    panels: [
      { dir: [-0.7, 0.4, -0.5], color: '#6f8cff', intensity: 4, size: [4, 6] },
      { dir: [0.8, 0.2, 0.4], color: '#b388ff', intensity: 2.5, size: [3, 5] },
      { dir: [0, 1, 0], color: '#27325c', intensity: 1, size: [10, 10] },
    ],
    backdrop: ['#0d1330', '#04050b'],
    exposure: 1.15,
  },
  {
    id: 'none',
    label: 'None (lights only)',
    sky: ['#000000', '#000000', '#000000'],
    panels: [],
    backdrop: ['#0f172a', '#0f172a'],
    exposure: 1,
  },
];

export function envPreset(id: string | undefined | null): EnvPresetDef | null {
  return ENV_PRESETS.find((e) => e.id === id) ?? null;
}

// ── Web budget ───────────────────────────────────────────────────────────────

/** A hero embed should stay under this (compressed GLB). */
export const WEB_BUDGET_BYTES = 2 * 1024 * 1024;

export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return '—';
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10 * 1024 ? 1 : 0)} KB`;
  return `${(n / (1024 * 1024)).toFixed(2)} MB`;
}

/** Budget readout: `ok` ≤ 75 %, `warn` ≤ 100 %, `over` beyond. */
export function budgetStatus(bytes: number, budget = WEB_BUDGET_BYTES): { tone: 'ok' | 'warn' | 'over'; pct: number; label: string } {
  const pct = budget > 0 ? Math.round((bytes / budget) * 100) : 0;
  const tone = pct <= 75 ? 'ok' : pct <= 100 ? 'warn' : 'over';
  return { tone, pct, label: `${formatBytes(bytes)} of ${formatBytes(budget)} (${pct}%)` };
}
