// Consumes a staged Insights → Scheduled Task draft from sessionStorage (E-22).
const KEY = 'otto:automation-draft';

export interface StagedTaskDraft {
  title: string;
  prompt: string;
}

export function takeStagedTaskDraft(): StagedTaskDraft | null {
  try {
    const raw = sessionStorage.getItem(KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as { target?: unknown; title?: unknown; prompt?: unknown };
    if (parsed.target !== 'scheduled_task' || typeof parsed.prompt !== 'string') return null;
    sessionStorage.removeItem(KEY);
    return {
      title: typeof parsed.title === 'string' ? parsed.title : '',
      prompt: parsed.prompt,
    };
  } catch {
    return null;
  }
}
