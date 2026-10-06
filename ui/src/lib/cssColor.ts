// CSS token → numeric colour, for canvases that can't take `var(--x)` (three.js
// materials, 2D canvas fills). Tokens are often not plain hex — `--border` is a
// `color-mix()`, a custom accent is set inline — so they are RESOLVED by the
// browser through a probe element (`color: var(--x)` → its computed `color`)
// and the computed string is parsed here. Modern engines serialize a resolved
// `color-mix()` as `color(srgb r g b / a)`, everything else as `rgb()/rgba()`.
//
// `parseCssColor` is pure (node:test covers it, unit/cssColor.test.ts); the
// reader touches the DOM and is called again on every theme change.

export interface Rgba {
  /** 0–1 channels (sRGB, not linearized). */
  r: number;
  g: number;
  b: number;
  a: number;
}

const clamp01 = (n: number): number => (Number.isFinite(n) ? Math.min(1, Math.max(0, n)) : 0);

/** One channel: `128`, `50%`, `0.5` (for `color(srgb …)`). */
function channel(tok: string, unit: 255 | 1): number {
  const t = tok.trim();
  if (t.endsWith('%')) return clamp01(parseFloat(t) / 100);
  return clamp01(parseFloat(t) / unit);
}

function alpha(tok: string | undefined): number {
  if (tok == null || tok.trim() === '') return 1;
  return channel(tok, 1);
}

/** `rgb(1, 2, 3)`, `rgba(1 2 3 / .5)`, `color(srgb .1 .2 .3 / .5)`, `#abc`,
 *  `#aabbcc(dd)`, `transparent` → channels; anything else → null. */
export function parseCssColor(input: string | null | undefined): Rgba | null {
  if (!input) return null;
  const s = input.trim().toLowerCase();
  if (s === 'transparent') return { r: 0, g: 0, b: 0, a: 0 };
  const hex = /^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/.exec(s);
  if (hex) {
    let h = hex[1];
    if (h.length <= 4) h = h.split('').map((c) => c + c).join('');
    const n = (i: number) => parseInt(h.slice(i, i + 2), 16) / 255;
    return { r: n(0), g: n(2), b: n(4), a: h.length === 8 ? n(6) : 1 };
  }
  const fn = /^(rgba?|color)\((.*)\)$/.exec(s);
  if (!fn) return null;
  let body = fn[2].trim();
  let unit: 255 | 1 = 255;
  if (fn[1] === 'color') {
    // Only the sRGB space maps 1:1 onto a canvas colour.
    if (!body.startsWith('srgb ')) return null;
    body = body.slice(5);
    unit = 1;
  }
  const [rgbPart, aPart] = body.split('/');
  const parts = rgbPart.split(/[\s,]+/).filter(Boolean);
  if (parts.length < 3) return null;
  const a = aPart !== undefined ? alpha(aPart) : parts.length >= 4 ? alpha(parts[3]) : 1;
  return { r: channel(parts[0], unit), g: channel(parts[1], unit), b: channel(parts[2], unit), a };
}

/** Resolve CSS custom properties to computed colours, in `host`'s context (so
 *  a scoped `[data-scheme]` / inline accent applies). Missing tokens → null. */
export function readTokenColors<K extends string>(names: readonly K[], host: Element = document.documentElement): Record<K, Rgba | null> {
  const probe = document.createElement('span');
  probe.style.display = 'none';
  host.appendChild(probe);
  const out = {} as Record<K, Rgba | null>;
  try {
    for (const name of names) {
      probe.style.color = '';
      probe.style.color = `var(${name})`;
      out[name] = parseCssColor(getComputedStyle(probe).color);
    }
  } finally {
    probe.remove();
  }
  return out;
}
