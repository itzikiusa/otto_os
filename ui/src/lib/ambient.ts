// Ambient backdrop: the soft, full-bleed wash behind Otto's chrome (sidebar,
// toolbar strip, status bar) and behind the Home desktop. Generated here from
// the accent + scheme — no network images — or from a user photo that is
// downscaled, blurred and luminance-clamped on the device (`processWallpaper`).
//
// THE CONTRACT that keeps text on glass AA: every backdrop pixel stays inside
// a scheme luminance band (`AMBIENT_LUM`). Light schemes only ever see a light
// backdrop, dark schemes a dark one, so the chrome's glass tint
// (`--glass-tint*`, tokens.css) composited over ANY ambient pixel keeps
// `--text` and `--text-dim` ≥ 4.5:1 — see unit/ambient.test.ts, which checks
// it against the real token values for all five theme × scheme combinations.
//
// Pure module (no DOM, no imports) so the unit tests can load it directly.

export type AmbientMode = 'none' | 'subtle' | 'wallpaper';
export type AmbientScheme = 'light' | 'dark';
export type Rgb = [number, number, number];

export const AMBIENT_MODES: { id: AmbientMode; label: string; desc: string }[] = [
  { id: 'none', label: 'None', desc: 'Plain window colours' },
  { id: 'subtle', label: 'Subtle', desc: 'A soft wash of your accent' },
  { id: 'wallpaper', label: 'Wallpaper', desc: 'A generated wallpaper, or your own photo' },
];

export function isAmbientMode(v: unknown): v is AmbientMode {
  return v === 'none' || v === 'subtle' || v === 'wallpaper';
}

/** Relative-luminance band every backdrop pixel is kept inside. The generator
 *  aims inside a tighter band (`TARGET`) because gradient interpolation in
 *  sRGB can dip a little below its endpoints' luminance. */
export const AMBIENT_LUM: Record<AmbientScheme, { min: number; max: number }> = {
  light: { min: 0.68, max: 1 },
  dark: { min: 0, max: 0.07 },
};
const TARGET: Record<AmbientScheme, { min: number; max: number }> = {
  light: { min: 0.74, max: 1 },
  dark: { min: 0, max: 0.05 },
};

// ── colour math ─────────────────────────────────────────────────────────────

export function parseHex(hex: string): Rgb | null {
  const m = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const h = m[1].length === 3 ? [...m[1]].map((c) => c + c).join('') : m[1];
  return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16)) as Rgb;
}

export function toHex(c: Rgb): string {
  return `#${c.map((v) => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, '0')).join('')}`;
}

function toLin(v: number): number {
  const c = v / 255;
  return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}
function fromLin(l: number): number {
  const c = l <= 0.0030402 ? l * 12.92 : 1.055 * l ** (1 / 2.4) - 0.055;
  return Math.round(Math.min(1, Math.max(0, c)) * 255);
}

/** WCAG relative luminance of an sRGB (0–255) colour. */
export function luminance(c: Rgb): number {
  return 0.2126 * toLin(c[0]) + 0.7152 * toLin(c[1]) + 0.0722 * toLin(c[2]);
}

/** WCAG contrast ratio between two colours. */
export function contrast(a: Rgb, b: Rgb): number {
  const x = luminance(a);
  const y = luminance(b);
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

/** `color-mix(in srgb, a p, b)` / alpha compositing of `a` at `p` over `b`
 *  (what the browser does for a translucent layer). */
export function mixSrgb(a: Rgb, b: Rgb, p: number): Rgb {
  return [0, 1, 2].map((i) => a[i] * p + b[i] * (1 - p)) as Rgb;
}

/** Pull `c` into the scheme's band by mixing toward white (light) or black
 *  (dark) in LINEAR light, where luminance mixes exactly — so the result sits
 *  on the bound, keeping as much of the colour as the band allows. */
export function clampLuminance(c: Rgb, band: { min: number; max: number }): Rgb {
  const lin = c.map(toLin) as Rgb;
  const l = 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
  if (l < band.min) {
    const t = (band.min - l) / (1 - l);
    return lin.map((v) => fromLin(v + (1 - v) * t)) as Rgb;
  }
  if (l > band.max) {
    const k = band.max / l;
    // Round DOWN so 8-bit rounding can't push the colour back over the bound.
    return lin.map((v) => Math.max(0, fromLin(v * k) - 1)) as Rgb;
  }
  return c;
}

function hue(c: Rgb): number {
  const [r, g, b] = c.map((v) => v / 255);
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const d = max - min;
  if (d === 0) return 220;
  const h = max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return (h * 60 + 360) % 360;
}

function hsl(h: number, s: number, l: number): Rgb {
  const a = s * Math.min(l, 1 - l);
  const f = (n: number): number => {
    const k = (n + h / 30) % 12;
    return l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1));
  };
  return [f(0) * 255, f(8) * 255, f(4) * 255].map(Math.round) as Rgb;
}

