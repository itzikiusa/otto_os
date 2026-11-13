// The runtime timeline's "new messages" pill (RuntimeFollow.svelte), pure so
// it is unit-tested (unit/threadsRuntimeFollow.test.ts).

/** Messages that arrived after the one the reader last saw at the bottom. A
 *  missing anchor (the reader never saw one) counts nothing. */
export function unreadSince(ids: readonly string[], lastSeenId: string | null): number {
  if (!lastSeenId) return 0;
  const at = ids.lastIndexOf(lastSeenId);
  return at < 0 ? 0 : ids.length - 1 - at;
}

/** The pill's words while scrolled up, or null when it should not show. */
export function followPillText(unread: number, unseen: number, farUp: boolean): string | null {
  if (unread > 0) return `${unread} new ${unread === 1 ? 'message' : 'messages'}`;
  if (unseen > 0) return 'New activity';
  return farUp ? 'Jump to latest' : null;
}

/** How close to the end counts as "at the bottom", in px. */
export const END_BAND = 40;

/** Whether the timeline keeps following its end after a scroll that left it
 *  `gap` px above the end (`up`: it moved toward the top). T3 Code's rule
 *  (timelineViewport.ts): only the reader lets go — a scroll they drove
 *  (wheel, touch, keys, a press) upward off the very end, or out of the end band. A scroll the
 *  page caused (content shrinking clamps scrollTop, the browser's scroll
 *  anchoring, our own pin) never does; reaching the end band again re-arms
 *  following. */
export function followAfterScroll(following: boolean, gap: number, byReader: boolean, up: boolean): boolean {
  // gap 0 after moving up = scrollTop clamped by content shrinking.
  if (byReader && up && gap >= 1) return false;
  if (gap < END_BAND) return true;
  return byReader ? false : following;
}
