// The agent sessions a PR offers (⋯ → Ask / Explain / Fix): the opening
// request a session gets, so it already knows which PR is meant. Pure, so the
// embedded PR agent (and "explain these files / lines") can build on it.
import type { OpenAgentSessionReq, PrDetail } from '../../lib/api/types';

export type PrAgentKind = 'ask' | 'explain' | 'fix';

const TITLE: Record<PrAgentKind, string> = { ask: 'Ask', explain: 'Explain', fix: 'Fix' };

export function prAgentRequest(
  kind: PrAgentKind,
  pr: PrDetail,
  ctx: { repoId: string; repoSlug: string; cwd: string | null; provider: OpenAgentSessionReq['provider'] },
): OpenAgentSessionReq {
  const ref = `pull request #${pr.number} “${pr.title}” in ${ctx.repoSlug} (${pr.source_branch} → ${pr.target_branch}${pr.url ? `, ${pr.url}` : ''})`;
  const read = 'Read it with the otto MCP tools (otto_get_pr, otto_get_pr_diff) or git.';
  const prompt =
    kind === 'ask'
      ? `We are talking about ${ref}. ${read} Load the context, reply with a one-paragraph summary, then wait for my question.`
      : kind === 'explain'
        ? `Explain ${ref}. ${read} Walk me through the diff: what it changes and why, how the pieces fit, and which files and lines deserve the closest reading (risky logic, missing tests). Be concrete, cite file:line.`
        : `Address the open review findings on ${ref}. ${read} Check out ${pr.source_branch}, fix each unresolved comment, run the relevant tests, and show me the diff. Do not push or post anything without asking me first.`;
  return {
    provider: ctx.provider,
    title: `${TITLE[kind]} PR #${pr.number}`,
    cwd: ctx.cwd,
    prompt,
    meta: {
      pr_context: {
        repo_id: ctx.repoId,
        number: pr.number,
        title: pr.title,
        url: pr.url,
        source_branch: pr.source_branch,
        target_branch: pr.target_branch,
      },
    },
  };
}
