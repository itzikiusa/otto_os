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
