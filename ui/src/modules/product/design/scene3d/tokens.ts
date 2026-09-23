// Brand-token colours for scene3d v2 — `token:color.<name>` references resolved
// against an `otto-brand` document (DTCG-style `color.<name>.$value`, nested
// groups and `{color.other}` aliases). Pure (no imports but types) so the
// viewer, the embed runtime and the unit tests share it.
//
// Brand Kit v1 owns the canonical resolver (`design-hall/brand/tokens.ts →
// resolveToken(doc, ref)`); this is the same signature as a local fallback so
// 3D keeps working without it. When both exist, prefer the Brand Kit one.

/** Everything after `token:` — the only non-hex colour syntax the doc allows. */
export const TOKEN_PREFIX = 'token:';
const TOKEN_RE = /^token:color\.([A-Za-z0-9_.-]{1,96})$/;
const HEX_RE = /^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/;
/** Alias hops before we give up (a → b → a loops stop here). */
const MAX_ALIAS_HOPS = 8;

/** `token:color.violet` → true. */
export function isTokenRef(v: unknown): v is string {
  return typeof v === 'string' && TOKEN_RE.test(v);
}

/** `token:color.brand.violet` → `brand.violet` (null when not a token). */
export function tokenName(ref: string): string | null {
  const m = TOKEN_RE.exec(ref);
  return m ? m[1] : null;
}

/** `violet` → `token:color.violet`. */
export function tokenRef(name: string): string {
  return `${TOKEN_PREFIX}color.${name}`;
}

/** `#abc` / `#AABBCC` / `#aabbccdd` → `#aabbcc`; anything else → null. */
export function normalizeHex(v: unknown): string | null {
  if (typeof v !== 'string' || !HEX_RE.test(v)) return null;
  let h = v.slice(1).toLowerCase();
  if (h.length === 3) h = h.split('').map((c) => c + c).join('');
  return `#${h.slice(0, 6)}`;
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

/** Walk `a.b.c` from `root`; a DTCG token is `{ $value }`, a bare value is accepted too. */
function lookup(root: unknown, path: string): unknown {
  let cur: unknown = root;
  for (const part of path.split('.')) {
    if (!isRecord(cur)) return undefined;
    cur = cur[part];
  }
  return isRecord(cur) && '$value' in cur ? cur.$value : cur;
}

/**
 * Resolve a colour token against a brand document.
 *
 * `ref` may be `token:color.<name>` or a bare `color.<name>` path. Returns a
 * `#rrggbb` string, or null when the kit has no such colour (or it is not a
 * colour). DTCG aliases (`"{color.primary}"`) are followed.
 */
export function resolveToken(doc: unknown, ref: string): string | null {
  if (!isRecord(doc)) return null;
  let path = ref.startsWith(TOKEN_PREFIX) ? ref.slice(TOKEN_PREFIX.length) : ref;
  for (let hop = 0; hop <= MAX_ALIAS_HOPS; hop++) {
    const v = lookup(doc, path);
    const hex = normalizeHex(v);
    if (hex) return hex;
    const alias = typeof v === 'string' ? /^\{([A-Za-z0-9_.-]+)\}$/.exec(v.trim()) : null;
    if (!alias) return null;
    path = alias[1];
  }
  return null;
}

export interface BrandSwatch {
  /** Token name under `color.` (`violet`, `brand.violet`). */
  name: string;
  /** Resolved `#rrggbb`. */
  value: string;
  /** What a document stores: `token:color.<name>`. */
  ref: string;
}

/** Every colour token of a brand document, flattened in document order (≤ 64). */
export function brandSwatches(doc: unknown, limit = 64): BrandSwatch[] {
  if (!isRecord(doc) || !isRecord(doc.color)) return [];
  const out: BrandSwatch[] = [];
  const walk = (node: Record<string, unknown>, prefix: string, depth: number) => {
    for (const [k, v] of Object.entries(node)) {
      if (out.length >= limit || k.startsWith('$')) continue;
      const name = prefix ? `${prefix}.${k}` : k;
      if (!/^[A-Za-z0-9_.-]{1,96}$/.test(name)) continue;
      const leaf = isRecord(v) && '$value' in v;
      if (leaf || typeof v === 'string') {
        const value = resolveToken(doc, `color.${name}`);
        if (value) out.push({ name, value, ref: tokenRef(name) });
      } else if (isRecord(v) && depth < 4) {
        walk(v, name, depth + 1);
      }
    }
  };
  walk(doc.color, '', 0);
  return out;
}

/**
 * The colour the renderer uses for a document value: hex as-is (normalized), a
 * token resolved against `brand`, else `fallback` (a missing kit or token
 * never breaks the render).
 */
export function resolveColor(value: string | undefined | null, brand: unknown, fallback: string): string {
  if (!value) return fallback;
  if (value.startsWith(TOKEN_PREFIX)) return resolveToken(brand, value) ?? fallback;
  return normalizeHex(value) ?? fallback;
}

/** Human label for a colour value (`Brand violet` / `#5b3df5`). */
export function colorLabel(value: string | undefined): string {
  if (!value) return 'Default';
  const n = tokenName(value);
  if (!n) return value;
  const last = n.split('.').pop() ?? n;
  return `Brand ${last.replace(/[-_]+/g, ' ')}`;
}
