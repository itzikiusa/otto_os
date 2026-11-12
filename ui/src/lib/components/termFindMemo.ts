// The terminal find bar's state (open or closed, the query) parked with a
// session's engine (Terminal.svelte, keepAlive hosts). A keepAlive Terminal
// parks the old session's xterm on a tab switch, and the host may unmount the
// Terminal itself (the Connections page leaving an SSH tab for a DB tab), so
// the memo lives at module level, keyed by session id, and is taken back when
// that session's engine is adopted. Bounded: the oldest memo goes first.

export interface FindMemo {
  open: boolean;
  query: string;
}

const MAX_MEMOS = 64;
const memos = new Map<string, FindMemo>();

/** Park `memo` for `sid` (a closed, empty bar just forgets it). */
export function parkFind(sid: string | null, memo: FindMemo): void {
  if (!sid) return;
  memos.delete(sid);
  if (!memo.open && !memo.query) return;
  memos.set(sid, { open: memo.open, query: memo.query });
  while (memos.size > MAX_MEMOS) memos.delete(memos.keys().next().value as string);
}

/** Take (and forget) the memo parked for `sid`, if any. */
export function takeFind(sid: string | null): FindMemo | null {
  if (!sid) return null;
  const memo = memos.get(sid) ?? null;
  memos.delete(sid);
  return memo;
}
