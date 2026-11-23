// Handoff helper for starting a new Thread prefilled from a Room Recap action
// item. Uses one-shot sessionStorage so `rooms/` stays decoupled from `threads/`
// (zero cross-module imports) and works across full-page navigations.

export const THREAD_DRAFT_HANDOFF_KEY = 'otto.thread_draft_handoff';

export interface RecapActionDraftInput {
  sessionTitle: string;
  action: string;
  overview?: string | null;
}

export interface ThreadDraftHandoff {
  draft: string;
  workspaceId: string | null;
  source: 'room_recap';
  ts: number;
}

/** Build a clean, editable starting prompt for a Room Recap action item. */
export function buildRecapActionDraft(input: RecapActionDraftInput): string {
  const title = input.sessionTitle.trim() || 'Session room';
  const action = input.action.trim();
  const overview = (input.overview ?? '').trim().replace(/\s+/g, ' ');
  const clippedOverview =
    overview.length > 280 ? `${overview.slice(0, 277).trimEnd()}…` : overview;

  const lines = [`Follow up on this action item from room “${title}”:`, '', `> ${action}`];
  if (clippedOverview) {
    lines.push('', `Room recap context: ${clippedOverview}`);
  }
  lines.push('');
  return lines.join('\n');
}

/** Persist a one-shot draft handoff for ThreadsPage to consume on mount/route. */
export function queueRecapActionThread(
  input: RecapActionDraftInput & { workspaceId?: string | null },
  storage: Pick<Storage, 'setItem'> | null = typeof sessionStorage !== 'undefined' ? sessionStorage : null,
): string {
  const draft = buildRecapActionDraft(input);
  const payload: ThreadDraftHandoff = {
    draft,
    workspaceId: input.workspaceId ?? null,
    source: 'room_recap',
    ts: Date.now(),
  };
  try {
    storage?.setItem(THREAD_DRAFT_HANDOFF_KEY, JSON.stringify(payload));
  } catch {
    /* private mode / quota — navigation still opens Threads */
  }
  return draft;
}
