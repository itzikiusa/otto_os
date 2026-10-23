// The git graph's viewport (arch-1-09: split out of GraphView.svelte): the
// phone/tablet breakpoint, the lane layout over the loaded history, the row
// window (virtualization) at a fixed pitch, scroll-driven paging and the
// jump-to-commit scroll that lands a row which isn't rendered yet. GraphView
// binds the scroller and the top spacer into it and renders `visibleRows`.

import { tick } from 'svelte';
import type { CommitInfo } from '../../../lib/api/types';
import { listRange, type ListRange } from '../../../lib/listWindow.svelte';
import { LANE_W, MAX_GUTTER_W, layoutLanes, type LaneMemo, type LaneRow } from '../graph-lanes';

export interface GraphViewportHost {
  readonly commits: CommitInfo[];
  readonly laneNames: Map<string, string>;
  readonly loadingMore: boolean;
  readonly hasMore: boolean;
  loadMore(): void;
  /** Replay an auto-fetch re-sync parked while the graph was hidden. */
  flushResync(): void;
}

export type GraphViewport = ReturnType<typeof createGraphViewport>;

/** Rows rendered beyond each edge, so scrolling isn't bare. */
export const OVERSCAN = 25;

export function createGraphViewport(h: GraphViewportHost) {
  // ── Mobile (phone) layout ─────────────────────────────────────────────────
  // On a phone the 3-pane desktop layout (refs | graph | diff) can't fit, so
  // the panes stack as COLLAPSIBLE accordion sections, each independently
  // scrollable. Tracked via matchMedia so the same component serves both
  // layouts without duplicating markup. The breakpoint is 1024px so it catches
  // phone portrait + landscape AND the tablet range (iPad portrait 834,
  // real-phone landscape 932) — the desktop 3-pane can't fit there without the
  // diff getting clipped off-screen. Desktop (≥1025) keeps the 3-pane view.
  let isMobile = $state(false);
  $effect(() => {
    const mq = window.matchMedia('(max-width: 1024px)');
    const sync = () => (isMobile = mq.matches);
    sync();
    mq.addEventListener('change', sync);
    return () => mq.removeEventListener('change', sync);
  });

  /** The scrolling commit list — also the element rows are looked up inside
   *  when jumping to a ref. */
  let panelEl = $state<HTMLDivElement | null>(null);
  /** The top spacer — its position gives the exact offset of the rows region
   *  inside the scroll content (sticky header + WIP row), so a jump can compute
   *  a scroll target for a row that isn't rendered yet. */
  let topSpacerEl = $state<HTMLDivElement | null>(null);

  // ── Row windowing (virtualization) ────────────────────────────────────────
  // Only the rows near the viewport are put in the DOM. Every row is a button +
  // an SVG lane gutter, so rendering a whole 6k-commit history cost seconds on
  // mount; with a window it is flat no matter how deep history goes. Rows are a
  // FIXED pitch, which is what makes the arithmetic (and the jump-to-ref
  // scroll target) exact. The pitch is applied INLINE on every row (and the SVG
  // gutter is drawn at the same height), so there is one source of truth:
  //  - desktop 32px: two text lines (subject + sha/author/date) need ~29px; the
  //    old 28px pitch jammed them together.
  //  - ≤1024px 46px touch rows: these used to be CSS-only while the windowing
  //    math and the gutter stayed at 28 — lanes broke into dashes between rows
  //    and the spacers / jump-to-ref target drifted by 18px per row.
  const rowH = $derived(isMobile ? 46 : 32);
  let viewTop = $state(0);
  let viewH = $state(0);

  /** Mirror the scroller's geometry into state so the window recomputes. */
  function sync(): void {
    const el = panelEl;
    if (!el) return;
    viewTop = el.scrollTop;
    viewH = el.clientHeight;
  }

  // Keep `viewH` correct across panel resizes (detail pane opening, window
  // resize, mobile accordion) — a stale height renders too few or too many rows.
  $effect(() => {
    const el = panelEl;
    if (!el) return;
    sync();
    const ro = new ResizeObserver(() => {
      sync();
      h.flushResync();
    });
    ro.observe(el);
    return () => ro.disconnect();
  });

  /** Pull the next page in as the user approaches the bottom, so history just
   *  keeps going instead of stopping at an arbitrary cutoff. */
  let scrollFrame = 0;
  function onScroll(): void {
    // One state write per frame: scroll events fire several times a frame on
    // trackpads, and each write re-derived the row window.
    if (scrollFrame) return;
    scrollFrame = requestAnimationFrame(() => {
      scrollFrame = 0;
      onScrollFrame();
    });
  }
  $effect(() => () => {
    if (scrollFrame) cancelAnimationFrame(scrollFrame);
    scrollFrame = 0;
  });
  function onScrollFrame(): void {
    sync();
    const el = panelEl;
    if (!el || h.loadingMore || !h.hasMore) return;
    // Two viewports of runway — enough that the next page is usually there
    // before the user reaches the end.
    if (el.scrollTop + el.clientHeight * 3 >= el.scrollHeight) h.loadMore();
  }

  /** A graph nobody can see (window hidden, another module/sub-tab covering
   *  it, the phone accordion folded) parks its auto-fetch re-syncs. */
  function visible(): boolean {
    if (typeof document !== 'undefined' && document.hidden) return false;
    const el = panelEl;
    return !!el && el.isConnected && el.getClientRects().length > 0;
  }

  // ── Lane layout ───────────────────────────────────────────────────────────
  // The graph: rows + the widest lane count reached (drives the gutter width so
  // it reflects the real fan-out, not just node columns). The pass itself is
  // `layoutLanes` (graph-lanes.ts); this derived owns its memo.
  let laneMemo: LaneMemo | null = null;
  const graph = $derived.by((): { rows: LaneRow[]; widest: number } => {
    const names = h.laneNames;
    const commits = h.commits;
    if (commits.length === 0) {
      laneMemo = null;
      return { rows: [], widest: 1 };
    }
    const out = layoutLanes(commits, names, laneMemo);
    laneMemo = out.memo;
    exposeLayoutProbe(out.laidOut);
    return { rows: out.rows, widest: out.widest };
  });

  /** E2E perf probe: how many rows the last layout pass computed. */
  function exposeLayoutProbe(rows: number): void {
    if (typeof window === 'undefined') return;
    (window as unknown as { __ottoGraphLayoutRows?: number }).__ottoGraphLayoutRows = rows;
  }

  const laneRows = $derived(graph.rows);

  /** The window of rows to render with the scroller at `top`: the shared
   *  arithmetic (lib/listWindow.svelte.ts) at the graph's fixed pitch, always
   *  windowed (min 0). viewH is 0 until the first measure — fall back to a
   *  screenful so the very first paint isn't a single row. */
  function windowAt(top: number): ListRange {
    return listRange(laneRows.length, rowH, top, viewH || 900, OVERSCAN, 0);
  }
  // Only these rows are rendered; `top`/`bottom` reserve the rest of the
  // height so the scrollbar still measures the FULL history.
  const rowWindow = $derived(windowAt(viewTop));
  const visibleRows = $derived(laneRows.slice(rowWindow.start, rowWindow.end));

  // Gutter width shared by every row so lane dots line up vertically. Sized to
  // the widest lane count actually reached (plus a half-lane for the node
  // radius), and capped (MAX_GUTTER_W) so a wide fan-out can't blow out the layout.
  const gutterWidth = $derived.by(() => {
    if (graph.rows.length === 0) return LANE_W;
    return Math.min(graph.widest * LANE_W + LANE_W / 2, MAX_GUTTER_W);
  });

  /** Sha the graph row for a revealed ref is pulsing on, so the row can flash
   *  once the scroll lands (a selection highlight alone is easy to miss when
   *  the jump moves the viewport hundreds of rows). */
  let pulseSha = $state<string | null>(null);
  let pulseTimer: ReturnType<typeof setTimeout> | null = null;

  /** Jump the list so the row for `sha` sits in the middle, and flash it.
   *
   *  With windowing the target row is usually NOT in the DOM yet, so this can't
   *  just call `scrollIntoView`. It computes the scroll offset from the row's
   *  INDEX (fixed pitch, measured off the top spacer so the sticky header and
   *  WIP row are accounted for), jumps there, then corrects against the real
   *  element once the window has rendered it. */
  function scrollToCommit(sha: string): void {
    const el = panelEl;
    if (!el) return;
    const idx = laneRows.findIndex((r) => r.commit.sha === sha);
    if (idx < 0) return;

    // Offset of the rows region within the scroll content.
    const rowsTop = topSpacerEl
      ? topSpacerEl.getBoundingClientRect().top - el.getBoundingClientRect().top + el.scrollTop
      : 0;
    // INSTANT, never 'smooth': a jump to an old ref can span tens of thousands
    // of pixels, and smooth-scrolling that far is a multi-second animation — it
    // reads as the app being slow when the data was already local.
    el.scrollTop = Math.max(0, rowsTop + idx * rowH - Math.max(0, (el.clientHeight - rowH) / 2));
    sync();

    // Correct on the next frame, once the window has rendered the target.
    requestAnimationFrame(() => {
      const row = el.querySelector<HTMLElement>(`[data-sha="${CSS.escape(sha)}"]`);
      if (row) row.scrollIntoView({ block: 'center', behavior: 'auto' });
      pulseSha = sha;
      if (pulseTimer) clearTimeout(pulseTimer);
      pulseTimer = setTimeout(() => (pulseSha = null), 1200);
    });
  }

  /** Focus the commit row for `sha` once the next render has it (the WIP row
   *  for null), else the scroller itself — the successor of a control that
   *  removed itself (the detail pane's close). */
  async function focusRow(sha: string | null): Promise<void> {
    await tick();
    const el = panelEl;
    if (!el) return;
    const target = sha
      ? el.querySelector<HTMLElement>(`[data-sha="${CSS.escape(sha)}"] .graph-select`)
      : el.querySelector<HTMLElement>('.wip-row');
    (target ?? el).focus({ preventScroll: true });
  }

  return {
    get isMobile() { return isMobile; },
    get panelEl() { return panelEl; },
    set panelEl(v: HTMLDivElement | null) { panelEl = v; },
    get topSpacerEl() { return topSpacerEl; },
    set topSpacerEl(v: HTMLDivElement | null) { topSpacerEl = v; },
    get rowH() { return rowH; },
    get laneRows() { return laneRows; },
    get visibleRows() { return visibleRows; },
    get padTop() { return rowWindow.top; },
    get padBottom() { return rowWindow.bottom; },
    get gutterWidth() { return gutterWidth; },
    get pulseSha() { return pulseSha; },
    onScroll,
    visible,
    windowAt,
    scrollToCommit,
    focusRow,
  };
}