// ── generated art ───────────────────────────────────────────────────────────

/** Four accent-derived hues (accent, two analogues, a quiet complement),
 *  every one inside the scheme's luminance band. */
export function ambientPalette(accent: string, scheme: AmbientScheme): Rgb[] {
  const h = hue(parseHex(accent) ?? [10, 132, 255]);
  const light = scheme === 'light';
  const tones: [number, number, number][] = light
    ? [
        [h, 0.85, 0.8],
        [h + 38, 0.75, 0.84],
        [h - 46, 0.7, 0.84],
        [h + 180, 0.45, 0.86],
      ]
    : [
        [h, 0.6, 0.3],
        [h + 38, 0.5, 0.26],
        [h - 46, 0.5, 0.26],
        [h + 180, 0.3, 0.22],
      ];
  return tones.map(([hh, s, l]) => clampLuminance(hsl((hh + 360) % 360, s, l), TARGET[scheme]));
}

const rgba = (c: Rgb, a: number): string => `rgba(${c[0]}, ${c[1]}, ${c[2]}, ${a})`;

/** The backdrop as a CSS `background-image` list, or `none`. `subtle` is a
 *  soft wash over the window colour (translucent — native vibrancy still
 *  shows through it in the desktop app); `wallpaper` paints its own tinted
 *  base, so it is opaque. A user photo replaces the generated wallpaper. */
export function ambientImage(
  mode: AmbientMode,
  accent: string,
  scheme: AmbientScheme,
  photo?: string | null,
): string {
  if (mode === 'none') return 'none';
  if (mode === 'wallpaper' && photo) return `url("${photo}")`;
  const [a, b, c, d] = ambientPalette(accent, scheme);
  if (mode === 'subtle') {
    return [
      `radial-gradient(70% 95% at 0% 0%, ${rgba(a, 0.9)}, transparent 72%)`,
      `radial-gradient(55% 75% at 100% 0%, ${rgba(b, 0.55)}, transparent 70%)`,
      `radial-gradient(65% 80% at 85% 100%, ${rgba(c, 0.7)}, transparent 72%)`,
    ].join(', ');
  }
  const base = clampLuminance(mixSrgb(a, scheme === 'light' ? [250, 250, 252] : [20, 20, 26], 0.35), TARGET[scheme]);
  return [
    `radial-gradient(55% 65% at 10% 6%, ${rgba(a, 0.95)}, transparent 72%)`,
    `radial-gradient(50% 60% at 96% 0%, ${rgba(b, 0.9)}, transparent 72%)`,
    `radial-gradient(65% 70% at 78% 100%, ${rgba(c, 0.9)}, transparent 72%)`,
    `radial-gradient(45% 55% at 0% 100%, ${rgba(d, 0.8)}, transparent 72%)`,
    `linear-gradient(${toHex(base)}, ${toHex(base)})`,
  ].join(', ');
}

/** Every colour the generated art can put under the chrome: the palette, the
 *  wallpaper base, and pairwise blends (gradient overlaps / falloff) — what the
 *  contrast test checks the glass against. */
export function ambientSamples(accent: string, scheme: AmbientScheme): Rgb[] {
  const pal = ambientPalette(accent, scheme);
  const base = clampLuminance(mixSrgb(pal[0], scheme === 'light' ? [250, 250, 252] : [20, 20, 26], 0.35), TARGET[scheme]);
  const all = [...pal, base];
  const out: Rgb[] = [...all];
  for (let i = 0; i < all.length; i++) {
    for (let j = i + 1; j < all.length; j++) {
      for (const t of [0.25, 0.5, 0.75]) out.push(mixSrgb(all[i], all[j], t));
    }
  }
  return out;
}

// ── user photo ──────────────────────────────────────────────────────────────

/** Clamp decoded RGBA pixels (in place) into the scheme's band. Run on the
 *  already-blurred, downscaled photo; the light and dark variants are stored
 *  side by side so switching scheme never needs the original again. */
export function clampPixels(data: Uint8ClampedArray, scheme: AmbientScheme): void {
  const band = TARGET[scheme];
  for (let i = 0; i < data.length; i += 4) {
    const c = clampLuminance([data[i], data[i + 1], data[i + 2]], band);
    data[i] = c[0];
    data[i + 1] = c[1];
    data[i + 2] = c[2];
    data[i + 3] = 255;
  }
}
