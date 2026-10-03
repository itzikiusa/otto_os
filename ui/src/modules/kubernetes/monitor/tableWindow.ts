// perf K8s R5: windowing for the per-cluster Monitor workloads TABLE. The
// table scrolls with the page body (not its own box) and one row can expand
// into a tall detail row, so the shared VirtualList (own scroll container,
// uniform rows) does not fit. Instead the table keeps native <table>
// semantics and renders only the rows near the viewport between two spacer
// rows — a bounded DOM (≈ viewport / rowH + 2·overscan rows, each with two
// SVG sparklines) however many workloads the cluster has.

/** Below this many rows the whole table renders (no spacers, exact layout). */
export const WINDOW_MIN_ROWS = 80;

export interface TableWindowInput {
  /** Rows in the (filtered, sorted) table. */
  count: number;
  /** Estimated height of one collapsed row, px. */
  rowH: number;
  /** Visible band of the scroll host, in TBODY coordinates: the host's
   *  scrollTop minus the tbody's offset inside the host's content. May be
   *  negative while the table starts below the fold. */
  viewTop: number;
  viewH: number;
  /** Index of the expanded row (-1 = none) and its detail row's height. */
  expanded?: number;
  detailH?: number;
  overscan?: number;
}

export interface TableWindow {
  start: number;
  /** Exclusive. */
  end: number;
  padTop: number;
  padBottom: number;
}

/** The rows to render and the spacer heights around them. Rows are laid out
 *  at `i * rowH`, plus `detailH` for every row after the expanded one. */
export function tableWindow(p: TableWindowInput): TableWindow {
  const { count, rowH } = p;
  if (count <= 0 || rowH <= 0) return { start: 0, end: 0, padTop: 0, padBottom: 0 };
  const overscan = Math.max(0, p.overscan ?? 8);
  const exp = p.expanded ?? -1;
  const detailH = exp >= 0 && exp < count ? Math.max(0, p.detailH ?? 0) : 0;
  // y (tbody coords) → row index, skipping the detail row's band.
  const indexAt = (y: number): number => {
    const afterDetail = (exp + 1) * rowH + detailH;
    const i = detailH > 0 && y >= afterDetail ? exp + 1 + Math.floor((y - afterDetail) / rowH) : Math.floor(y / rowH);
    return Math.max(0, Math.min(count - 1, i));
  };
  const first = indexAt(Math.max(0, p.viewTop));
  const last = indexAt(Math.max(0, p.viewTop + Math.max(0, p.viewH)));
  const start = Math.max(0, first - overscan);
  const end = Math.min(count, last + overscan + 1);
  const top = (i: number): number => i * rowH + (detailH > 0 && i > exp ? detailH : 0);
  return {
    start,
    end,
    padTop: top(start),
    padBottom: Math.max(0, top(count) - top(end)),
  };
}
