// What a Git page route shows. Focus (my PRs + my Jira work, across every
// repo) is a page-level view that lives in the URL, so the toggle, the Git
// crumb, a repo link inside Focus, Back/Forward and a reload all agree:
//   #/git/focus            → Focus
//   #/git/<repo>/focus     → legacy link (Focus used to be a repo sub-tab):
//                            open <repo>, then show Focus (#/git/focus)
//   #/git/<repo>[/<tab>]   → that repo (tab) — Focus off
//   #/git/<repo>/pr/<n>    → a PR detail
// Pure so unit/gitRoute.test.ts runs it under node --test.

export interface GitRoute {
  focus: boolean;
  /** A repo the route opens as a tab (null: none). */
  repo: string | null;
  /** Its sub-tab, when named (never 'focus'). */
  tab: string | null;
  /** A PR detail route. */
  pr: boolean;
  /** The legacy #/git/<repo>/focus: rewrite to #/git/focus after opening. */
  legacyFocus: boolean;
}

/** `parts`: the router's path segments (`['git', …]`). */
export function parseGitRoute(parts: readonly string[]): GitRoute {
  const [, a = null, b = null, c] = parts;
  if (a === 'focus') return { focus: true, repo: null, tab: null, pr: false, legacyFocus: false };
  const pr = b === 'pr' && c !== undefined;
  const legacyFocus = !pr && b === 'focus';
  return { focus: false, repo: a, tab: pr || legacyFocus ? null : b, pr, legacyFocus };
}

/** The route for showing (or leaving) Focus. */
export const focusPath = (on: boolean): string => (on ? 'git/focus' : 'git');
