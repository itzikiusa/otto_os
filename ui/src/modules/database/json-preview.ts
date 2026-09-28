// Bounded JSON previews for grid cells. Pure (no Svelte, no imports) so the
// node unit tests can load it directly.

/** Hard cap on the text ONE grid cell puts in the DOM (see results-format's
 *  `clip`). The preview writer below stops right after it. */
export const CELL_MAX = 512;

// Bounded JSON preview caches, per object: a scrolled-in row re-renders its
// cells, and a 50 KB document must not be re-serialized for every appearance.
const compactPreviews = new WeakMap<object, string>();
const prettyPreviews = new WeakMap<object, string>();

/** Thrown internally once a preview has enough characters. */
const PREVIEW_FULL = Symbol('preview-full');

/**
 * The first `max + 1` characters of `JSON.stringify(v)` (or of the 2-space
 * pretty form) WITHOUT serializing the rest: the writer stops as soon as the
 * preview is long enough, so a 50 KB document costs ~`max` characters of work
 * instead of 50 KB. Short values come back whole and identical to
 * `compactJson`/`prettyJson`, so `clip(previewJson(v))` renders exactly what
 * `clip(compactJson(v))` did. Cached per object for the default `max`. Copy,
 * the cell viewer and export keep using the full serializers.
 */
export function previewJson(v: unknown, max = CELL_MAX, pretty = false): string {
  const cacheable = max === CELL_MAX && v !== null && typeof v === 'object';
  const cache = pretty ? prettyPreviews : compactPreviews;
  if (cacheable) {
    const hit = cache.get(v as object);
    if (hit !== undefined) return hit;
  }
  const limit = max + 1;
  let out = '';
  const push = (s: string): void => {
    out += s;
    if (out.length >= limit) throw PREVIEW_FULL;
  };
  const nl = (depth: number): string => (pretty ? '\n' + '  '.repeat(depth) : '');
  const write = (x: unknown, depth: number, inArray: boolean): boolean => {
    if (x !== null && typeof x === 'object' && typeof (x as { toJSON?: unknown }).toJSON === 'function') {
      x = (x as { toJSON: () => unknown }).toJSON();
    }
    switch (typeof x) {
      case 'string':
        // Only the head can reach the preview; escaping a slice keeps the cost
        // bounded (a cut surrogate pair lands past the limit, never shown).
        push(JSON.stringify(x.length > limit ? x.slice(0, limit) : x));
        return true;
      case 'number':
        push(Number.isFinite(x) ? String(x) : 'null');
        return true;
      case 'boolean':
        push(String(x));
        return true;
      case 'bigint':
        push(String(x));
        return true;
      case 'undefined':
      case 'function':
      case 'symbol':
        if (inArray) push('null');
        return inArray;
    }
    if (x === null) {
      push('null');
      return true;
    }
    if (depth > 64) {
      push('…');
      return true;
    }
    if (Array.isArray(x)) {
      if (x.length === 0) {
        push('[]');
        return true;
      }
      push('[');
      x.forEach((item, i) => {
        if (i > 0) push(',');
        push(nl(depth + 1));
        write(item, depth + 1, true);
      });
      push(nl(depth) + ']');
      return true;
    }
    const entries = Object.keys(x as object);
    let wrote = 0;
    push('{');
    for (const k of entries) {
      const val = (x as Record<string, unknown>)[k];
      const t = typeof val;
      if (t === 'undefined' || t === 'function' || t === 'symbol') continue;
      if (wrote > 0) push(',');
      push(nl(depth + 1) + JSON.stringify(k) + (pretty ? ': ' : ':'));
      write(val, depth + 1, false);
      wrote++;
    }
    push(wrote > 0 ? nl(depth) + '}' : '}');
    return true;
  };
  let preview: string;
  try {
    write(v, 0, false);
    preview = out;
  } catch (e) {
    if (e !== PREVIEW_FULL) {
      // What JSON.stringify would do with it (cycles, a throwing toJSON):
      // the full serializers' fallback.
      try {
        return (pretty ? JSON.stringify(v, null, 2) : JSON.stringify(v)) ?? String(v);
      } catch {
        return String(v);
      }
    }
    preview = out.slice(0, limit);
  }
  if (cacheable) cache.set(v as object, preview);
  return preview;
}
