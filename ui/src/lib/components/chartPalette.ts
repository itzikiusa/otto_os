// The one categorical palette for hand-rolled SVG charts (database Chart,
// MetricChart). Values are the scheme-aware `--cat-*` tokens (lib/tokens.css):
// SVG `fill`/`stroke` and inline `style` resolve `var()` at paint time, so a
// scheme or theme switch recolours a drawn chart with no re-render. The tokens
// are graphics-only (>= 3:1 on --surface): always pair a series with its
// legend label. More than six series wrap around.
export const CHART_COLORS: readonly string[] = [
  'var(--cat-1)',
  'var(--cat-2)',
  'var(--cat-3)',
  'var(--cat-4)',
  'var(--cat-5)',
  'var(--cat-6)',
];

/** Colour of series `i`, wrapping past the palette's end. */
export function chartColor(i: number): string {
  return CHART_COLORS[((i % CHART_COLORS.length) + CHART_COLORS.length) % CHART_COLORS.length];
}
