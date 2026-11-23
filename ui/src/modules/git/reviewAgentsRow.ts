// Pure row helpers for ReviewAgents.svelte: stable unique keys (two custom
// reviewers can share a default name like "New agent") and which action button
// belongs on each row so non-summarizer rows never render "Retry summary".

export type AgentRowRetryAction = 'retry-agent' | 'retry-summary' | null;

/** Stable unique key for an agent row even when two agents share a name. */
export function agentRowKey(index: number, name: string): string {
  return `${index}:${name}`;
}

/** True when a non-summarizer reviewer row can be stopped mid-run. */
export function canStopReviewAgent(index: number, total: number, status: string): boolean {
  const lastIdx = total - 1;
  return index < lastIdx && (status === 'running' || status === 'waiting');
}

/** Which retry button (if any) should render on row `index` of `total`.
 *  Only the trailing summarizer (`index === total - 1`) may show Retry summary;
 *  reviewer rows (`index < total - 1`) only show Retry once settled. */
export function agentRowRetryAction(
  index: number,
  total: number,
  status: string,
  view: 'running' | 'done',
): AgentRowRetryAction {
  const lastIdx = total - 1;
  const settled = status === 'done' || status === 'error' || status === 'skipped';
  if (index < lastIdx && settled) return 'retry-agent';
  if (index === lastIdx && (view === 'done' || settled)) return 'retry-summary';
  return null;
}
