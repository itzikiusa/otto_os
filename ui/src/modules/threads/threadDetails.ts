// The thread details card's pure half (ThreadDetailsCard.svelte): where the
// card may float. The ONE git action it offers (Commit → Push → Create PR →
// "PR #N") is shared with Git's commit card: lib/gitNextStep.ts.
// Import-free (types only), unit-tested in unit/threadsDetails.test.ts.

export { gitAction, type GitAction, type GitActionKind, type GitActionState } from '../../lib/gitNextStep.ts';

/** The card's inline size and the room it takes beside the conversation
 *  (card + a gap on each side): `--thread-card-reserve` while it floats. */
// 264 + 32 + the 808 column = 1104: floats at 1440 with the sidebar open (~1123).
export const CARD_INLINE = 264;
export const CARD_RESERVE = CARD_INLINE + 32;
/** The reading column the card must never cover, plus its side padding. */
export const READING_COLUMN = 760 + 48;

/** The card floats top-right only beside ONE conversation (or the new-thread
 *  stage), with the right panel closed and a full reading column left over;
 *  everywhere else it heads the right panel. */
export function cardFloats(o: { centerWidth: number; compact: boolean; rightPanel: boolean; panes: number; altView: boolean }): boolean {
  return !o.compact && !o.rightPanel && !o.altView && o.panes <= 1 && o.centerWidth >= READING_COLUMN + CARD_RESERVE;
}
