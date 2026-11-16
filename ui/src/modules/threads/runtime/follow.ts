// The runtime timeline's "new messages" pill and scroll-stability rules
// (RuntimeFollow.svelte + RuntimeWorkFold.svelte), pure so they are unit-tested
// (unit/threadsRuntimeFollow.test.ts, unit/runtimeLiveSyncAndScroll.test.ts).

/** Messages that arrived after the one the reader last saw at the bottom. A
 *  missing anchor (the reader never saw one) counts nothing. */
export function unreadSince(ids: readonly string[], lastSeenId: string | null): number {
  if (!lastSeenId) return 0;
  const at = ids.lastIndexOf(lastSeenId);
  return at < 0 ? 0 : ids.length - 1 - at;
}

/** The pill's words while scrolled up, or null when it should not show. A
 *  request waiting on the person (`needs`: "Approval needed", attentionDock.ts)
 *  outranks a message count — it is why they should come back down. */
export function followPillText(unread: number, unseen: number, farUp: boolean, needs: string | null = null): string | null {
  if (needs) return needs;
  if (unread > 0) return `${unread} new ${unread === 1 ? 'message' : 'messages'}`;
  if (unseen > 0) return 'New activity';
  return farUp ? 'Jump to latest' : null;
}

/** How close to the end counts as "at the bottom", in px. */
export const END_BAND = 40;

/** True only when `scrollTop` moved upward WITHOUT content shrinking
 *  (`scrollHeight < lastScrollHeight` clamps `scrollTop` downward and must not
 *  be misclassified as the reader scrolling up). */
export function didScrollUpByUser(
  scrollTop: number,
  lastTop: number,
  scrollHeight: number,
  lastScrollHeight: number,
): boolean {
  return scrollTop < lastTop && scrollHeight >= lastScrollHeight - 1;
}

/** Scroll delta (`scroller.scrollTop += delta`) needed when a work-log block
 *  auto-folds (`shouldFold`: `false -> true`) while the reader is NOT pinned to
 *  the bottom, so visible text below the collapsing block moves 0 px. */
export function foldScrollCompensation(
  following: boolean,
  beforeTop: number,
  beforeBottom: number,
  afterBottom: number,
  viewportHeight: number,
): number {
  if (following) return 0;
  // Block is completely below the viewport: collapsing it moves nothing on screen.
  if (beforeTop >= viewportHeight) return 0;
  const delta = afterBottom - beforeBottom;
  return Math.abs(delta) >= 1 ? delta : 0;
}

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

/** A timeline row's identity: RuntimeTimelineRows' `{#each}` key. */
export function timelineRowKey(r: { kind: string; id?: string; item?: { id: string } }): string {
  return r.kind === 'message' && r.item ? r.item.id : (r.id ?? '');
}

/** The rows the timeline renders: the newest `cap`. While the reader is
 *  scrolled up, `holdFrom` (the first row they had rendered when they left
 *  the end, RuntimeFollow) keeps the window from starting after it, so a new
 *  row at the end never drops a row off the top and slides the text they are
 *  reading (`.rt-scroll` has `overflow-anchor: none`). */
export function renderWindow<R>(rows: R[], cap: number, holdFrom: string | null, key: (r: R) => string): R[] {
  let start = Math.max(0, rows.length - cap);
  if (start > 0 && holdFrom) {
    for (let i = 0; i < start; i++) {
      if (key(rows[i]) === holdFrom) {
        start = i;
        break;
      }
    }
  }
  return start > 0 ? rows.slice(start) : rows;
}
