// Takes a draft staged for the Goal form (sessionStorage, E-22): Insights'
// "Hand off as a goal…" (insights/actionHandoff.ts), the Automate guide's Goal
// row (a blank draft) and an older loop's "Continue as a goal" (its id). The
// Workflows page opens the form filled; nothing starts until Start goal.
const KEY = 'otto:automation-draft';

export interface StagedGoalDraft {
  title: string;
  prompt: string;
  /** Continue this older goal loop as a goal. */
  loopId?: string;
}

export function takeStagedGoalDraft(): StagedGoalDraft | null {
  try {
    const raw = sessionStorage.getItem(KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as { target?: unknown; title?: unknown; prompt?: unknown; loop_id?: unknown };
    if (parsed.target !== 'goal') return null;
    sessionStorage.removeItem(KEY);
    const loopId = typeof parsed.loop_id === 'string' && parsed.loop_id ? parsed.loop_id : undefined;
    return {
      title: typeof parsed.title === 'string' ? parsed.title : '',
      prompt: typeof parsed.prompt === 'string' ? parsed.prompt : '',
      ...(loopId ? { loopId } : {}),
    };
  } catch {
    return null;
  }
}
