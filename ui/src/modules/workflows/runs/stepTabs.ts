// Which tabs a step's pane shows (ux.md §4.4), by what the step is. Pure;
// unit-tested in ui/unit/workflowStepTabs.test.ts.
import type { NodeRunState, PhaseMark } from '../../../lib/api/types';

export type PaneTab = 'conversation' | 'findings' | 'output' | 'files' | 'summary' | 'summarizer' | 'log' | 'preview' | 'result' | 'criteria' | 'input';

/** Kinds whose work is an agent's conversation. */
const AGENT_KINDS = new Set(['agent_prompt', 'prepare_context', 'product_analyze', 'product_rewrite', 'product_plan', 'product_publish', 'canvas', 'git_pr']);

export const TAB_LABEL: Record<PaneTab, string> = {
  conversation: 'Conversation', findings: 'Findings', output: 'Output', files: 'Files', summary: 'Summary', summarizer: 'Summarizer',
  log: 'Log', preview: 'Preview', result: 'Result', criteria: 'Criteria', input: 'Input',
};

/** The pane's tabs, the first one the default. `goal`: the Result of a goal run. */
export function paneTabs(what: { result?: boolean; goal?: boolean; child?: boolean; kind?: string; node?: NodeRunState }): PaneTab[] {
  if (what.result) return what.goal ? ['result', 'criteria', 'files', 'input'] : ['result', 'files', 'input'];
  if (what.child) return what.kind === 'review_run' ? ['conversation', 'findings', 'files'] : ['conversation', 'files'];
  const kind = what.kind ?? '';
  if (kind.endsWith('_trigger')) return ['input'];
  if (kind === 'review_run') return ['summary', 'summarizer', 'log', 'files'];
  // A check, and a goal's loop: the criteria with their evidence first.
  if (kind === 'check' || (kind === 'loop' && what.goal)) return ['criteria', 'log', 'output'];
  if (kind === 'human_approval') return ['preview', 'log'];
  // An agent step asking to go ahead keeps its conversation (the tool call it asks about).
  if (AGENT_KINDS.has(kind) || what.node?.thread) return ['conversation', 'output', 'files'];
  if (what.node?.attention?.kind === 'approval') return ['preview', 'log'];
  return ['log', 'output'];
}

/** An older (terminal-session) step's sessions: the ones still kept open in
 *  Agents; `gone` when every one was checked and none is kept — the pane then
 *  says so instead of offering an action that leads nowhere. */
export function keptSessions(ids: readonly string[], kept: Readonly<Record<string, boolean>>): { open: string[]; gone: boolean } {
  return { open: ids.filter((id) => kept[id]), gone: ids.length > 0 && ids.every((id) => kept[id] === false) };
}

export interface PhaseSegment {
  phase: string;
  label: string;
  ms: number;
  note?: string;
}

const PHASE_WORD: Record<string, string> = {
  queued: 'Queued', waiting: 'Waited', starting: 'Starting', working: 'Working', needs_you: 'With you', finishing: 'Finishing',
};

/** The phase bar's segments: each phase until the next one, the last until
 *  the step settled (or `now` while it runs). */
export function phaseSegments(phases: readonly PhaseMark[], end: number): PhaseSegment[] {
  return phases.map((p, i) => {
    const until = i + 1 < phases.length ? Date.parse(phases[i + 1].at) : end;
    const ms = Math.max(0, until - Date.parse(p.at));
    return { phase: p.phase, label: PHASE_WORD[p.phase] ?? p.phase, ms, ...(p.note ? { note: p.note } : {}) };
  });
}
