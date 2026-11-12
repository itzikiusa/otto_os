// What a PR comment can do besides Reply / Resolve — edit, delete, react —
// handed down by PrDetail through Svelte context, so every CommentThread under
// it (the Conversation AND the diff's inline threads, which DiffViewer renders
// without knowing about any of this) gets the same controls. Outside a PR
// (no provider) the context is absent and the controls don't render.
import { getContext, setContext } from 'svelte';
import type { PrComment, ReactionKey } from '../../lib/api/types';

export interface PrCommentActions {
  /** Save new text (optimistic). Rejects after rolling back, so the editor
   *  can stay open with the draft. */
  edit(c: PrComment, body: string): Promise<void>;
  /** Ask, then delete for everyone. */
  remove(c: PrComment): Promise<void>;
  /** Toggle your `key` reaction (optimistic, rolled back on failure). */
  react(c: PrComment, key: ReactionKey): Promise<void>;
  /** The forge has a reactions API (false on Bitbucket Cloud). */
  reactionsSupported(): boolean;
}

const KEY = Symbol('pr-comment-actions');

export function providePrCommentActions(a: PrCommentActions): void {
  setContext(KEY, a);
}

export function prCommentActions(): PrCommentActions | undefined {
  return getContext<PrCommentActions | undefined>(KEY);
}
