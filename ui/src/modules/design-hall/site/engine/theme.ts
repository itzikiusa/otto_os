// Site Studio — brand tokens → site theme.
//
// A brand kit (`otto-brand/1`: `color.<name>.$value`, `font.display|body|mono`,
// `radius.*`, `space.*`, …; the legacy `typography.*.$value.fontFamily` shape
// is read too) flattens to CSS custom properties `--brand-<group>-<name>`
// (`color.primary` → `--brand-color-primary`). The site stylesheet never names
// a kit token directly: it reads theme SLOTS (`--os-primary`, `--os-ink`, …),
// each bound to the kit token that fills it (`var(--brand-color-primary)`) or
// a sensible default when the kit has no such token / no kit is linked.
//
// Mirrored by crates/otto-design/src/site/theme.rs — keep the slot rules,
// defaults and sanitizing identical.

import type { SiteSection } from './types';

/** Defaults when no brand kit is linked (a violet + amber launch palette). */
export const DEFAULT_SLOTS = {
  primary: '#5B3DF5',
  accent: '#FFB547',
  ink: '#14122B',
  surface: '#FFFFFF',
  surfaceAlt: '#F6F4FF',
  fontDisplay: '"Inter Display", Inter, ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif',
  fontBody: 'Inter, ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif',
  radius: '14px',
} as const;

export interface BrandToken {
  /** Dotted path in the kit, e.g. `color.primary`. */
  path: string;
  /** `--brand-color-primary`. */
  cssVar: string;
  /** Sanitized CSS value. */
  value: string;
}

export type SlotName =
  | 'primary'
  | 'accent'
  | 'ink'
  | 'surface'
  | 'surface-alt'
  | 'font-display'
  | 'font-body'
  | 'radius';

export interface Theme {
  /** Kit tokens in document order (empty without a kit). */
  tokens: BrandToken[];
  byPath: Record<string, string>;
  /** Concrete slot values (for contrast math and swatches). */
  primary: string;
  accent: string;
  ink: string;
  surface: string;
  surfaceAlt: string;
  /** The lighter / darker of surface and ink (text on dark / light). */
  paper: string;
  night: string;
  /** Button label colours with the best contrast on primary / accent. */
  onPrimary: string;
  onAccent: string;
  fontDisplay: string;
  fontBody: string;
  radius: string;
  /** Slot → the CSS it is bound to (`var(--brand-color-primary)` or a literal). */
  slotCss: Record<SlotName, string>;
  /** Slot → kit path it came from (null = default). */
  slotPath: Record<SlotName, string | null>;
  kitName: string | null;
}

const SKIP_GROUPS = new Set(['type', 'version', 'name', 'description', 'logos', 'voice', 'meta', 'notes']);
const MAX_TOKENS = 400;

/** `surfaceAlt` / `Surface Alt` → `surface-alt`; anything else → `-`. */
export function tokenSlug(s: string): string {
  return s
    .replace(/([a-z0-9])([A-Z])/g, '$1-$2')
    .toLowerCase()
    .replace(/[^a-z0-9-]+/g, '-')
    .replace(/-+/g, '-')
    .replace(/^-|-$/g, '');
}

/** Strip anything that could escape a declaration (`; { } < > \` and controls). */
export function cssValue(v: string): string {
  let out = '';
  for (const ch of v) {
    const c = ch.charCodeAt(0);
    if (c < 32 || ch === ';' || ch === '{' || ch === '}' || ch === '<' || ch === '>' || ch === '\\') continue;
    out += ch;
  }
  return out.trim().slice(0, 200);
}

/** `var(<name>)` — built at runtime so the ui-guards var() scan never sees a partial name. */
export function cssVar(name: string): string {
  return 'var(' + name + ')';
}

function scalar(v: unknown): string | null {
  if (typeof v === 'string') return v;
  if (typeof v === 'number' && Number.isFinite(v)) return String(v);
  return null;
}

