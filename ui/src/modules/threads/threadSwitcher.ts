// The ⌘K switcher's thread rows (Orca's ⌘J jump palette, in Otto's one ⌘K):
// what needs you first (Approval, Question, Failed), then what finished while
// you looked elsewhere (Done), then what is working, then the rest by recency
// — inside each, the order the sidebar shows them (threadNav.order), so the
// two lists never disagree. One status word per row. Pure, import-free
// (types only): unit/threadsSwitcher.test.ts.
import type { PaletteThreadRow, PaletteThreadTone } from '../../lib/paletteThreads.svelte';
import type { StateLike } from './threadAttention';

export interface SwitcherThread {
  id: string;
  title: string;
  project: string;
  /** lib/status.ts `sessionState` (threadNav `threadRowState`). */
  state: StateLike;
  /** Finished a turn you have not looked at. */
  unread: boolean;
  /** What a waiting runtime thread asks for, when its requests are loaded. */
  asks: 'approval' | 'question' | null;
  lastActive: string;
}

function bucket(t: SwitcherThread): number {
  if (t.state.key === 'needs-you' || t.state.key === 'failed') return 0;
  if (t.unread) return 1;
  if (t.state.key === 'working' || t.state.key === 'running') return 2;
  return 3;
}

/** "now", "5m", "3h", "2d" — the quiet word for a thread that needs nothing. */
export function shortAgo(iso: string, nowMs: number): string {
  const s = Math.max(0, Math.floor((nowMs - Date.parse(iso)) / 1000));
  if (!Number.isFinite(s) || s < 60) return 'now';
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86_400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86_400)}d`;
}

function word(t: SwitcherThread, nowMs: number): { word: string; tone: PaletteThreadTone } {
  if (t.state.key === 'needs-you') return { word: t.asks === 'question' ? 'Question' : t.asks === 'approval' ? 'Approval' : 'Needs you', tone: 'warning' };
  if (t.state.key === 'failed') return { word: 'Failed', tone: 'danger' };
  if (t.unread) return { word: 'Done', tone: 'success' };
  if (t.state.key === 'working' || t.state.key === 'running') return { word: 'Working', tone: null };
  return { word: shortAgo(t.lastActive, nowMs), tone: null };
}

/** The switcher's rows: `sidebarOrder` is the sidebar's order (threads it
 *  does not show follow, newest first). */
export function switcherRows(threads: readonly SwitcherThread[], sidebarOrder: readonly string[], nowMs: number): PaletteThreadRow[] {
  const at = new Map(sidebarOrder.map((id, i) => [id, i] as const));
  const sorted = [...threads].sort(
    (a, b) =>
      bucket(a) - bucket(b) ||
      (at.get(a.id) ?? Number.MAX_SAFE_INTEGER) - (at.get(b.id) ?? Number.MAX_SAFE_INTEGER) ||
      b.lastActive.localeCompare(a.lastActive),
  );
  return sorted.map((t) => ({ id: t.id, title: t.title, project: t.project, ...word(t, nowMs) }));
}
