// Side surfaces (docs/design/guidelines/layout.md "Side surfaces"): the one
// key scheme, the shared sizes and the width maths every list pane and Detail
// pane uses. Pure, so the rules are unit-tested (ui/unit/sidePanes.test.ts).
//
//   otto.pane.<module>.<pane>.w        the pane's width (px)
//   otto.pane.<module>.<pane>.hidden   '1' while the user has hidden it
//
// A page that stored its width / hidden state under an older key passes it
// as `legacy`: read once while the new key is still empty, then written
// under the new key (the old one is left alone, so an older build that is
// rolled back still finds it).
import { LIST_PANE } from '../../paneResizer.ts';

export { LIST_PANE };
export interface PaneRange {
  readonly default: number;
  readonly min: number;
  readonly max: number;
}
/** Rich rows (pull requests): one wider preset, same rules. */
export const WIDE_LIST_PANE: PaneRange = { default: 420, min: 280, max: 720 };
/** Every right-hand Detail pane. */
export const DETAIL_PANE: PaneRange = { default: 380, min: 300, max: 560 };
/** What the content beside a pane keeps before the pane yields (tablet clamp)
 *  or a Detail pane becomes a sheet over it. */
export const MIN_CONTENT = 480;
/** The one phone drawer (the Navigator): from the left, this wide. Right-hand
 *  sheets (Detail pane, Side panel) are full width on a phone. */
export const DRAWER_LEFT_WIDTH = 'min(86vw, 320px)';

export type PaneField = 'w' | 'hidden';
export function paneKey(module: string, pane: string, field: PaneField): string {
  return `otto.pane.${module}.${pane}.${field}`;
}

export interface PaneStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}
/** localStorage, or null where it is unavailable (private window, SSR). */
export function paneStorage(): PaneStorage | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
}

export interface PaneLegacy {
  /** The old width key (a plain number string). */
  w?: string;
  /** The old hidden state, read once (null = none stored). */
  hidden?: () => boolean | null;
}

function read(s: PaneStorage | null, key: string): string | null {
  try {
    return s?.getItem(key) ?? null;
  } catch {
    return null;
  }
}
function put(s: PaneStorage | null, key: string, value: string): void {
  try {
    s?.setItem(key, value);
  } catch {
    /* storage unavailable: the pane just isn't remembered */
  }
}

/** The stored width, clamped into the range; the default when none (or a bad one) is stored. */
export function loadPaneW(s: PaneStorage | null, module: string, pane: string, range: PaneRange, legacy?: PaneLegacy, fallback = range.default): number {
  const key = paneKey(module, pane, 'w');
  let raw = read(s, key);
  if (raw === null && legacy?.w) {
    raw = read(s, legacy.w);
    if (raw !== null) put(s, key, raw);
  }
  const v = raw === null ? NaN : Number(raw);
  return Number.isFinite(v) && v > 0 ? Math.max(range.min, Math.min(range.max, Math.round(v))) : fallback;
}

export function savePaneW(s: PaneStorage | null, module: string, pane: string, width: number): void {
  put(s, paneKey(module, pane, 'w'), String(Math.round(width)));
}

export function loadPaneHidden(s: PaneStorage | null, module: string, pane: string, legacy?: PaneLegacy): boolean {
  const key = paneKey(module, pane, 'hidden');
  const raw = read(s, key);
  if (raw !== null) return raw === '1';
  const old = legacy?.hidden?.() ?? null;
  if (old === null) return false;
  put(s, key, old ? '1' : '0');
  return old;
}

export function savePaneHidden(s: PaneStorage | null, module: string, pane: string, hidden: boolean): void {
  put(s, paneKey(module, pane, 'hidden'), hidden ? '1' : '0');
}

/** The widest a list pane may be in a row `rowW` wide and still leave the
 *  content `minContent` — never below the range's min (a tablet then gives
 *  the content less). The divider's max is this value, so dragging past it is
 *  impossible and aria-valuemax tells the truth. Unknown width → the range max. */
export function fitPaneMax(rowW: number, range: PaneRange, minContent = MIN_CONTENT): number {
  if (!(rowW > 0)) return range.max;
  return Math.max(range.min, Math.min(range.max, Math.floor(rowW - minContent)));
}

/** A list nested in another list's content (Brokers › Consumer groups,
 *  Settings › Access groups) whose row cannot hold its list at the minimum
 *  AND the content's `minContent` (a tablet beside the outer list) pushes
 *  like a phone: the list is the content until an item opens, then the item
 *  with a Back. A top-level list never does (it clamps). Unknown width → no. */
export function nestedPush(rowW: number, range: PaneRange, nested: boolean, minContent = MIN_CONTENT): boolean {
  return nested && rowW > 0 && rowW < range.min + minContent;
}

/** A Detail pane in a row `rowW` wide: inline while the content keeps
 *  `minContent` beside the pane at its minimum, else a sheet over the content.
 *  `shown` is the inline width (the user's, or all the room when maximized,
 *  never more than the room). */
export function detailFit(rowW: number, width: number, range: PaneRange, minContent = MIN_CONTENT, maximized = false): { sheet: boolean; shown: number } {
  if (!(rowW > 0)) return { sheet: false, shown: width };
  const room = Math.floor(rowW - minContent);
  if (room < range.min) return { sheet: true, shown: width };
  return { sheet: false, shown: Math.max(range.min, Math.min(maximized ? room : width, room)) };
}