/** Flatten a brand document into tokens (document order, bounded). */
export function flattenBrand(doc: unknown): BrandToken[] {
  const out: BrandToken[] = [];
  if (!doc || typeof doc !== 'object' || Array.isArray(doc)) return out;
  const seen = new Set<string>();
  const push = (path: string[], raw: string) => {
    if (out.length >= MAX_TOKENS) return;
    const segs = path.map(tokenSlug).filter(Boolean);
    if (segs.length < 2) return;
    const p = segs.join('.');
    const value = cssValue(raw);
    if (!value || seen.has(p)) return;
    seen.add(p);
    out.push({ path: p, cssVar: '--brand-' + segs.join('-'), value });
  };
  const walk = (group: string, node: unknown, path: string[], depth: number) => {
    if (depth > 4 || !node || typeof node !== 'object' || Array.isArray(node)) return;
    for (const [k, v] of Object.entries(node as Record<string, unknown>)) {
      if (k.startsWith('$')) continue;
      const here = [...path, k];
      if (v && typeof v === 'object' && !Array.isArray(v) && '$value' in (v as object)) {
        const val = (v as Record<string, unknown>).$value;
        const s = scalar(val);
        if (s != null) push(here, s);
        else if (group === 'typography' && val && typeof val === 'object') {
          // Legacy composite typography → a font family token.
          const fam = scalar((val as Record<string, unknown>).fontFamily);
          if (fam) push(['font', ...here.slice(1)], fam);
        }
      } else {
        const s = scalar(v);
        if (s != null) push(here, s);
        else walk(group, v, here, depth + 1);
      }
    }
  };
  for (const [group, node] of Object.entries(doc as Record<string, unknown>)) {
    if (group.startsWith('$') || SKIP_GROUPS.has(group)) continue;
    walk(group, node, [group], 0);
  }
  return out;
}

// ── Colour math (WCAG 2.x) ───────────────────────────────────────────────────

/** Parse `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb(r g b)` / `rgb(r, g, b)` → [r,g,b] 0..255. */
export function parseColor(v: string): [number, number, number] | null {
  const s = v.trim().toLowerCase();
  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/.exec(s);
  if (hex) {
    let h = hex[1];
    if (h.length === 3) h = h.split('').map((c) => c + c).join('');
    return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)];
  }
  const rgb = /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)/.exec(s);
  if (rgb) {
    const c = [Number(rgb[1]), Number(rgb[2]), Number(rgb[3])];
    if (c.every((x) => Number.isFinite(x) && x >= 0 && x <= 255)) return [c[0], c[1], c[2]];
  }
  return null;
}

