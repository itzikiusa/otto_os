// What the PR's agent thread is asked when a person picks ⋯ → Ask / Explain /
// Fix: the opening request, so it already knows which PR is meant. Pure; the
// embedded PR agent (PrAgentPanel, pr-agent-context.ts) builds "explain these
// files / lines" on top of it.
import type { PrDetail } from '../../lib/api/types';

export type PrAgentKind = 'ask' | 'explain' | 'fix';

/** "pull request #51 “title” in owner/repo (feature → main, url)". */
export function prRef(pr: PrDetail, repoSlug: string): string {
  return `pull request #${pr.number} “${pr.title}” in ${repoSlug} (${pr.source_branch} → ${pr.target_branch}${pr.url ? `, ${pr.url}` : ''})`;
}

/** The opening instruction for each ⋯ entry. `question` is the person's own
 *  words for Ask (asked straight away, so no extra round trip). */
export function prAgentPrompt(kind: PrAgentKind, pr: PrDetail, repoSlug: string, question = ''): string {
  const ref = prRef(pr, repoSlug);
  const read = 'Read it with the otto MCP tools (otto_get_pr, otto_get_pr_diff) or git.';
  if (kind === 'ask') {
    const q = question.trim();
    return q
      ? `We are talking about ${ref}. ${read} Load the context you need, then answer my question:\n\n${q}`
      : `We are talking about ${ref}. ${read} Load the context, reply with a one-paragraph summary, then wait for my question.`;
  }
  if (kind === 'explain') {
    return `Explain ${ref}. ${read} Walk me through the diff: what it changes and why, how the pieces fit, and which files and lines deserve the closest reading (risky logic, missing tests). Be concrete, cite file:line.`;
  }
  return `Address the open review findings on ${ref}. ${read} Check out ${pr.source_branch}, fix each unresolved comment, run the relevant tests, and show me the diff. Do not commit, push or post anything without asking me first.`;
}
