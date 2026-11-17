// The Threads sidebar's list as one sequence: the order "next / previous
// thread" and "Next that needs you" walk (pinned first, then every project's
// inbox order, then the folded origin groups — folded rows included, so a
// thread behind a fold is still reached), and ↑/↓ / Home / End between the
// rows when the list has focus (Enter opens: the rows are buttons).
import type { Session } from '../../lib/api/types';
import { rovingIndex } from './threadAttention';

/** Unique live thread ids in sidebar order. */
export function sidebarOrder(
  pinned: readonly Session[],
  groups: readonly { items: readonly { session: Session }[] }[],
  side: readonly { items: readonly Session[] }[],
): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  const add = (s: Session): void => {
    if (s.archived || seen.has(s.id)) return;
    seen.add(s.id);
    out.push(s.id);
  };
  pinned.forEach(add);
  for (const g of groups) for (const r of g.items) add(r.session);
  for (const g of side) g.items.forEach(add);
  return out;
}

/** ↑/↓ / Home / End over the rows' main buttons inside `list`. */
export function onListKeydown(e: KeyboardEvent, list: HTMLElement): void {
  if (e.altKey || e.metaKey || e.ctrlKey || e.shiftKey) return;
  const from = (e.target as HTMLElement | null)?.closest?.('.tr-main');
  if (!from) return;
  const rows = [...list.querySelectorAll<HTMLElement>('.tr-main')];
  const next = rovingIndex(e.key, rows.indexOf(from as HTMLElement), rows.length);
  if (next === null) return;
  e.preventDefault();
  rows[next]?.focus();
}

/** `use:listKeys` on the list's scroller: the arrow keys between its rows. */
export function listKeys(list: HTMLElement): { destroy: () => void } {
  const on = (e: KeyboardEvent): void => onListKeydown(e, list);
  list.addEventListener('keydown', on);
  return { destroy: () => list.removeEventListener('keydown', on) };
}