export function luminance(v: string): number | null {
  const c = parseColor(v);
  if (!c) return null;
  const lin = c.map((x) => {
    const n = x / 255;
    return n <= 0.03928 ? n / 12.92 : ((n + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
}

/** WCAG contrast ratio, one decimal; null when either colour can't be parsed. */
export function contrast(a: string, b: string): number | null {
  const la = luminance(a);
  const lb = luminance(b);
  if (la == null || lb == null) return null;
  const hi = Math.max(la, lb);
  const lo = Math.min(la, lb);
  return Math.round(((hi + 0.05) / (lo + 0.05)) * 10) / 10;
}

/** `#aabbcc` mix of two parseable colours (`t` = share of `b`). */
export function mix(a: string, b: string, t: number): string {
  const ca = parseColor(a);
  const cb = parseColor(b);
  if (!ca || !cb) return a;
  const h = (n: number) => Math.round(n).toString(16).padStart(2, '0');
  return '#' + [0, 1, 2].map((i) => h(ca[i] + (cb[i] - ca[i]) * t)).join('');
}

// ── Theme ────────────────────────────────────────────────────────────────────

const SLOT_SOURCES: Record<SlotName, string[]> = {
  primary: ['color.primary', 'color.brand', 'color.main'],
  accent: ['color.accent', 'color.secondary', 'color.highlight'],
  ink: ['color.ink', 'color.text', 'color.foreground', 'color.fg'],
  surface: ['color.surface', 'color.background', 'color.bg', 'color.paper'],
  'surface-alt': ['color.surface-alt', 'color.subtle', 'color.muted-surface', 'color.canvas'],
  'font-display': ['font.display', 'font.heading', 'font.headline', 'font.title'],
  'font-body': ['font.body', 'font.text', 'font.base', 'font.sans'],
  radius: ['radius.card', 'radius.m', 'radius.md', 'radius.default', 'radius.base'],
};

/** Build the theme from a brand document (null → defaults). */
export function buildTheme(brandDoc: unknown, kitName: string | null = null): Theme {
  const tokens = flattenBrand(brandDoc);
  const byPath: Record<string, string> = {};
  for (const t of tokens) byPath[t.path] = t.value;
  const slotPath = {} as Record<SlotName, string | null>;
  const slotCss = {} as Record<SlotName, string>;
  const pick = (slot: SlotName, fallback: string, firstOf?: string): string => {
    let path = SLOT_SOURCES[slot].find((p) => byPath[p] != null) ?? null;
    if (!path && firstOf) path = tokens.find((t) => t.path.startsWith(firstOf + '.'))?.path ?? null;
    slotPath[slot] = path;
    const tok = path ? tokens.find((t) => t.path === path) : undefined;
    slotCss[slot] = tok ? cssVar(tok.cssVar) : fallback;
    return tok ? tok.value : fallback;
  };
  const primary = pick('primary', DEFAULT_SLOTS.primary, 'color');
  const accent = pick('accent', DEFAULT_SLOTS.accent);
  const ink = pick('ink', DEFAULT_SLOTS.ink);
  const surface = pick('surface', DEFAULT_SLOTS.surface);
  // Surface-alt defaults to a whisper of the primary over the surface.
  const altDefault = slotPath.primary || slotPath.surface ? mix(surface, primary, 0.06) : DEFAULT_SLOTS.surfaceAlt;
  const surfaceAlt = pick('surface-alt', altDefault);
  const fontDisplay = pick('font-display', DEFAULT_SLOTS.fontDisplay);
  const fontBody = pick('font-body', DEFAULT_SLOTS.fontBody);
  const radius = pick('radius', DEFAULT_SLOTS.radius, 'radius');
  const ls = luminance(surface) ?? 1;
  const li = luminance(ink) ?? 0;
  const paper = ls >= li ? surface : ink;
  const night = ls >= li ? ink : surface;
  const best = (bg: string) => ((contrast(paper, bg) ?? 0) >= (contrast(night, bg) ?? 0) ? paper : night);
  return {
    tokens,
    byPath,
    primary,
    accent,
    ink,
    surface,
    surfaceAlt,
    paper,
    night,
    onPrimary: best(primary),
    onAccent: best(accent),
    fontDisplay,
    fontBody,
    radius,
    slotCss,
    slotPath,
    kitName,
  };
}

/**
 * The declarations that theme a site: every kit token as `--brand-*`, then the
 * slots bound to them. Used as the editor root's inline style and as the
 * export's `.os-site { … }` block (so kit tokens always beat the defaults
 * site.css declares with zero specificity).
 */
export function themeDeclarations(t: Theme): [string, string][] {
  const out: [string, string][] = t.tokens.map((k) => [k.cssVar, k.value]);
  const slot = (name: string, css: string) => out.push([name, css]);
  slot('--os-primary', t.slotCss.primary);
  slot('--os-accent', t.slotCss.accent);
  slot('--os-ink', t.slotCss.ink);
  slot('--os-surface', t.slotCss.surface);
  slot('--os-surface-alt', t.slotCss['surface-alt']);
  slot('--os-paper', t.paper === t.surface ? cssVar('--os-surface') : cssVar('--os-ink'));
  slot('--os-night', t.night === t.ink ? cssVar('--os-ink') : cssVar('--os-surface'));
  slot('--os-on-primary', t.onPrimary === t.paper ? cssVar('--os-paper') : cssVar('--os-night'));
  slot('--os-on-accent', t.onAccent === t.paper ? cssVar('--os-paper') : cssVar('--os-night'));
  slot('--os-font-display', t.slotCss['font-display']);
  slot('--os-font-body', t.slotCss['font-body']);
  slot('--os-radius', t.slotCss.radius);
  return out;
}

/** `a: b; c: d` (an inline style / a CSS block body). */
export function themeStyle(t: Theme): string {
  return themeDeclarations(t)
    .map(([k, v]) => `${k}: ${v}`)
    .join('; ');
}

// ── Section backgrounds, tone and contrast ──────────────────────────────────

export const GRADIENTS = ['soft', 'primary', 'ink', 'sunset'] as const;
export type GradientPreset = (typeof GRADIENTS)[number];

export interface ResolvedBackground {
  /** Value for the section's `--os-bg`, or null = the default surface. */
  css: string | null;
  /** Gradient preset class (`os-bg-<preset>`), if any. */
  preset: GradientPreset | null;
  /** Concrete colour the text sits on (contrast checks). */
  color: string;
  tone: 'light' | 'dark';
  /** A raw colour that is not one of the kit's tokens. */
  offBrand: boolean;
  /** `token:<path>` that the kit doesn't define (rendered as the default). */
  unknownToken: string | null;
}

export function toneFor(color: string, t: Theme): 'light' | 'dark' {
  return (contrast(t.night, color) ?? 21) >= (contrast(t.paper, color) ?? 0) ? 'light' : 'dark';
}

export function resolveBackground(bg: string | undefined, t: Theme): ResolvedBackground {
  const v = (bg ?? '').trim();
  const base = (color: string, css: string | null, extra: Partial<ResolvedBackground> = {}): ResolvedBackground => ({
    css,
    preset: null,
    color,
    tone: toneFor(color, t),
    offBrand: false,
    unknownToken: null,
    ...extra,
  });
  if (!v) return base(t.surface, null);
  if (v.startsWith('token:')) {
    const path = v.slice(6).trim();
    const tok = t.tokens.find((k) => k.path === path);
    if (tok) return base(parseColor(tok.value) ? tok.value : t.surface, cssVar(tok.cssVar));
    // Slot aliases work without a kit: token:color.primary etc.
    const alias: Record<string, [string, string]> = {
      'color.primary': [t.primary, cssVar('--os-primary')],
      'color.accent': [t.accent, cssVar('--os-accent')],
      'color.ink': [t.ink, cssVar('--os-ink')],
      'color.surface': [t.surface, cssVar('--os-surface')],
      'color.surface-alt': [t.surfaceAlt, cssVar('--os-surface-alt')],
    };
    const a = alias[path];
    if (a) return base(a[0], a[1]);
    return base(t.surface, null, { unknownToken: path });
  }
  if (v.startsWith('gradient:')) {
    const p = v.slice(9) as GradientPreset;
    if (!GRADIENTS.includes(p)) return base(t.surface, null);
    const color = p === 'soft' ? t.surfaceAlt : p === 'ink' ? t.night : t.primary;
    const css = p === 'soft' ? cssVar('--os-surface-alt') : p === 'ink' ? cssVar('--os-night') : cssVar('--os-primary');
    return base(color, css, { preset: p });
  }
  if (parseColor(v)) {
    const onKit = t.tokens.some((k) => k.value.toLowerCase() === v.toLowerCase());
    return base(v, cssValue(v), { offBrand: !onKit });
  }
  return base(t.surface, null);
}

export interface SectionContrast {
  /** Body text on the section background. */
  text: number | null;
  /** Primary button label on its fill. */
  button: number | null;
  tone: 'light' | 'dark';
}

/** WCAG checks for one section (text ≥ 4.5, large display text ≥ 3). */
export function sectionContrast(s: Pick<SiteSection, 'style'>, t: Theme): SectionContrast {
  const bg = resolveBackground(s.style?.background, t);
  const fg = bg.tone === 'light' ? t.night : t.paper;
  const btnBg = bg.tone === 'light' ? t.primary : t.accent;
  const btnFg = bg.tone === 'light' ? t.onPrimary : t.onAccent;
  return { text: contrast(fg, bg.color), button: contrast(btnFg, btnBg), tone: bg.tone };
}

/** Brand colour swatches offered by the inspector (kit colours, else the slots). */
export function swatches(t: Theme): { value: string; label: string; color: string }[] {
  const kit = t.tokens.filter((k) => k.path.startsWith('color.') && parseColor(k.value));
  if (kit.length) return kit.slice(0, 12).map((k) => ({ value: `token:${k.path}`, label: k.path.slice(6), color: k.value }));
  return [
    { value: 'token:color.surface', label: 'surface', color: t.surface },
    { value: 'token:color.surface-alt', label: 'surface-alt', color: t.surfaceAlt },
    { value: 'token:color.primary', label: 'primary', color: t.primary },
    { value: 'token:color.accent', label: 'accent', color: t.accent },
    { value: 'token:color.ink', label: 'ink', color: t.ink },
  ];
}
