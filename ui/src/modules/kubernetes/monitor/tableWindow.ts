// perf K8s R5: windowing for the per-cluster Monitor workloads TABLE. The
// table scrolls with the page body (not its own box) and one row can expand
// into a tall detail row, so the shared VirtualList (own scroll container,
// uniform rows) does not fit. Instead the table keeps native <table>
// semantics and renders only the rows near the viewport between two spacer
// rows — a bounded DOM (≈ viewport / rowH + 2·overscan rows, each with two
// SVG sparklines) however many workloads the cluster has.

import { listRange } from '../../../lib/listWindow.svelte';

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
 *  at `i * rowH`, plus `detailH` for every row after the expanded one — the
 *  shared `listRange` with its `extra` band (one window primitive app-wide). */
export function tableWindow(p: TableWindowInput): TableWindow {
  const { count, rowH } = p;
  if (count <= 0 || rowH <= 0) return { start: 0, end: 0, padTop: 0, padBottom: 0 };
  const exp = p.expanded ?? -1;
  const r = listRange(count, rowH, p.viewTop, Math.max(0, p.viewH), Math.max(0, p.overscan ?? 8), 0, {
    after: exp,
    h: p.detailH ?? 0,
  });
  return { start: r.start, end: r.end, padTop: r.top, padBottom: r.bottom };
}
