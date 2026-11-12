// "All connections" query history. The daemon serves history per connection
// (`GET …/db/history`); this view asks each database connection for its recent
// rows and merges them newest first, so a query run yesterday on another
// connection is one filter away instead of a connection switch per guess.
// Pure, so node tests import it directly.
import type { DbHistoryEntry } from '../../lib/api/types';

/** How many connections the merged view reads (most recently opened first). */
export const ALL_HISTORY_CONNECTIONS = 20;
/** Rows per connection in the merged view. */
export const ALL_HISTORY_PER_CONN = 50;

export interface ConnLike {
  id: string;
  name: string;
  last_opened_at?: string | null;
}

/** A merged row: the entry plus the connection it ran on. */
export interface AllHistoryRow extends DbHistoryEntry {
  connName: string;
}

/** The connections to read: most recently opened first (never-opened last,
 *  by name), capped. */
export function historyConnections<C extends ConnLike>(conns: readonly C[], max = ALL_HISTORY_CONNECTIONS): C[] {
  return [...conns]
    .sort((a, b) => (b.last_opened_at ?? '').localeCompare(a.last_opened_at ?? '') || a.name.localeCompare(b.name))
    .slice(0, max);
}

/** Every connection's rows as one list, newest first, narrowed to `connId`
 *  ('' = all) and to rows whose statement or error contains `search`. */
export function mergeHistory(
  lists: readonly { conn: ConnLike; rows: readonly DbHistoryEntry[] }[],
  connId: string,
  search: string,
): AllHistoryRow[] {
  const needle = search.trim().toLowerCase();
  const out: AllHistoryRow[] = [];
  for (const { conn, rows } of lists) {
    if (connId && conn.id !== connId) continue;
    for (const h of rows) {
      if (needle && !h.statement.toLowerCase().includes(needle) && !(h.error ?? '').toLowerCase().includes(needle)) continue;
      out.push({ ...h, connection_id: h.connection_id || conn.id, connName: conn.name });
    }
  }
  return out.sort((a, b) => b.created_at.localeCompare(a.created_at));
}
