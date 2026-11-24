// The ONE next git step for a checkout — Commit → Push → Create PR → the
// open PR — shared by Git's commit card (modules/git/CommitCard.svelte),
// Threads' details card and its commit sheet. Import-free (types only),
// unit-tested in unit/gitNextStep.test.ts and unit/threadsDetails.test.ts.
import type { Repo } from './api/types';

export type GitActionKind = 'commit' | 'push' | 'publish' | 'create-pr' | 'pr' | 'none';

export interface GitActionState {
  /** Uncommitted changes in the checkout. */
  changes: number;
  branch: string | null;
  upstream: string | null;
  ahead: number;
  /** The repo has a remote to push to. */
  remote: boolean;
  /** A forge Otto can open a pull request on (GitHub, GitLab, Bitbucket). */
  forge: boolean;
  /** The branch's open pull request, when there is one. */
  prNumber: number | null;
}

export interface GitAction {
  kind: GitActionKind;
  label: string;
}

/** The next step for this checkout: work to commit first, then commits to
 *  push, then the pull request (opened, or to create). */
export function gitAction(s: GitActionState): GitAction {
  if (s.changes > 0) return { kind: 'commit', label: 'Commit' };
  if (s.branch && s.remote && !s.upstream) return { kind: 'publish', label: 'Publish branch' };
  if (s.branch && s.upstream && s.ahead > 0) return { kind: 'push', label: s.ahead === 1 ? 'Push 1 commit' : `Push ${s.ahead} commits` };
  if (s.prNumber != null) return { kind: 'pr', label: `PR #${s.prNumber}` };
  if (s.branch && s.forge) return { kind: 'create-pr', label: 'Create PR' };
  return { kind: 'none', label: 'Nothing to commit' };
}

/** The commit card's one primary, in words: with work to commit it commits
 *  AND pushes when there is somewhere to push (Commit only is the secondary),
 *  then "Push (↑N)" → "Create pull request" → "Open pull request". */
export function primaryStep(s: GitActionState): GitAction {
  const a = gitAction(s);
  switch (a.kind) {
    case 'commit':
      return { kind: 'commit', label: s.branch && s.remote ? 'Commit and push' : 'Commit' };
    case 'push':
      return { kind: 'push', label: `Push (↑${s.ahead})` };
    case 'create-pr':
      return { kind: 'create-pr', label: 'Create pull request' };
    case 'pr':
      return { kind: 'pr', label: 'Open pull request' };
    default:
      return a;
  }
}

/** The repo's state in words for the header ("3 files changed · ↑2 not
 *  pushed"); empty parts drop out, a clean synced branch says "Clean". */
export function stateWords(s: { changes: number; ahead: number; behind: number; upstream: string | null; branch: string | null }): string {
  const parts: string[] = [];
  if (s.changes > 0) parts.push(s.changes === 1 ? '1 file changed' : `${s.changes} files changed`);
  if (s.branch && !s.upstream) parts.push('not published');
  else if (s.ahead > 0) parts.push(`↑${s.ahead} not pushed`);
  if (s.behind > 0) parts.push(`↓${s.behind} to pull`);
  return parts.length ? parts.join(' · ') : 'Clean';
}

/** Display name of a repo's forge, null when Otto can't open PRs there. */
export function forgeLabel(repo: Repo | null | undefined): string | null {
  const kind = repo?.forge && repo.forge !== 'unrecognized' ? repo.forge : repo?.provider;
  if (kind === 'github') return 'GitHub';
  if (kind === 'bitbucket') return 'Bitbucket';
  if (kind === 'gitlab') return 'GitLab';
  return null;
}
