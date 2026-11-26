// Takes a draft Insights staged for the Goal form (sessionStorage, E-22;
// insights/actionHandoff.ts writes it). Nothing starts: the form opens filled.
const KEY = 'otto:automation-draft';

export interface StagedGoalDraft {
  title: string;
  prompt: string;
}

export function takeStagedGoalDraft(): StagedGoalDraft | null {
  try {
    const raw = sessionStorage.getItem(KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as { target?: unknown; title?: unknown; prompt?: unknown };
    if (parsed.target !== 'goal' || typeof parsed.prompt !== 'string') return null;
    sessionStorage.removeItem(KEY);
    return { title: typeof parsed.title === 'string' ? parsed.title : '', prompt: parsed.prompt };
  } catch {
    return null;
  }
}
