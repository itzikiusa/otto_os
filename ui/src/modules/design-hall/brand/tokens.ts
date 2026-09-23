// Brand Kit — pure token helpers (no Svelte, no fetch) shared by the Brand Kit
// editor and every studio that reads a kit (Site Studio, 3D Studio, …):
//
//   brandCssVars(doc)                        → { '--brand-color-primary': '#5B3DF5', … }
//   resolveToken(doc, 'token:color.primary') → '#5B3DF5'
//
// A kit is an `otto-brand/1` document (docs/contracts/api.md § Brand Kit).
// Other artifacts name a token as `token:<group>.<name>` (or the CSS property
// `var(--brand-<group>-<name>)`) and link the kit with rel `uses_tokens`.
//
// This mirrors crates/otto-design/src/brand — `doc.rs` (names, validation,
// canonical values), `contrast.rs` (WCAG) and `impact.rs` (references). Keep the
// two in step. Unit-tested: ui/unit/brandTokens.test.ts.

import type {
  BrandDoc,
  BrandFontRole,
  BrandLogoKind,
  BrandTokenChange,
  BrandTokenGroup,
} from '../../../lib/api/types';

export const BRAND_SCHEMA = 'otto-brand/1';
export const TOKEN_GROUPS: readonly BrandTokenGroup[] = ['color', 'font', 'type', 'radius', 'space'];
export const FONT_ROLES: readonly BrandFontRole[] = ['display', 'body', 'mono'];
export const LOGO_KINDS: readonly BrandLogoKind[] = ['full', 'mark', 'mono'];
export const MAX_TOKENS_PER_GROUP = 64;
export const MAX_LIST_ITEMS = 20;
const MAX_PX = 10_000;
const MAX_TEXT = 2_000;
const MAX_STACK = 300;
const DESIGN_PREFIX = 'otto://design/';

type Rec = Record<string, unknown>;

function rec(v: unknown): Rec | null {
  return v && typeof v === 'object' && !Array.isArray(v) ? (v as Rec) : null;
}

function str(v: unknown): string | null {
  return typeof v === 'string' ? v : null;
}

// ── Names and values ─────────────────────────────────────────────────────────

/** `[A-Za-z0-9][A-Za-z0-9_-]{0,63}`. */
export function isValidTokenName(s: string): boolean {
  return /^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/.test(s);
}

/** `#abc` → `#AABBCC`; `#aabbcc` / `#aabbccdd` upper-cased; else null. */
export function normalizeHex(s: string): string | null {
  const m = /^#([0-9a-fA-F]+)$/.exec(s.trim());
  if (!m) return null;
  const h = m[1];
  if (h.length === 3) return `#${h.split('').map((c) => c + c).join('').toUpperCase()}`;
  if (h.length === 6 || h.length === 8) return `#${h.toUpperCase()}`;
  return null;
}

/** `surfaceAlt` / `surface_alt` → `surface-alt` (the CSS spelling of a token name). */
export function cssIdent(name: string): string {
  let out = '';
  let prev = '';
  for (const c of name) {
    if (c >= 'A' && c <= 'Z') {
      if (prev && ((prev >= 'a' && prev <= 'z') || (prev >= '0' && prev <= '9'))) out += '-';
      out += c.toLowerCase();
    } else if (c === '_') {
      out += '-';
    } else {
      out += c;
    }
    prev = c;
  }
  return out;
}

/** `--brand-<group>-<cssIdent(name)>`. */
export function cssVarName(group: BrandTokenGroup, name: string): string {
  return `--brand-${group}-${cssIdent(name)}`;
}

/** Shortest number form: `16`, `1.5`, `0.333`. */
export function fmtNum(n: number): string {
  return Number.isInteger(n) ? String(n) : String(Math.round(n * 1000) / 1000);
}

const UNSAFE_CSS = /[;{}<>\\\n\r]/;

/** Drop the characters that could close a declaration or a tag. */
export function cleanCssValue(s: string): string {
  return s.replace(/[;{}<>\\\n\r]/g, '').trim();
}

/** The font role a text style uses (`display` for display/headline/title/hero/h1–h9, `mono` for code). */
export function fontRoleForStyle(style: string): BrandFontRole {
  const s = style.toLowerCase();
  if (s.includes('mono') || s.includes('code')) return 'mono';
  if (/^h[0-9]$/.test(s) || s.includes('display') || s.includes('headline') || s.includes('title') || s.includes('hero')) return 'display';
  return 'body';
}

