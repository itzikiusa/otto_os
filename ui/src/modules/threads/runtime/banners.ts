// The order and folding of RuntimeBannerStack (unit/runtimeWorkLog.test.ts).
/** One notice above the composer. `urgent` ones (an error, a question, an
 *  approval) always show; the rest fill up to one, then fold into one "N
 *  more" line — one quiet notice at a time above the composer. */
export interface BannerBlock {
  key: string;
  /** Its name in the folded line ("plan", "queued message"). */
  label: string;
  urgent?: boolean;
}

export const BANNER_ROOM = 1;

/** Which blocks show fully and which fold, given blocks in priority order. */
export function splitBanners(blocks: readonly BannerBlock[], room = BANNER_ROOM): { shown: BannerBlock[]; folded: BannerBlock[] } {
  const urgent = blocks.filter((b) => b.urgent).length;
  let left = Math.max(0, room - urgent);
  const shown: BannerBlock[] = [];
  const folded: BannerBlock[] = [];
  for (const b of blocks) {
    if (b.urgent) shown.push(b);
    else if (left > 0) {
      shown.push(b);
      left--;
    } else folded.push(b);
  }
  return { shown, folded };
}
