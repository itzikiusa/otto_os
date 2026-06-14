// Otto brand theme for the walkthrough videos.
export const theme = {
  bg: '#0a0c10',
  bgGradient: 'radial-gradient(1200px 700px at 70% -10%, #16203a 0%, #0a0c10 55%)',
  surface: '#11151c',
  surface2: '#171c26',
  border: '#222a36',
  text: '#e8edf4',
  textDim: '#8b97a8',
  accent: '#3d5bff', // Otto blue (logo)
  accent2: '#9ee039', // terminal green
  danger: '#e5534b',
  warn: '#febc2e',
  // status colors
  working: '#9ee039',
  idle: '#8b97a8',
  font: '-apple-system, "SF Pro Display", "Inter", "Segoe UI", system-ui, sans-serif',
  mono: '"SF Mono", "JetBrains Mono", "Menlo", monospace',
} as const;

export const VIDEO = { width: 1920, height: 1080, fps: 30 } as const;
