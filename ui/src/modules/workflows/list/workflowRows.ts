// A Workflows-tab row's words (ux.md §8): its last run's word (Needs you,
// Failed, Working) else when the last run ended, and the dim third line.
// Pure; unit-tested in ui/unit/workflowRows.test.ts.
import type { Workflow } from '../../../lib/api/types';
import { ago, fallbackState, inboxFold, runWord, stepDuration, type Word } from '../runs/runWords.ts';

export interface WorkflowRowWords {
  word: string | null;
  tone: Word['tone'];
  time: string;
  line3: string;
}

export function workflowRow(w: Pick<Workflow, 'graph' | 'description' | 'last_run' | 'running_count'>, now: number): WorkflowRowWords {
  const last = w.last_run ?? null;
  const running = w.running_count ?? 0;
  const state = last ? (last.overview_state ?? fallbackState(last.status)) : null;
  let word: Word | null = null;
  if (state && (last?.attention || inboxFold(state) === 'needs')) word = runWord({ state: inboxFold(state) === 'needs' ? state : 'needs_you', attention: [] });
  else if (running > 0) word = { word: running > 1 ? `${running} running` : 'Working', tone: '' };
  else if (state === 'failed') word = runWord({ state, attention: [] });
  const steps = w.graph?.nodes?.filter((n) => !n.kind.endsWith('_trigger')).length ?? 0;
  const about = w.description?.split('\n')[0].trim() || `${steps} ${steps === 1 ? 'step' : 'steps'}`;
  const tail = !last ? 'No runs yet' : last.duration_ms != null && !running ? `last run took ${stepDuration(last.duration_ms)}` : '';
  return {
    word: word?.word ?? null,
    tone: word?.tone ?? '',
    time: last?.ended_at ? ago(now - Date.parse(last.ended_at)) : '',
    line3: [about, tail].filter(Boolean).join(' · '),
  };
}

/** Goals (`origin: goal`) fold apart from authored workflows; both by name. */
export function splitGoals<W extends Pick<Workflow, 'name' | 'origin'>>(list: readonly W[]): { workflows: W[]; goals: W[] } {
  const byName = (a: W, b: W) => a.name.localeCompare(b.name);
  return { workflows: list.filter((w) => w.origin !== 'goal').sort(byName), goals: list.filter((w) => w.origin === 'goal').sort(byName) };
}
