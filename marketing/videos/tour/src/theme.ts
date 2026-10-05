// Film palette — lifted from ui/src/lib/tokens.css (native theme, dark scheme:
// --accent, the semantic colours and the --cat-* graphic series) so the film
// reads as the same product as the footage inside it.
export const C = {
  accent: '#0a84ff',
  accentText: '#6cb2ff',
  bg: '#141417',
  bgDeep: '#08080c',
  surface: '#202024',
  surface2: '#2a2a2f',
  border: 'rgba(255,255,255,0.10)',
  text: '#f2f2f5',
  textDim: '#a1a1aa',
  success: '#56d06c',
  warning: '#e3b341',
  danger: '#ff8a80',
  info: '#6cb2ff',
  // --cat-1 … --cat-6
  blue: '#6cb2ff',
  amber: '#e3b341',
  green: '#3fcf8e',
  violet: '#b18cff',
  pink: '#ff8fb1',
  cyan: '#5ad1d1',
};

export const FONT = "-apple-system, BlinkMacSystemFont, 'SF Pro Display', 'SF Pro Text', 'Helvetica Neue', sans-serif";
export const MONO = "ui-monospace, 'SF Mono', SFMono-Regular, Menlo, monospace";

/** Sidebar groups (ui/src/lib/sidebar.ts) — each tints its chapters. */
export const GROUP_TINT: Record<string, string> = {
  Work: C.accent,
  Automate: C.violet,
  Build: C.green,
  Infrastructure: C.amber,
  Insight: C.pink,
  Shell: C.blue,
  Everywhere: C.cyan,
};

/** The two backdrop companions of each tint (an analogous glow + a contrast). */
export const GROUP_GLOW: Record<string, [string, string]> = {
  Work: [C.violet, C.cyan],
  Automate: [C.pink, C.accent],
  Build: [C.cyan, C.accent],
  Infrastructure: [C.pink, C.violet],
  Insight: [C.violet, C.amber],
  Shell: [C.violet, C.cyan],
  Everywhere: [C.accent, C.violet],
};

/** `#rrggbb` → `rgba(r,g,b,a)`. */
export function rgba(hex: string, a: number): string {
  const n = parseInt(hex.slice(1), 16);
  return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${a})`;
}

/** Linear blend of two `#rrggbb` colours. */
export function mix(a: string, b: string, k: number): string {
  const pa = parseInt(a.slice(1), 16);
  const pb = parseInt(b.slice(1), 16);
  const ch = (s: number) => Math.round(((pa >> s) & 255) * (1 - k) + ((pb >> s) & 255) * k);
  return `#${((ch(16) << 16) | (ch(8) << 8) | ch(0)).toString(16).padStart(6, '0')}`;
}
