// Film palette — lifted from ui/src/lib/tokens.css (native theme, dark scheme)
// so the film reads as the same product as the footage inside it.
export const C = {
  accent: '#0a84ff',
  accentText: '#6cb2ff',
  bg: '#141417',
  bgDeep: '#0b0b0e',
  surface: '#202024',
  surface2: '#2a2a2f',
  border: 'rgba(255,255,255,0.10)',
  text: '#f2f2f5',
  textDim: '#a1a1aa',
  success: '#56d06c',
  warning: '#e3b341',
  danger: '#ff8a80',
  info: '#6cb2ff',
  purple: '#7c3aed',
  teal: '#0d9488',
  pink: '#db2777',
};

export const FONT = "-apple-system, BlinkMacSystemFont, 'SF Pro Display', 'SF Pro Text', 'Helvetica Neue', sans-serif";
export const MONO = "ui-monospace, 'SF Mono', SFMono-Regular, Menlo, monospace";

/** Sidebar groups (ui/src/lib/sidebar.ts) — tinted chips on chapter titles. */
export const GROUP_TINT: Record<string, string> = {
  Work: C.accent,
  Automate: C.purple,
  Build: C.success,
  Infrastructure: C.warning,
  Insight: C.pink,
  Shell: C.info,
  Everywhere: C.teal,
};
