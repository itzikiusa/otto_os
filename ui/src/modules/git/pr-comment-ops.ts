// The PR comment writes behind pr-comment-ctx: each one patches the PR on
// screen first (pr-comments.ts) and puts the comment back the way it was if
// the forge refuses. Delete is outward-facing — everyone on the PR loses the
// comment — so it asks first and re-reads the PR afterwards: what happens to a
// deleted thread head's replies is the forge's call, not ours to guess.
import { api } from '../../lib/api/client';
import type { PrComment, PrDetail, PrReaction } from '../../lib/api/types';
import { confirmOutward } from '../../lib/confirmOutward';
import { toastError } from '../../lib/toastError';
import { toasts } from '../../lib/toast.svelte';
import type { PrCommentActions } from './pr-comment-ctx';
import { findComment, mapComment, reactedByMe, toggleReaction } from './pr-comments';
import { invalidatePr } from './pr-cache';

export interface CommentOpsDeps {
  repoId: () => string;
  number: () => number;
  /** The PR on screen (null while loading). */
  get: () => PrDetail | null;
  /** Show (and cache) a patched PR. */
  set: (pr: PrDetail) => void;
  /** Quiet re-read of the PR. */
  reload: () => Promise<void>;
  /** "repo · PR #n “title”" for the delete confirm. */
  where: () => string;
  /** False once the view is gone (or moved to another PR). */
  alive: (repoId: string, num: number) => boolean;
}

export function createCommentOps(d: CommentOpsDeps): PrCommentActions {
  const url = (c: PrComment): string => `/repos/${d.repoId()}/prs/${d.number()}/comments/${encodeURIComponent(c.id)}`;
  /** Patch comment `id` on the PR currently shown (no-op once it moved on). */
  const patch = (rid: string, num: number, id: string, fn: (c: PrComment) => PrComment | null): void => {
    const pr = d.get();
    if (pr && d.alive(rid, num)) d.set({ ...pr, comments: mapComment(pr.comments, id, fn) });
  };
  const reactionsOf = (id: string): PrReaction[] | undefined => findComment(d.get()?.comments ?? [], id)?.reactions;

  return {
    async edit(c, body) {
      const rid = d.repoId();
      const num = d.number();
      const before = c.body;
      patch(rid, num, c.id, (x) => ({ ...x, body }));
      invalidatePr(rid, num);
      try {
        const saved = await api.patch<PrComment>(url(c), { body });
        if (saved && typeof saved.body === 'string') patch(rid, num, c.id, (x) => ({ ...x, body: saved.body }));
      } catch (e) {
        patch(rid, num, c.id, (x) => ({ ...x, body: before }));
        throw e;
      }
    },

    async remove(c) {
      const rid = d.repoId();
      const num = d.number();
      const preview = c.body.length > 160 ? `${c.body.slice(0, 159)}…` : c.body;
      const ok = await confirmOutward({
        verb: 'Delete comment',
        title: 'Delete this comment?',
        where: d.where(),
        what: `Your comment “${preview}”`,
        who: (c.replies ?? []).length > 0
          ? 'It is removed for everyone on the pull request. The forge decides what happens to its replies.'
          : 'It is removed for everyone on the pull request.',
        danger: true,
      });
      if (!ok || !d.alive(rid, num)) return;
      const snapshot = d.get();
      patch(rid, num, c.id, () => null);
      invalidatePr(rid, num);
      try {
        await api.del(url(c));
        toasts.success('Comment deleted');
      } catch (e) {
        if (snapshot && d.alive(rid, num)) d.set(snapshot);
        toastError('Couldn’t delete the comment', e);
        return;
      }
      if (d.alive(rid, num)) await d.reload();
    },

    async react(c, key) {
      const rid = d.repoId();
      const num = d.number();
      const before = reactionsOf(c.id) ?? c.reactions;
      const adding = !reactedByMe(before, key);
      patch(rid, num, c.id, (x) => ({ ...x, reactions: toggleReaction(x.reactions, key) }));
      invalidatePr(rid, num);
      const path = `${url(c)}/reactions/${key}`;
      try {
        if (adding) {
          const next = await api.put<PrReaction[]>(path);
          if (Array.isArray(next)) patch(rid, num, c.id, (x) => ({ ...x, reactions: next }));
        } else {
          await api.del(path);
        }
      } catch (e) {
        patch(rid, num, c.id, (x) => ({ ...x, reactions: before }));
        toastError(adding ? 'Couldn’t add the reaction' : 'Couldn’t remove the reaction', e);
      }
    },

    reactionsSupported: () => d.get()?.reactions_supported === true,
  };
}
