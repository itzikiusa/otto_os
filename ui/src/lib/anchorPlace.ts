// Place a fixed popover against its trigger, CLAMPED into the viewport with a
// capped height (AGENTS.md floating-UI rule): it prefers `prefer`, takes the
// roomier side when that doesn't fit, never leaves the 8 px margin, and its
// `maxH` keeps every row reachable by scrolling. Logical `align` (RTL-aware).
export interface AnchorPlacement {
  top: number;
  left: number;
  maxH: number;
}

export function placeAnchored(
  anchor: Element,
  panel: HTMLElement,
  opts: { prefer?: 'above' | 'below'; align?: 'start' | 'end'; gap?: number } = {},
): AnchorPlacement {
  const margin = 8;
  const gap = opts.gap ?? 6;
  const a = anchor.getBoundingClientRect();
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const w = panel.offsetWidth;
  const h = panel.scrollHeight;
  const roomAbove = a.top - gap - margin;
  const roomBelow = vh - a.bottom - gap - margin;
  const preferAbove = (opts.prefer ?? 'below') === 'above';
  const above = preferAbove
    ? roomAbove >= Math.min(h, 360) || roomAbove > roomBelow
    : !(roomBelow >= Math.min(h, 360) || roomBelow >= roomAbove);
  const maxH = Math.max(160, above ? roomAbove : roomBelow);
  const height = Math.min(h, maxH);
  let top = above ? a.top - gap - height : a.bottom + gap;
  top = Math.max(margin, Math.min(top, vh - margin - height));
  const rtl = getComputedStyle(anchor).direction === 'rtl';
  const end = (opts.align ?? 'start') === 'end';
  let left = end !== rtl ? a.right - w : a.left;
  left = Math.max(margin, Math.min(left, vw - margin - w));
  return { top, left, maxH };
}