function px(v: unknown): number | null {
  return typeof v === 'number' && Number.isFinite(v) && v >= 0 && v <= MAX_PX ? v : null;
}

function weight(v: unknown): number | null {
  return typeof v === 'number' && Number.isInteger(v) && v >= 1 && v <= 1000 ? v : null;
}

// ── Readers (lenient: entries that don't validate are skipped) ───────────────

export interface ColorTok {
  name: string;
  hex: string;
  description?: string;
}
export interface FontTok {
  name: BrandFontRole;
  stack: string;
  weights: number[];
}
export interface TypeTok {
  name: string;
  size: number;
  line: number;
  weight: number;
}
export interface PxTok {
  name: string;
  px: number;
  description?: string;
}

function entries(doc: unknown, group: string): [string, Rec][] {
  const g = rec(rec(doc)?.[group]);
  if (!g) return [];
  const out: [string, Rec][] = [];
  for (const [k, v] of Object.entries(g)) {
    const o = rec(v);
    if (o && isValidTokenName(k)) out.push([k, o]);
    if (out.length >= MAX_TOKENS_PER_GROUP) break;
  }
  return out;
}

export function colorTokens(doc: unknown): ColorTok[] {
  const out: ColorTok[] = [];
  for (const [name, o] of entries(doc, 'color')) {
    const hex = normalizeHex(str(o.$value) ?? '');
    if (!hex) continue;
    const d = str(o.$description);
    out.push(d ? { name, hex, description: d } : { name, hex });
  }
  return out;
}

export function fontTokens(doc: unknown): FontTok[] {
  const out: FontTok[] = [];
  for (const [name, o] of entries(doc, 'font')) {
    if (!(FONT_ROLES as readonly string[]).includes(name)) continue;
    const stack = cleanCssValue(str(o.$value) ?? '');
    if (!stack) continue;
    const weights = Array.isArray(o.weights) ? o.weights.map(weight).filter((w): w is number => w != null).slice(0, 9) : [];
    out.push({ name: name as BrandFontRole, stack, weights });
  }
  return out;
}

export function typeStyles(doc: unknown): TypeTok[] {
  if (!rec(rec(doc)?.type)) return [];
  const out: TypeTok[] = [];
  for (const [name, o] of entries(doc, 'type')) {
    const size = px(o.size);
    const line = px(o.line);
    const w = weight(o.weight);
    if (size && line && w) out.push({ name, size, line, weight: w });
  }
  return out;
}

export function pxTokens(doc: unknown, group: 'radius' | 'space'): PxTok[] {
  const out: PxTok[] = [];
  for (const [name, o] of entries(doc, group)) {
    const n = px(o.$value);
    if (n == null) continue;
    const d = str(o.$description);
    out.push(d ? { name, px: n, description: d } : { name, px: n });
  }
  return out;
}

/** The font a text style is set in: its role's font, else the first font. */
export function fontForStyle(doc: unknown, style: string): FontTok | null {
  const fonts = fontTokens(doc);
  const want = fontRoleForStyle(style);
  return fonts.find((f) => f.name === want) ?? fonts[0] ?? null;
}

// ── CSS custom properties ────────────────────────────────────────────────────

/**
 * Every token of the kit as a CSS custom property, in group order:
 * `--brand-color-<n>`, `--brand-font-<role>`, `--brand-type-<n>-size|-line|-weight`,
 * `--brand-radius-<n>`, `--brand-space-<n>`. Same names as the server's CSS export.
 */
export function brandCssVars(doc: unknown): Record<string, string> {
  const out: Record<string, string> = {};
  for (const c of colorTokens(doc)) out[cssVarName('color', c.name)] = c.hex;
  for (const f of fontTokens(doc)) out[cssVarName('font', f.name)] = f.stack;
  for (const t of typeStyles(doc)) {
    const base = cssVarName('type', t.name);
    out[`${base}-size`] = `${fmtNum(t.size)}px`;
    out[`${base}-line`] = `${fmtNum(t.line)}px`;
    out[`${base}-weight`] = String(t.weight);
  }
  for (const r of pxTokens(doc, 'radius')) out[cssVarName('radius', r.name)] = `${fmtNum(r.px)}px`;
  for (const s of pxTokens(doc, 'space')) out[cssVarName('space', s.name)] = `${fmtNum(s.px)}px`;
  return out;
}

