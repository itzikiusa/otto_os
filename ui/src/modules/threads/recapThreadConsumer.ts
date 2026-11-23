// Consumer side of `otto.thread_draft_handoff` (written by Room Recap's
// "Start thread" action). One-shot with a 5-minute TTL so stale payloads never
// hijack a later visit to Threads.

export const THREAD_DRAFT_HANDOFF_KEY = 'otto.thread_draft_handoff';
const MAX_AGE_MS = 5 * 60_000;

export interface ConsumedThreadDraft {
  draft: string;
  workspaceId: string | null;
}

export function consumeThreadDraftHandoff(
  storage: Pick<Storage, 'getItem' | 'removeItem'> | null = typeof sessionStorage !== 'undefined' ? sessionStorage : null,
  now = Date.now(),
): ConsumedThreadDraft | null {
  if (!storage) return null;
  let raw: string | null = null;
  try {
    raw = storage.getItem(THREAD_DRAFT_HANDOFF_KEY);
    if (!raw) return null;
    storage.removeItem(THREAD_DRAFT_HANDOFF_KEY);
  } catch {
    return null;
  }
  try {
    const parsed = JSON.parse(raw) as { draft?: unknown; workspaceId?: unknown; ts?: unknown };
    if (typeof parsed.draft !== 'string' || !parsed.draft.trim()) return null;
    if (typeof parsed.ts === 'number' && now - parsed.ts > MAX_AGE_MS) return null;
    const workspaceId = typeof parsed.workspaceId === 'string' && parsed.workspaceId.trim() ? parsed.workspaceId : null;
    return { draft: parsed.draft, workspaceId };
  } catch {
    return null;
  }
}
