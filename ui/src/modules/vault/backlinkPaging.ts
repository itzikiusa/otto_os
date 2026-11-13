// Backlinks arrive a page at a time (`GET …/backlinks?offset=&with_total=true`):
// a hub note in a 10k-note vault has thousands, and the panel grows on demand.
import type { VaultBacklink, VaultBacklinkPage } from '../../lib/api/types';

/** A reply from a daemon that predates paging is the bare row array: every
 *  backlink at once, whatever offset was asked for. */
export function toBacklinkPage(reply: VaultBacklinkPage | VaultBacklink[], offset: number): VaultBacklinkPage {
  if (Array.isArray(reply)) return { backlinks: reply, total: reply.length, offset, limit: reply.length };
  return reply;
}

const key = (row: VaultBacklink): string => `${row.path}\n${row.kind}`;

/** The shown rows plus the next page. A row already shown is not repeated:
 *  an edit elsewhere can shift the server's ordering between two pages. */
export function appendBacklinks(shown: VaultBacklink[], next: VaultBacklink[]): VaultBacklink[] {
  const seen = new Set(shown.map(key));
  return [...shown, ...next.filter((row) => !seen.has(key(row)))];
}

/** How many backlinks are not loaded yet (the "Show more" count). */
export function backlinksRemaining(shown: number, total: number): number {
  return Math.max(0, total - shown);
}
