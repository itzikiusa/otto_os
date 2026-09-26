// Contrast-safe fill for a user-picked custom accent (Settings → Appearance).
// Theme accents have their `--accent-solid` / `--accent-contrast` pair tuned
// in tokens.css; a custom accent can be any colour, so pick the pair here:
// the accent darkened 16% under white text when that clears WCAG AA (4.5:1),
// otherwise the accent itself under near-black text (bright yellows/greens).

const DARK_TEXT = '#111111';

function parseHex(hex: string): [number, number, number] | null {
  const m = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const h = m[1].length === 3 ? [...m[1]].map((c) => c + c).join('') : m[1];
  return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255) as [number, number, number];
}

function luminance([r, g, b]: [number, number, number]): number {
  const lin = (c: number) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

function ratio(a: number, b: number): number {
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

/** `--accent-solid` / `--accent-contrast` values for `hex`, or null if unparsable. */
export function accentFill(hex: string): { solid: string; contrast: string } | null {
  const rgb = parseHex(hex);
  if (!rgb) return null;
  const darkened = luminance(rgb.map((c) => c * 0.84) as [number, number, number]);
  if (ratio(1, darkened) >= 4.5) {
    return { solid: `color-mix(in srgb, ${hex} 84%, black)`, contrast: '#ffffff' };
  }
  const own = luminance(rgb);
  const dark = luminance(parseHex(DARK_TEXT)!);
  // Whichever reads better on the undarkened accent (dark text wins for any
  // accent bright enough to have failed the white check above).
  return ratio(own, dark) >= ratio(1, own)
    ? { solid: hex, contrast: DARK_TEXT }
    : { solid: `color-mix(in srgb, ${hex} 84%, black)`, contrast: '#ffffff' };
}

/** Preserve as much custom hue as possible while keeping small accent labels
 * readable on the theme's solid surfaces and accent-tinted selections. */
export function accentText(hex: string, text: string, surfaces: string[]): string | null {
  const accent = parseHex(hex);
  const foreground = parseHex(text);
  const backgrounds = surfaces.map(parseHex).filter((v): v is [number, number, number] => v !== null);
  if (!accent || !foreground || backgrounds.length === 0) return null;
  const mix = (a: number[], b: number[], weight: number): [number, number, number] =>
    a.map((v, i) => v * weight + b[i] * (1 - weight)) as [number, number, number];
  const candidates = [...backgrounds, ...backgrounds.map((bg) => mix(accent, bg, 0.14))];
  for (let percent = 62; percent >= 0; percent--) {
    const color = mix(accent, foreground, percent / 100);
    if (candidates.every((bg) => ratio(luminance(color), luminance(bg)) >= 4.6)) {
      return `color-mix(in srgb, ${hex} ${percent}%, ${text})`;
    }
  }
  return text;
}
