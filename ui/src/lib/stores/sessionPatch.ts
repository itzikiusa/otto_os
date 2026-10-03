// Session-list patching — the PURE half (no runes) so `node --test` can
// exercise it; used by the workspace store's session events.

/** `list` with the item `id` replaced by `patch(item)` — or the SAME array when
 *  the id isn't in it or the patch returns the item unchanged. Session events
 *  used to `.map()` both session lists unconditionally; every new array re-ran
 *  ~15 deriveds (sorted channel / other-workspace groups…), the palette
 *  registry and the notification rows, even for a session in neither list. */
export function patchSessionIn<T extends { id: string }>(list: T[], id: string, patch: (s: T) => T): T[] {
  const i = list.findIndex((s) => s.id === id);
  if (i < 0) return list;
  const cur = list[i];
  const next = patch(cur);
  if (next === cur) return list;
  const out = list.slice();
  out[i] = next;
  return out;
}

/** A queued `session_status` write: the new status and the `last_active_at`
 *  the daemon stamped with it (perf R6 — status bursts are coalesced). */
export interface StatusPatch {
  status: string;
  at: string;
}

/** `list` with every row named in `pending` re-stamped (status +
 *  `last_active_at`) in ONE pass — or the SAME array when none is in it. A
 *  burst of N `session_status` events used to replace `sessions` N times, each
 *  rebuilding the id map and the sidebar buckets. */
export function applyStatusPatches<T extends { id: string; status: string; last_active_at: string }>(
  list: T[],
  pending: ReadonlyMap<string, StatusPatch>,
): T[] {
  if (pending.size === 0) return list;
  let out: T[] | null = null;
  for (let i = 0; i < list.length; i++) {
    const p = pending.get(list[i].id);
    if (!p) continue;
    out ??= list.slice();
    out[i] = { ...list[i], status: p.status, last_active_at: p.at };
  }
  return out ?? list;
}

/** Ids of `statusMap` entries to drop: not a loaded row (`known`) and not
 *  live (`working`/`running` — a panel watching a background session it
 *  fetched itself keeps its live dot). Without this the map kept an entry for
 *  every session id an event ever named (~100 review agents a day). */
export function staleStatusIds(statusMap: Record<string, string>, known: ReadonlySet<string>): string[] {
  const out: string[] = [];
  for (const [id, st] of Object.entries(statusMap)) {
    if (!known.has(id) && st !== 'working' && st !== 'running') out.push(id);
  }
  return out;
}

/** Whether a background (non-shown) row that just EXITED can leave the live
 *  list (perf R2): nothing on screen holds it (tab / pane / in-flight fetch).
 *  Its owning panel — a PR/commit draft, a run's agents — has the id from its
 *  own request by then, and a refresh drops exited background rows anyway
 *  (`pinnedIds` skips them). */
export function canDropExited(id: string, held: { tabs: readonly string[]; panes: readonly string[]; ensuring: ReadonlySet<string> }): boolean {
  return !held.tabs.includes(id) && !held.panes.includes(id) && !held.ensuring.has(id);
}
