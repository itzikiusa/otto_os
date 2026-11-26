// Pure sessionStorage handoff from Insights Action Plan items to the Goal form
// (Workflows) or Scheduled Tasks (E-22). Keeps insights/ decoupled from
// workflows/ and scheduled-tasks/ (zero cross-module imports).
import type { ActionItem } from './insightsParse';

export const AUTOMATION_DRAFT_KEY = 'otto:automation-draft';

export interface AutomationDraftPayload {
  target: 'goal' | 'scheduled_task';
  title: string;
  prompt: string;
  sourceLabel?: string;
}

export function buildAutomationDraft(
  target: 'goal' | 'scheduled_task',
  action: Pick<ActionItem, 'title' | 'metric' | 'current' | 'target' | 'effort' | 'ids'>,
  periodLabel?: string,
): AutomationDraftPayload {
  const details: string[] = [action.title.trim()];
  if (action.metric) {
    const move = action.current
      ? `${action.current}${action.target ? ` → ${action.target}` : ''}`
      : action.target || '';
    details.push(`Metric: ${action.metric}${move ? ` (${move})` : ''}.`);
  }
  if (action.ids.length > 0) {
    details.push(`Action IDs: ${action.ids.join(', ')}.`);
  }
  if (periodLabel) {
    details.push(`Source: Insights report (${periodLabel}).`);
  }
  return {
    target,
    title: action.title.trim().slice(0, 80),
    prompt: details.join('\n'),
    sourceLabel: periodLabel,
  };
}

export function stageAutomationDraft(payload: AutomationDraftPayload): void {
  try {
    sessionStorage.setItem(AUTOMATION_DRAFT_KEY, JSON.stringify(payload));
  } catch {
    /* private/sandboxed storage — route still navigates cleanly */
  }
}
