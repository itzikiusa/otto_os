// ─────────────────────────────────────────────────────────────────────────────
// Otto brand + UI theme for the walkthrough videos.
//
// These values are copied 1:1 from the real app so the in-video mockups read as
// genuine Otto, not an approximation:
//   • tokens        → ui/src/lib/tokens.css   (native / pro-dark / warm themes)
//   • status colors → --status-working/idle/exited
//   • fonts         → --font-ui / --font-mono
//   • logo palette  → ui/public/favicon.svg   (purple #863bff, cyan glints #47bfff)
//
// The cinematic wrapper (title cards, captions, backgrounds, kinetic type) uses
// the BRAND identity — the purple→cyan of the actual mark — while the app
// mockups render in a real Otto theme (native dark by default). That keeps the
// product surfaces honest and the storytelling distinctive.
// ─────────────────────────────────────────────────────────────────────────────

export const VIDEO = { width: 1920, height: 1080, fps: 30 } as const;

export const fonts = {
  ui: '-apple-system, BlinkMacSystemFont, "SF Pro Display", "SF Pro Text", "Inter", "Helvetica Neue", sans-serif',
  mono: 'ui-monospace, "SF Mono", "JetBrains Mono", "Menlo", monospace',
} as const;

export const radius = { s: 5, m: 8, l: 12, xl: 18 } as const;

// Status colors — identical across every Otto theme.
export const status = {
  working: '#28c840', // green, pulses while an agent runs
  idle: '#98989f', // gray
  exited: '#ff5f57', // red / traffic-light close
  needsYou: '#febc2e', // amber — blocked on the operator
  reconnectable: '#febc2e',
} as const;

// macOS traffic lights.
export const traffic = { close: '#ff5f57', min: '#febc2e', max: '#28c840' } as const;

// The signature high-contrast selection used on the real nav's active row.
export const navActive = { bg: '#7ee787', fg: '#0a0a0a', edge: '#2ea043' } as const;

// ── App themes (verbatim from tokens.css) ──────────────────────────────────
export interface Theme {
  name: string;
  scheme: 'dark' | 'light';
  bg: string;
  bgSidebar: string;
  surface: string;
  surface2: string;
  border: string;
  text: string;
  textDim: string;
  termBg: string;
  shadow: string;
  accent: string;
  accentContrast: string;
}

export const themes = {
  nativeDark: {
    name: 'native', scheme: 'dark',
    bg: '#1e1e23', bgSidebar: '#232328', surface: '#2a2a30', surface2: '#323238',
    border: 'rgba(255,255,255,0.1)', text: '#f2f2f5', textDim: '#98989f',
    termBg: '#131318', shadow: '0 8px 30px rgba(0,0,0,0.45)',
    accent: '#0a84ff', accentContrast: '#ffffff',
  },
  nativeLight: {
    name: 'native', scheme: 'light',
    bg: '#f5f5f7', bgSidebar: '#ececf1', surface: '#ffffff', surface2: '#f0f0f4',
    border: 'rgba(0,0,0,0.1)', text: '#1d1d1f', textDim: '#6e6e73',
    termBg: '#fbfbfd', shadow: '0 8px 30px rgba(0,0,0,0.12)',
    accent: '#0a84ff', accentContrast: '#ffffff',
  },
  proDark: {
    name: 'pro-dark', scheme: 'dark',
    bg: '#16161c', bgSidebar: '#1c1c23', surface: '#1f1f27', surface2: '#26262f',
    border: 'rgba(255,255,255,0.08)', text: '#e8e8ee', textDim: '#9a9aa8',
    termBg: '#0f0f14', shadow: '0 8px 30px rgba(0,0,0,0.55)',
    accent: '#6c5ce7', accentContrast: '#ffffff',
  },
  warmLight: {
    name: 'warm', scheme: 'light',
    bg: '#faf9f7', bgSidebar: '#f3f1ed', surface: '#ffffff', surface2: '#f0ede8',
    border: 'rgba(60,50,30,0.12)', text: '#3d3a35', textDim: '#6b6760',
    termBg: '#fbfaf7', shadow: '0 8px 30px rgba(60,50,30,0.14)',
    accent: '#0f9d58', accentContrast: '#ffffff',
  },
  warmDark: {
    name: 'warm', scheme: 'dark',
    bg: '#211f1b', bgSidebar: '#262420', surface: '#2c2a25', surface2: '#33312b',
    border: 'rgba(255,245,225,0.1)', text: '#e8e4da', textDim: '#a09a8e',
    termBg: '#1a1916', shadow: '0 8px 30px rgba(0,0,0,0.5)',
    accent: '#2bb673', accentContrast: '#ffffff',
  },
} satisfies Record<string, Theme>;

// Default theme used by the app mockups.
export const T: Theme = themes.nativeDark;

// ── Brand identity (the purple→cyan of the real Otto mark) ──────────────────
export const brand = {
  purple: '#863bff', // primary mark fill (display-p3 .5252 .23 1)
  purpleDeep: '#7e14ff', // glow lobes
  violet: '#6c5ce7', // pro-dark accent / brand mid
  cyan: '#47bfff', // bright glint in the mark
  ink: '#0b0b10', // cinematic backdrop base
  ink2: '#111118',
  mist: '#ede6ff', // pale highlight from the mark
  grad: 'linear-gradient(120deg, #863bff 0%, #6c5ce7 42%, #47bfff 100%)',
  gradSoft: 'linear-gradient(120deg, #a06bff 0%, #6c5ce7 50%, #5fcaff 100%)',
  glow: '#7e14ff',
} as const;

// Cinematic background — a deep, slightly purple void with two brand auroras.
export const cinematicBg =
  'radial-gradient(1300px 760px at 72% -12%, #241844 0%, rgba(20,12,32,0) 55%),' +
  'radial-gradient(1100px 700px at 8% 116%, #102744 0%, rgba(8,12,22,0) 52%),' +
  'linear-gradient(180deg, #0c0a14 0%, #08080e 100%)';

// Per-provider accent colors for chips/tiles (agents, swarm, review lenses).
export const providers = {
  claude: '#d97757', // Anthropic clay
  codex: '#10a37f', // OpenAI green
  agy: '#8ab4f8', // Antigravity (Google) blue
  gemini: '#a78bfa', // Gemini violet
  shell: '#0a84ff', // neutral Otto blue
} as const;

// A small categorical palette for charts / multi-series accents.
export const series = ['#0a84ff', '#28c840', '#febc2e', '#bf7aff', '#47bfff', '#ff8a65'] as const;

// ── color helpers (string-based; safe for inline styles) ────────────────────
/** Hex (#rgb / #rrggbb) → "r,g,b". Falls back to a mid-gray for non-hex. */
function rgbTriplet(hex: string): string {
  let h = hex.trim();
  if (h[0] !== '#') return '150,150,160';
  h = h.slice(1);
  if (h.length === 3) h = h.split('').map((c) => c + c).join('');
  const n = parseInt(h.slice(0, 6), 16);
  return `${(n >> 16) & 255},${(n >> 8) & 255},${n & 255}`;
}

/** rgba() from a hex color + alpha (0–1). */
export const alpha = (hex: string, a: number): string => `rgba(${rgbTriplet(hex)},${a})`;
