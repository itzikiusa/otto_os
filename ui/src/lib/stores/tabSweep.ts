// Safety net for session tabs that outlive their session. Normally the
// `session_removed` / `session_archive_changed` events (and every resync's
// `reconcileTabs`) close them; this re-checks the open tabs against the
// daemon when the window comes back (focus / visible, throttled) and after a
// bulk action, so a path that slipped past those closes on its own — and says
// which tab it was, to find the path next time. The PURE half (no runes) so
// `node --test` exercises it (unit/tabSweep.test.ts); App wiring lives in
// shell/installShell.svelte.ts.

import type { Id } from '../api/types';
import { idChunks } from './sessionBuckets.ts';

/** At most one window-return check per this long. */
export const TAB_SWEEP_THROTTLE_MS = 60_000;

/** Why a tab is closed: its row is gone, or it is archived (archiving closes
 *  the tab; an archived session has no live view). */
export type StaleReason = 'gone' | 'archived';

/** Ids worth asking about: real session tabs/panes (not the `__…` sentinels
 *  such as the docked DB Explorer), each once. */
export function sweepIds(tabs: readonly Id[], panes: readonly Id[]): Id[] {
  return [...new Set([...tabs, ...panes])].filter((id) => !id.startsWith('__'));
}

/**
 * The tabs a `GET /sessions?ids=` answer proves stale. A chunk whose request
 * FAILED (`unknown`) proves nothing — its ids are never closed. An id the
 * answer omits is gone (the endpoint returns every row that exists, archived
 * ones included); an id it returns archived is archived.
 */
export function staleTabs(
  asked: readonly Id[],
  rows: readonly { id: Id; archived?: boolean }[],
  unknown: ReadonlySet<Id>,
): { id: Id; reason: StaleReason }[] {
  const byId = new Map(rows.map((r) => [r.id, r] as const));
  const out: { id: Id; reason: StaleReason }[] = [];
  for (const id of asked) {
    if (unknown.has(id)) continue;
    const row = byId.get(id);
    if (!row) out.push({ id, reason: 'gone' });
    else if (row.archived) out.push({ id, reason: 'archived' });
  }
  return out;
}

/** Whether a window-return check is due (`force` = after a bulk action). */
export function sweepDue(now: number, last: number, force = false): boolean {
  return force || now - last >= TAB_SWEEP_THROTTLE_MS;
}

/** What the sweep needs from the workspace store (structural: no rune import). */
export interface TabSweepHost {
  readonly openTabs: readonly Id[];
  readonly panes: readonly Id[];
  readonly currentId: Id | null;
  getSession(id: Id): { title: string; status: string } | null;
  readonly statusMap: Readonly<Record<Id, string>>;
  closeTab(id: Id): void;
}

/**
 * Ask the daemon about every open tab (chunked like the store's own by-id
 * fetch) and close the ones proven stale — only if they are still open and
 * the workspace didn't change meanwhile. Resolves the closed ids.
 */
export async function sweepTabs(
  host: TabSweepHost,
  fetchRows: (ids: Id[]) => Promise<{ id: Id; archived?: boolean }[]>,
  warn: (msg: string, detail: Record<string, unknown>) => void = (m, d) => console.warn(m, d),
): Promise<Id[]> {
  const asked = sweepIds(host.openTabs, host.panes);
  if (asked.length === 0) return [];
  const workspace = host.currentId;
  // Snapshot what we knew BEFORE asking: the log should say what the tab showed.
  const known = new Map(asked.map((id) => [id, { title: host.getSession(id)?.title, status: host.statusMap[id] ?? host.getSession(id)?.status }] as const));
  const unknown = new Set<Id>();
  const pages = await Promise.all(
    idChunks(asked).map((chunk) =>
      fetchRows(chunk).catch(() => {
        for (const id of chunk) unknown.add(id);
        return [];
      }),
    ),
  );
  if (host.currentId !== workspace) return [];
  const closed: Id[] = [];
  for (const { id, reason } of staleTabs(asked, pages.flat(), unknown)) {
    if (!host.openTabs.includes(id) && !host.panes.includes(id)) continue;
    warn(`[otto] closing a tab whose session is ${reason === 'gone' ? 'gone' : 'archived'}`, {
      id,
      workspace,
      ...known.get(id),
      reason,
      // `inStore` false = the row had already left the store but the tab
      // stayed: a removal event closed nothing. True = the store still held
      // a row the daemon no longer has: the removal event never arrived.
      inStore: host.getSession(id) !== null,
    });
    host.closeTab(id);
    closed.push(id);
  }
  return closed;
}
