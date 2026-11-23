// The thread details card's pure half (ThreadDetailsCard.svelte): the ONE git
// action it offers for the checkout's state — Commit → Push → Create PR →
// "PR #N" — and where the card may float. Import-free (types only),
// unit-tested in unit/threadsDetails.test.ts.

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

/** The card's inline size and the room it takes beside the conversation
 *  (card + a gap on each side): `--thread-card-reserve` while it floats. */
// 264 + 32 + the 808 column = 1104: floats at 1440 with the sidebar open (~1123).
export const CARD_INLINE = 264;
export const CARD_RESERVE = CARD_INLINE + 32;
/** The reading column the card must never cover, plus its side padding. */
export const READING_COLUMN = 760 + 48;

/** The card floats top-right only beside ONE conversation (or the new-thread
 *  stage), with the right panel closed and a full reading column left over;
 *  everywhere else it heads the right panel. */
export function cardFloats(o: { centerWidth: number; compact: boolean; rightPanel: boolean; panes: number; altView: boolean }): boolean {
  return !o.compact && !o.rightPanel && !o.altView && o.panes <= 1 && o.centerWidth >= READING_COLUMN + CARD_RESERVE;
}
