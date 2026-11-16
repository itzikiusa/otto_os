// ⌘F over a whole runtime thread (T3 Code #10439), through the shared
// find-in-page bar (lib/findProviders.ts): next/prev, the match count and
// the highlight are FindInPage's. Only the newest `rowCap` rows are mounted,
// so past that the provider searches every loaded message (user and
// assistant text) and reveals a hit by widening the mounted window. Below
// it everything is mounted and the plain DOM walk is exact.
import { tick } from 'svelte';
import type { FindProvider } from '../../../lib/findProviders';
import type { TimelineRow } from './timeline';

/** The searchable text of a timeline row: a message's text, else nothing. */
export function rowFindText(row: TimelineRow | undefined): string {
  return row?.kind === 'message' ? row.item.text : '';
}

/** Rows to mount so row `i` of `total` is shown (a little context above). */
export function capToShow(i: number, total: number, cap: number): number {
  return i >= total - cap ? cap : total - i + 20;
}

export function threadFindProvider(o: {
  root: () => HTMLElement | null;
  rows: () => TimelineRow[];
  cap: () => number;
  setCap: (n: number) => void;
}): FindProvider {
  return {
    root: o.root,
    active: () => o.rows().length > o.cap(),
    count: () => o.rows().length,
    text: (i) => rowFindText(o.rows()[i]),
    reveal: async (i) => {
      o.setCap(capToShow(i, o.rows().length, o.cap()));
      await tick();
    },
    rowElement: (i) => {
      const row = o.rows()[i];
      if (row?.kind !== 'message') return null;
      return o.root()?.querySelector(`[data-rt-msg="${CSS.escape(row.item.id)}"]`) ?? null;
    },
  };
}
