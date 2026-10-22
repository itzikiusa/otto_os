// Why a review's summary came from the deterministic fallback
// (error-handling-5-05). The daemon keeps the cause in the summarizer row's
// note — "deterministic fallback (summarizer: <cause>) — N final comments" —
// and the row's `provider` is the summarizer that actually ran, so the UI
// no longer blames "claude" for a codex / agy summarizer or for a run that
// worked but returned no comments.
import type { ReviewAgentState } from '../../lib/api/types';

const CAUSE = /^deterministic fallback \(summarizer: ([\s\S]*)\) — \d+ final comments?$/;

export interface SummaryFallback {
  /** The summarizer's provider ("claude" when the row names none). */
  provider: string;
  /** The recorded cause, or '' on rows written before it was stored. */
  cause: string;
}

/** The summarizer row's fallback details (it is the last agent row). */
export function summaryFallback(agents: readonly ReviewAgentState[] | undefined): SummaryFallback {
  const row = agents && agents.length > 0 ? agents[agents.length - 1] : undefined;
  return {
    provider: row?.provider?.trim() || 'claude',
    cause: row?.note?.match(CAUSE)?.[1]?.trim() ?? '',
  };
}

/** Tooltip for a fallback badge / note. */
export function summaryFallbackTitle(f: SummaryFallback, what = 'these comments'): string {
  const why = f.cause ? ` (${f.cause})` : '';
  return `The ${f.provider} summarizer’s output wasn’t used${why}; ${what} were deduped and ranked by the deterministic fallback.`;
}

/** The local review's note line. */
export function summaryFallbackSentence(f: SummaryFallback): string {
  const why = f.cause ? ` (${f.cause})` : '';
  return `The ${f.provider} summarizer’s output wasn’t used${why}, so findings were deduplicated and ranked mechanically.`;
}