/** The vars as a stylesheet block (`:root { … }` by default). */
export function brandCssText(doc: unknown, selector = ':root'): string {
  const lines = Object.entries(brandCssVars(doc)).map(([k, v]) => `  ${k}: ${v};`);
  return `${selector} {\n${lines.join('\n')}${lines.length ? '\n' : ''}}\n`;
}

/** The vars as an inline `style` attribute value. */
export function brandStyle(doc: unknown): string {
  return Object.entries(brandCssVars(doc))
    .map(([k, v]) => `${k}: ${v}`)
    .join('; ');
}

// ── Token references ─────────────────────────────────────────────────────────

export interface TokenRef {
  group: BrandTokenGroup;
  name: string;
  /** A type style's sub-property: `size` | `line` | `weight`. */
  sub: string | null;
}

/** `token:color.primary` / `color.primary` / `token:type.display.size` → its parts. */
export function parseTokenRef(ref: string): TokenRef | null {
  const s = ref.trim().replace(/^token:/, '');
  const [group, name, sub, ...rest] = s.split('.');
  if (rest.length || !group || !name) return null;
  if (!(TOKEN_GROUPS as readonly string[]).includes(group) || !isValidTokenName(name)) return null;
  if (sub != null && !(group === 'type' && (sub === 'size' || sub === 'line' || sub === 'weight'))) return null;
  return { group: group as BrandTokenGroup, name, sub: sub ?? null };
}

/** `token:color.primary` (the form other documents store). */
export function tokenRef(group: BrandTokenGroup, name: string): string {
  return `token:${group}.${name}`;
}

const FALLBACK_STACK = 'system-ui, sans-serif';

/**
 * The CSS value a reference resolves to in `doc`, or null when the kit has no
 * such token. Accepts `token:<group>.<name>[.<sub>]`, `<group>.<name>` and a
 * `--brand-…` / `var(--brand-…)` property. A type style without a sub-property
 * resolves to a `font` shorthand (`800 64px/72px "Inter", system-ui`).
 */
export function resolveToken(doc: unknown, ref: string): string | null {
  const t = ref.trim();
  const v = /^var\(\s*(--brand-[\w-]+)\s*\)$/.exec(t)?.[1] ?? (t.startsWith('--brand-') ? t : null);
  if (v) return brandCssVars(doc)[v] ?? null;
  const r = parseTokenRef(t);
  if (!r) return null;
  switch (r.group) {
    case 'color':
      return colorTokens(doc).find((c) => c.name === r.name)?.hex ?? null;
    case 'font':
      return fontTokens(doc).find((f) => f.name === r.name)?.stack ?? null;
    case 'radius':
    case 'space': {
      const p = pxTokens(doc, r.group).find((x) => x.name === r.name);
      return p ? `${fmtNum(p.px)}px` : null;
    }
    case 'type': {
      const s = typeStyles(doc).find((x) => x.name === r.name);
      if (!s) return null;
      if (r.sub === 'size') return `${fmtNum(s.size)}px`;
      if (r.sub === 'line') return `${fmtNum(s.line)}px`;
      if (r.sub === 'weight') return String(s.weight);
      const stack = fontForStyle(doc, s.name)?.stack ?? FALLBACK_STACK;
      return `${s.weight} ${fmtNum(s.size)}px/${fmtNum(s.line)}px ${stack}`;
    }
  }
  return null;
}

function identChar(c: string | undefined): boolean {
  return !!c && /[A-Za-z0-9_-]/.test(c);
}

/**
 * The token keys (`group.name`, sorted) a document names — `token:<group>.<name>`
 * and any `--brand-…` property `doc` defines. Mirrors `impact::token_refs`.
 */
