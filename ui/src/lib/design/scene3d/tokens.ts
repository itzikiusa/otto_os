// Brand-token colours in scene3d v2 — the SYNTAX side only. A document stores a
// brand colour as `token:color.<name>` (the form Brand Kit's impact preview
// finds consumers by); resolving it against a kit is Brand Kit's job
// (`design-hall/brand/tokens.ts → resolveToken(doc, ref)`), which the Design
// Hall host binds into a `resolve` callback. The Product arena has no kit, so
// its tokens fall back to a neutral colour. Pure (no imports) so the viewer,
// the embed runtime and the unit tests share it.

export const TOKEN_PREFIX = 'token:';
/** Brand Kit's token-name grammar: `[A-Za-z0-9][A-Za-z0-9_-]{0,63}`. */
const TOKEN_RE = /^token:color\.([A-Za-z0-9][A-Za-z0-9_-]{0,63})$/;
const HEX_RE = /^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/;

/** `token:color.violet` → true. */
export function isTokenRef(v: unknown): v is string {
  return typeof v === 'string' && TOKEN_RE.test(v);
}

/** `token:color.violet` → `violet` (null when not a colour token). */
export function tokenName(ref: string): string | null {
  const m = TOKEN_RE.exec(ref);
  return m ? m[1] : null;
}

/** `#abc` / `#AABBCC` / `#aabbccdd` → `#aabbcc`; anything else → null. */
export function normalizeHex(v: unknown): string | null {
  if (typeof v !== 'string' || !HEX_RE.test(v)) return null;
  let h = v.slice(1).toLowerCase();
  if (h.length === 3) h = h.split('').map((c) => c + c).join('');
  return `#${h.slice(0, 6)}`;
}

/** A kit lookup: `token:color.<name>` → a hex colour, or null when the kit lacks it. */
export type TokenResolver = (ref: string) => string | null;

/**
 * The colour the renderer uses for a document value: hex as-is (normalized), a
 * token through `resolve` (the host's brand kit), else `fallback` — a missing
 * kit or token never breaks the render.
 */
export function resolveColor(value: string | undefined | null, resolve: TokenResolver | null, fallback: string): string {
  if (!value) return fallback;
  if (value.startsWith(TOKEN_PREFIX)) return normalizeHex(resolve?.(value) ?? null) ?? fallback;
  return normalizeHex(value) ?? fallback;
}

/** A kit colour offered as a swatch (the host builds these from the kit). */
export interface BrandSwatch {
  /** Token name (`violet`). */
  name: string;
  /** Resolved `#rrggbb`. */
  value: string;
  /** What a document stores: `token:color.<name>`. */
  ref: string;
}

/** Human label for a colour value (`Brand violet` / `#5b3df5`). */
export function colorLabel(value: string | undefined): string {
  if (!value) return 'Default';
  const n = tokenName(value);
  if (!n) return value;
  return `Brand ${n.replace(/[-_]+/g, ' ')}`;
}
