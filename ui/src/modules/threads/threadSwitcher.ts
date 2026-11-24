// The ⌘K switcher's thread order (Orca's ⌘J jump palette, in Otto's one ⌘K),
// from the sidebar's own inbox logic (threadInbox.ts): its class rank —
// needs you, Failed, Done (finished unread) — then what is working (the
// sidebar's Working section, here ahead of the rest so a capped list never
// drops it), then the rest; inside a rank, the order the sidebar shows them
// (threadNav.order). The words are the
// sidebar's (threadInbox `statusWord`). Pure: unit/threadsSwitcher.test.ts.
import type { PaletteThreadTone } from '../../lib/paletteThreads.svelte';
import { inboxScore, type InboxClass } from './threadInbox.ts';

const rank = (c: InboxClass): number => (c === 'working' ? inboxScore('done') + 0.5 : inboxScore(c));

/** `items` in switcher order: inbox rank first, then the sidebar's order
 *  (threads it does not show follow, newest first). */
export function switcherOrder<T extends { id: string; cls: InboxClass; lastActive: string }>(items: readonly T[], sidebarOrder: readonly string[]): T[] {
  const at = new Map(sidebarOrder.map((id, i) => [id, i] as const));
  return [...items].sort(
    (a, b) =>
      rank(a.cls) - rank(b.cls) ||
      (at.get(a.id) ?? Number.MAX_SAFE_INTEGER) - (at.get(b.id) ?? Number.MAX_SAFE_INTEGER) ||
      b.lastActive.localeCompare(a.lastActive),
  );
}

/** The tone of a status word (the sidebar row colours them the same way). */
export function wordTone(word: string): PaletteThreadTone {
  if (word === 'Approval' || word === 'Question' || word === 'Needs you' || word === 'Limited') return 'warning';
  if (word === 'Failed') return 'danger';
  if (word === 'Done') return 'success';
  return null;
}