export function tokenRefs(text: string, doc: unknown): string[] {
  const out = new Set<string>();
  for (const m of text.matchAll(/token:([A-Za-z0-9_.-]+)/g)) {
    if (identChar(text[(m.index ?? 0) - 1])) continue;
    const [g, n] = m[1].split('.').filter(Boolean);
    if (g && n && (TOKEN_GROUPS as readonly string[]).includes(g)) out.add(`${g}.${n}`);
  }
  const index = cssIndex(doc);
  for (const m of text.matchAll(/--brand-[A-Za-z0-9_-]+/g)) {
    if (identChar(text[(m.index ?? 0) - 1])) continue;
    const key = index[m[0]];
    if (key) out.add(key);
  }
  return [...out].sort();
}

/** CSS property → token key (`--brand-type-display-size` → `type.display`). */
export function cssIndex(doc: unknown): Record<string, string> {
  const out: Record<string, string> = {};
  for (const c of colorTokens(doc)) out[cssVarName('color', c.name)] = `color.${c.name}`;
  for (const f of fontTokens(doc)) out[cssVarName('font', f.name)] = `font.${f.name}`;
  for (const t of typeStyles(doc)) {
    const base = cssVarName('type', t.name);
    for (const s of ['size', 'line', 'weight']) out[`${base}-${s}`] = `type.${t.name}`;
  }
  for (const r of pxTokens(doc, 'radius')) out[cssVarName('radius', r.name)] = `radius.${r.name}`;
  for (const s of pxTokens(doc, 'space')) out[cssVarName('space', s.name)] = `space.${s.name}`;
  return out;
}

// ── Diff ─────────────────────────────────────────────────────────────────────

/** Every token as `group.name → canonical value` (what the server diffs). */
export function flatTokens(doc: unknown): Record<string, string> {
  const out: Record<string, string> = {};
  for (const c of colorTokens(doc)) out[`color.${c.name}`] = c.hex;
  for (const f of fontTokens(doc)) out[`font.${f.name}`] = f.stack;
  for (const t of typeStyles(doc)) out[`type.${t.name}`] = `${fmtNum(t.size)}/${fmtNum(t.line)}/${t.weight}`;
  for (const r of pxTokens(doc, 'radius')) out[`radius.${r.name}`] = `${fmtNum(r.px)}px`;
  for (const s of pxTokens(doc, 'space')) out[`space.${s.name}`] = `${fmtNum(s.px)}px`;
  return out;
}

/** Token-level diff (sorted by token), the same shape the impact route returns. */
export function diffTokens(before: unknown, after: unknown): BrandTokenChange[] {
  const a = flatTokens(before);
  const b = flatTokens(after);
  const keys = [...new Set([...Object.keys(a), ...Object.keys(b)])].sort();
  const out: BrandTokenChange[] = [];
  for (const k of keys) {
    const x = a[k] ?? null;
    const y = b[k] ?? null;
    if (x === y) continue;
    out.push({ token: k, change: x == null ? 'added' : y == null ? 'removed' : 'changed', before: x, after: y });
  }
  return out;
}

// ── Contrast (WCAG 2.x) ──────────────────────────────────────────────────────

