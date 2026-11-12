// The workspace switcher at scale (a dozen workspaces): the recently used
// ones first, and live activity per workspace from rows the store already
// holds (no extra requests). Pure — unit/workspaceSwitch.test.ts.

import type { Id, Session } from '../lib/api/types';
import type { MenuItem } from '../lib/contextmenu.svelte';

/** App-wide (not per window): most recently selected workspace ids, newest first. */
export const LS_WS_RECENT = 'otto_ws_recent';
/** How many recents lead the switcher. */
export const RECENT_MAX = 4;

export function parseRecent(raw: string | null): Id[] {
  try {
    const v: unknown = JSON.parse(raw ?? '[]');
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string').slice(0, RECENT_MAX) : [];
  } catch {
    return [];
  }
}

/** `recent` with `id` moved to the front, capped. */
export function noteRecent(recent: readonly Id[], id: Id): Id[] {
  return [id, ...recent.filter((x) => x !== id)].slice(0, RECENT_MAX);
}

/** Recents (that still exist, newest first) and the rest in list order. A
 *  short list (≤ RECENT_MAX + 1) gets no recents block — it's scannable as is. */
export function switcherOrder<W extends { id: Id }>(list: readonly W[], recent: readonly Id[]): { recent: W[]; rest: W[] } {
  if (list.length <= RECENT_MAX + 1) return { recent: [], rest: [...list] };
  const byId = new Map(list.map((w) => [w.id, w] as const));
  const lead = recent.map((id) => byId.get(id)).filter((w): w is W => w !== undefined);
  const ids = new Set(lead.map((w) => w.id));
  return { recent: lead, rest: list.filter((w) => !ids.has(w.id)) };
}

export interface WsActivity {
  working: number;
  needs: number;
}

/** Working / needs-you counts per workspace. */
export function workspaceActivity(
  sessions: Iterable<Pick<Session, 'id' | 'workspace_id'>>,
  status: Readonly<Record<Id, string>>,
  needsYou: Readonly<Record<Id, boolean>>,
): Map<Id, WsActivity> {
  const out = new Map<Id, WsActivity>();
  const seen = new Set<Id>();
  for (const s of sessions) {
    if (seen.has(s.id)) continue;
    seen.add(s.id);
    const working = status[s.id] === 'working';
    const needs = needsYou[s.id] === true;
    if (!working && !needs) continue;
    const a = out.get(s.workspace_id) ?? { working: 0, needs: 0 };
    if (working) a.working++;
    if (needs) a.needs++;
    out.set(s.workspace_id, a);
  }
  return out;
}

/** "2 working · 1 needs you", or undefined when idle. */
export function activityText(a: WsActivity | undefined): string | undefined {
  if (!a) return undefined;
  const parts = [a.working ? `${a.working} working` : '', a.needs === 1 ? '1 needs you' : a.needs ? `${a.needs} need you` : ''];
  return parts.filter(Boolean).join(' · ') || undefined;
}

/** The switcher menu: recents, a divider, then everything else. */
export function switcherItems<W extends { id: Id; name: string; root_path: string }>(
  list: readonly W[],
  recent: readonly Id[],
  activity: ReadonlyMap<Id, WsActivity>,
  currentId: Id | null,
  select: (id: Id) => void,
): MenuItem[] {
  const row = (w: W): MenuItem => ({
    label: w.name,
    checked: currentId === w.id,
    title: w.root_path,
    hint: activityText(activity.get(w.id)),
    action: () => select(w.id),
  });
  const { recent: lead, rest } = switcherOrder(list, recent);
  return lead.length === 0 ? rest.map(row) : [...lead.map(row), { separator: true }, ...rest.map(row)];
}
