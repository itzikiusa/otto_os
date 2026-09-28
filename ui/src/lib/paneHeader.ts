// Pure helpers behind the session pane header (modules/agents/SessionView.svelte).
// Kept DOM-free so the node unit tests drive them (unit/paneHeader.test.ts).
//
// The header is ONE row whose content adapts to the PANE's inline size. CSS
// container queries on `.pane` (container name `pane`) hide/compact the inline
// chips and controls; `paneTier()` is the script-side twin of those same
// breakpoints, used only to decide which of the hidden controls come back as
// rows in the ⋯ menu. Keep `PANE_TIER_MIN` and the `@container pane (…)`
// rules in SessionView.svelte in step.

/** How a session pane shows an agent: the raw terminal, or the conversation
 *  rebuilt from the provider transcript (docs/design/conversation-view.md §5.1). */
export type SessionViewMode = 'terminal' | 'chat';

/**
 * Width tiers, widest → narrowest:
 * - `full`    ≥ 720 px — labelled Terminal/Chat toggle, provider chip with its
 *   name + idle countdown + cwd folder, task/now/handover chips;
 * - `compact` 420–719 — icon-only toggle, the provider chip shrinks to its icon;
 * - `minimal` 200–419 — dot + title + one view-flip icon + ⋯ (zoom, ✕ and the
 *   details move into ⋯);
 * - `micro`   < 200    — dot + title + ⋯ (the view switch moves into ⋯ too).
 */
export type PaneTier = 'full' | 'compact' | 'minimal' | 'micro';

/** Lower bound (inclusive, px) of each tier but the last. */
export const PANE_TIER_MIN = { full: 720, compact: 420, minimal: 200 } as const;

/** The tier for a pane of `width` px. An unmeasured pane (0 / NaN) is `full`,
 *  so the first paint never flashes the minimal header on a roomy pane. */
export function paneTier(width: number): PaneTier {
  if (!(width > 0)) return 'full';
  if (width >= PANE_TIER_MIN.full) return 'full';
  if (width >= PANE_TIER_MIN.compact) return 'compact';
  if (width >= PANE_TIER_MIN.minimal) return 'minimal';
  return 'micro';
}

/** True when `tier` is at least as narrow as `than` (`atMost('micro','minimal')`). */
export function tierAtMost(tier: PaneTier, than: PaneTier): boolean {
  const order: PaneTier[] = ['full', 'compact', 'minimal', 'micro'];
  return order.indexOf(tier) >= order.indexOf(than);
}

/**
 * Read a persisted per-session view choice. The retired `split` mode (chat
 * beside the terminal inside one pane) reads as `chat` — the side a Split user
 * was reading; the terminal is one click away. Anything else unknown → null
 * (= the default view).
 */
export function parseSessionView(raw: string | null | undefined): SessionViewMode | null {
  if (raw === 'terminal' || raw === 'chat') return raw;
  if (raw === 'split') return 'chat';
  return null;
}

/** The other view — what the toggle and ⌘⇧C switch to. */
export function otherSessionView(mode: SessionViewMode): SessionViewMode {
  return mode === 'chat' ? 'terminal' : 'chat';
}

/** The last path segment of a working directory, for the inline chip
 *  (`/Users/me/src/otto/` → `otto`); `/` and empty stay as they are. */
export function cwdLabel(cwd: string | null | undefined): string {
  const c = (cwd ?? '').trim();
  if (c === '') return '';
  const trimmed = c.replace(/[/\\]+$/, '');
  if (trimmed === '') return c.slice(0, 1);
  const parts = trimmed.split(/[/\\]/);
  return parts[parts.length - 1] || trimmed;
}

/** Everything the header no longer shows inline, as `[label, value]` rows —
 *  the provider chip's tooltip, its details menu and the ⋯ info rows. */
export interface PaneDetailsInput {
  provider?: string | null;
  /** A themed session's full name ("Cristiano Ronaldo") when it differs from the title. */
  nameFull?: string | null;
  account?: string | null;
  /** The session state's words ("Suspended", "Needs you"…), when not plain running/idle. */
  state?: string | null;
  idle?: string | null;
  tasks?: { done: number; total: number } | null;
  now?: string | null;
  handoverFrom?: string | null;
  handoverPending?: boolean;
  cwd?: string | null;
}

export function paneDetails(d: PaneDetailsInput): [string, string][] {
  const rows: [string, string][] = [];
  const put = (k: string, v: string | null | undefined): void => {
    const s = (v ?? '').trim();
    if (s !== '') rows.push([k, s]);
  };
  put('Agent', d.provider);
  put('Name', d.nameFull);
  put('Account', d.account);
  put('State', d.state);
  put('Idle', d.idle);
  if (d.tasks && d.tasks.total > 0) put('Tasks', `${d.tasks.done}/${d.tasks.total} done`);
  put('Now', d.now);
  put('Handed over from', d.handoverFrom);
  if (d.handoverPending) put('Handover', 'Preparing the brief…');
  put('Folder', d.cwd);
  return rows;
}

/** The detail rows as one multi-line tooltip. */
export function paneDetailsTitle(rows: [string, string][]): string {
  return rows.map(([k, v]) => `${k}: ${v}`).join('\n');
}