/** Relative luminance of `#RGB`/`#RRGGBB[AA]` (alpha ignored); null when not a hex colour. */
export function luminance(hex: string): number | null {
  const h = normalizeHex(hex);
  if (!h) return null;
  const lin = [1, 3, 5].map((i) => {
    const c = parseInt(h.slice(i, i + 2), 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
}

/** Contrast ratio (1–21, 2 decimals) of two hex colours. */
export function contrast(a: string, b: string): number | null {
  const la = luminance(a);
  const lb = luminance(b);
  if (la == null || lb == null) return null;
  const [hi, lo] = la > lb ? [la, lb] : [lb, la];
  return Math.round(((hi + 0.05) / (lo + 0.05)) * 100) / 100;
}

export type ContrastLevel = 'AAA' | 'AA' | 'AA-large' | 'fail';

export function contrastLevel(r: number): ContrastLevel {
  return r >= 7 ? 'AAA' : r >= 4.5 ? 'AA' : r >= 3 ? 'AA-large' : 'fail';
}

/** The kit's ink: the `ink` token, else `text`, else the darkest colour, else black. */
export function inkOf(doc: unknown): { name: string | null; hex: string } {
  const colors = colorTokens(doc);
  for (const want of ['ink', 'text']) {
    const c = colors.find((x) => x.name === want);
    if (c) return { name: c.name, hex: c.hex };
  }
  let best: ColorTok | null = null;
  let bestL = Infinity;
  for (const c of colors) {
    const l = luminance(c.hex);
    if (l != null && l < bestL) {
      bestL = l;
      best = c;
    }
  }
  return best ? { name: best.name, hex: best.hex } : { name: null, hex: '#000000' };
}

/** Readable text colour on a fill: white or the kit's ink, whichever contrasts more. */
export function onColor(fill: string, ink = '#000000'): string {
  const w = contrast(fill, '#FFFFFF') ?? 0;
  const k = contrast(fill, ink) ?? 0;
  return w >= k ? '#FFFFFF' : ink;
}

// ── Roles for previews ───────────────────────────────────────────────────────

export interface BrandPalette {
  primary: string;
  accent: string;
  ink: string;
  surface: string;
  surfaceAlt: string;
}

/**
 * The five colours a generic preview needs, by token name with fallbacks:
 * primary (else the first colour), accent (else the second), ink, surface
 * (else white), surface-alt / surfaceAlt / surface_alt (else surface).
 */
export function brandPalette(doc: unknown): BrandPalette {
  const colors = colorTokens(doc);
  const by = (...names: string[]) => colors.find((c) => names.includes(c.name))?.hex;
  const primary = by('primary', 'brand') ?? colors[0]?.hex ?? '#4F46E5';
  const accent = by('accent', 'secondary') ?? colors.find((c) => c.hex !== primary)?.hex ?? primary;
  const surface = by('surface', 'background', 'bg') ?? '#FFFFFF';
  return {
    primary,
    accent,
    ink: inkOf(doc).hex,
    surface,
    surfaceAlt: by('surface-alt', 'surfaceAlt', 'surface_alt', 'muted') ?? surface,
  };
}

// ── Validation (client mirror of the server's `brand::doc::issues`) ──────────

/** Every problem with a kit, as `path: problem` (empty = valid). */
export function validateBrandDoc(doc: unknown): string[] {
  const out: string[] = [];
  const root = rec(doc);
  if (!root) return ['the document must be a JSON object'];
  if (root.$schema !== undefined && root.$schema !== BRAND_SCHEMA) out.push(`$schema: expected "${BRAND_SCHEMA}"`);
  if (root.name !== undefined && (typeof root.name !== 'string' || root.name.length > 200)) out.push('name: a string up to 200 characters');
  const group = (name: string, check: (path: string, tok: Rec) => void) => {
    const g = rec(root[name]);
    if (!g) {
      out.push(`${name}: must be an object of tokens`);
      return;
    }
    const keys = Object.keys(g).filter((k) => !k.startsWith('$'));
    if (keys.length > MAX_TOKENS_PER_GROUP) out.push(`${name}: at most ${MAX_TOKENS_PER_GROUP} tokens`);
    for (const k of keys.slice(0, MAX_TOKENS_PER_GROUP)) {
      const path = `${name}.${k}`;
      if (!isValidTokenName(k)) out.push(`${path}: names are letters, digits, - or _`);
      const tok = rec(g[k]);
      if (!tok) {
        out.push(`${path}: must be an object`);
        continue;
      }
      if (tok.$description !== undefined && (typeof tok.$description !== 'string' || tok.$description.length > MAX_TEXT)) {
        out.push(`${path}.$description: a string up to ${MAX_TEXT} characters`);
      }
      check(path, tok);
    }
  };
  if (root.color !== undefined) {
    group('color', (p, t) => {
      if (!normalizeHex(str(t.$value) ?? '')) out.push(`${p}: use a hex colour like #5B3DF5`);
    });
  }
  if (root.font !== undefined) {
    group('font', (p, t) => {
      if (!(FONT_ROLES as readonly string[]).includes(p.slice(5))) out.push(`${p}: font roles are display, body and mono`);
      const s = str(t.$value);
      if (!s || !s.trim()) out.push(`${p}: a font family stack is required`);
      else if (s.length > MAX_STACK) out.push(`${p}: longer than ${MAX_STACK} characters`);
      else if (UNSAFE_CSS.test(s)) out.push(`${p}: may not contain ; { } < > \\ or line breaks`);
      if (t.weights !== undefined && (!Array.isArray(t.weights) || t.weights.length > 9 || t.weights.some((w) => weight(w) == null))) {
        out.push(`${p}.weights: up to 9 integers from 1 to 1000`);
      }
    });
  }
  if (root.type !== undefined && root.type !== 'otto-brand') {
    group('type', (p, t) => {
      if (!(px(t.size) ?? 0)) out.push(`${p}.size: a positive px number`);
      if (!(px(t.line) ?? 0)) out.push(`${p}.line: a positive px number`);
      if (weight(t.weight) == null) out.push(`${p}.weight: an integer from 1 to 1000`);
    });
  }
  for (const g of ['radius', 'space']) {
    if (root[g] !== undefined) {
      group(g, (p, t) => {
        if (px(t.$value) == null) out.push(`${p}: a px number from 0 to ${MAX_PX}`);
      });
    }
  }
  if (root.logos !== undefined) {
    if (!Array.isArray(root.logos)) out.push('logos: must be an array');
    else {
      if (root.logos.length > 16) out.push('logos: at most 16 logos');
      root.logos.slice(0, 16).forEach((l, i) => {
        const o = rec(l);
        if (!o) return void out.push(`logos[${i}]: must be an object`);
        if (!str(o.name)?.trim()) out.push(`logos[${i}].name: a name is required`);
        if (!(LOGO_KINDS as readonly string[]).includes(str(o.kind) ?? '')) out.push(`logos[${i}].kind: full, mark or mono`);
        const a = o.asset;
        if (a != null && a !== '' && !(typeof a === 'string' && isValidAsset(a))) out.push(`logos[${i}].asset: an otto://design/<id> reference`);
      });
    }
  }
  for (const g of ['voice', 'imagery']) {
    if (root[g] === undefined) continue;
    const o = rec(root[g]);
    if (!o) {
      out.push(`${g}: must be an object`);
      continue;
    }
    if (o.summary !== undefined && (typeof o.summary !== 'string' || o.summary.length > MAX_TEXT)) out.push(`${g}.summary: up to ${MAX_TEXT} characters`);
    for (const k of ['do', 'dont']) {
      const l = o[k];
      if (l !== undefined && !(Array.isArray(l) && l.length <= MAX_LIST_ITEMS && l.every((x) => typeof x === 'string' && x.length <= MAX_TEXT))) {
        out.push(`${g}.${k}: up to ${MAX_LIST_ITEMS} short lines`);
      }
    }
  }
  return out;
}

/** An `otto://design/<id>[@…][#…]` reference or `blob:<sha256>`. */
export function isValidAsset(s: string): boolean {
  if (s.startsWith(DESIGN_PREFIX)) return /^otto:\/\/design\/[A-Za-z0-9_-]{1,64}(@(approved|latest|v[0-9]+))?(#[A-Za-z0-9_:.-]{1,128})?$/.test(s);
  return /^blob:[0-9a-f]{64}$/.test(s);
}

// ── Normalizing for the editor ───────────────────────────────────────────────

function pxOf(v: unknown): number | null {
  if (typeof v === 'number') return px(v);
  if (typeof v === 'string') {
    const m = /^\s*(-?[0-9]*\.?[0-9]+)\s*(px)?\s*$/.exec(v);
    return m ? px(Number(m[1])) : null;
  }
  return null;
}

/**
 * A kit document ready to edit: `$schema` set, every group present, loose
 * values tidied (`"14px"` → 14, a bare `"#fff"` colour → `{$value}`), and the
 * Phase 0 scaffold shape (`type: "otto-brand"`, `typography.*.$value`) moved
 * into `font` / `type`. Unknown top-level keys are kept, after the known ones.
 */
export function normalizeBrandDoc(raw: unknown, fallbackName = 'Brand kit'): BrandDoc {
  const r = rec(raw) ?? {};
  const legacy = typeof r.type === 'string';
  const color: Rec = {};
  for (const [k, v] of Object.entries(rec(r.color) ?? {})) {
    color[k] = typeof v === 'string' ? { $value: v } : v;
  }
  const font: Rec = { ...(rec(r.font) ?? {}) };
  let type: Rec | undefined = rec(r.type) ? { ...(r.type as Rec) } : undefined;
  const typography = rec(r.typography);
  if (!type && typography) {
    type = {};
    for (const [k, v] of Object.entries(typography)) {
      const val = rec(rec(v)?.$value);
      if (!val) continue;
      const size = pxOf(val.fontSize);
      const line = pxOf(val.lineHeight);
      const w = typeof val.fontWeight === 'number' ? val.fontWeight : Number(val.fontWeight);
      if (size && line && weight(w)) type[k] = { size, line, weight: w };
      const fam = str(val.fontFamily);
      const role = fontRoleForStyle(k);
      if (fam && !font[role]) font[role] = { $value: cleanCssValue(fam) };
    }
  }
  const pxGroup = (v: unknown): Rec => {
    const out: Rec = {};
    for (const [k, t] of Object.entries(rec(v) ?? {})) {
      const o = rec(t);
      const n = o ? pxOf(o.$value) : pxOf(t);
      if (n == null) {
        out[k] = t; // keep it; validation says what's wrong
        continue;
      }
      const d = o ? str(o.$description) : null;
      out[k] = d ? { $value: n, $description: d } : { $value: n };
    }
    return out;
  };
  const known: Rec = {
    $schema: BRAND_SCHEMA,
    name: str(r.name)?.trim() || fallbackName,
    color,
    font,
    ...(type ? { type } : {}),
    radius: pxGroup(r.radius),
    space: pxGroup(r.space),
    logos: Array.isArray(r.logos) ? r.logos : [],
    ...(rec(r.imagery) ? { imagery: r.imagery } : {}),
    ...(rec(r.voice) ? { voice: r.voice } : {}),
  };
  const skip = new Set([...Object.keys(known), 'type', 'typography', ...(legacy ? ['version'] : [])]);
  for (const [k, v] of Object.entries(r)) if (!skip.has(k)) known[k] = v;
  return known as unknown as BrandDoc;
}

/** Parse a kit's source; null when it isn't a JSON object. */
export function parseBrandDoc(text: string, fallbackName?: string): BrandDoc | null {
  try {
    const v: unknown = JSON.parse(text);
    return rec(v) ? normalizeBrandDoc(v, fallbackName) : null;
  } catch {
    return null;
  }
}

/** The stored form (stable, pretty-printed). */
export function serializeBrandDoc(doc: BrandDoc): string {
  return JSON.stringify(doc, null, 2) + '\n';
}

// ── Editing helpers (return new objects; key order kept) ─────────────────────

/** Rename a key of a token group, keeping its position. */
export function renameKey<T>(group: Record<string, T>, from: string, to: string): Record<string, T> {
  if (from === to || !(from in group) || to in group) return group;
  const out: Record<string, T> = {};
  for (const [k, v] of Object.entries(group)) out[k === from ? to : k] = v;
  return out;
}

/** `base`, else `base-2`, `base-3` … — the first name not in `taken`. */
export function uniqueName(base: string, taken: Iterable<string>): string {
  const set = new Set(taken);
  if (!set.has(base)) return base;
  for (let i = 2; ; i++) if (!set.has(`${base}-${i}`)) return `${base}-${i}`;
}

const BRAND_WORDS = /\b(brand|tokens?|colou?rs?|contrast|palette|fonts?|typeface|typography|radius|radii|spacing)\b/i;

function escapeRe(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/**
 * The learned team rules that talk about this kit: they mention brand words
 * (colour, font, contrast, spacing …) or one of the kit's token names / colour
 * roles ("amber is never text" → an `accent` colour named in a description
 * doesn't count; the word must appear in the rule).
 */
export function rulesForKit<T extends { rule: string }>(rules: readonly T[], doc: unknown): T[] {
  const names = TOKEN_GROUPS.flatMap((g) => Object.keys(rec(rec(doc)?.[g]) ?? {}))
    .filter((n) => n.length >= 3)
    .map((n) => new RegExp(`\\b${escapeRe(cssIdent(n).replace(/-/g, ' '))}\\b|\\b${escapeRe(n)}\\b`, 'i'));
  return rules.filter((r) => BRAND_WORDS.test(r.rule) || names.some((re) => re.test(r.rule)));
}

/** A readable label for a token key: `color.surface-alt` → `Surface alt`. */
export function tokenLabel(name: string): string {
  const words = cssIdent(name).split('-').filter(Boolean).join(' ');
  return words ? words.charAt(0).toUpperCase() + words.slice(1) : name;
}
