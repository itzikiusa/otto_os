// Shared roving-tabindex keyboard handler for SVG bar charts (ArrowLeft /
// ArrowRight / Home / End + Enter / Space). Kept out of UsagePage.svelte so
// both charts share one accessible implementation and unit tests can cover it.

export function handleBarChartKey(
  event: KeyboardEvent,
  keys: readonly string[],
  idx: number,
  attrName: string,
  onSelect: (key: string) => void,
): void {
  if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault();
    if (keys[idx]) onSelect(keys[idx]);
    return;
  }
  let nextIdx = -1;
  if (event.key === 'ArrowLeft') nextIdx = Math.max(0, idx - 1);
  else if (event.key === 'ArrowRight') nextIdx = Math.min(keys.length - 1, idx + 1);
  else if (event.key === 'Home') nextIdx = 0;
  else if (event.key === 'End') nextIdx = keys.length - 1;
  if (nextIdx < 0 || nextIdx === idx) return;
  event.preventDefault();
  const nextKey = keys[nextIdx];
  if (!nextKey) return;
  onSelect(nextKey);
  const svg = (event.currentTarget as Element | null)?.closest('svg');
  const escaped = typeof CSS !== 'undefined' && typeof CSS.escape === 'function' ? CSS.escape(nextKey) : nextKey;
  svg?.querySelector<SVGRectElement>(`[${attrName}="${escaped}"]`)?.focus();
}
