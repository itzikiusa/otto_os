// Withdraw your review on a PR (`DELETE …/approve` | `DELETE …/request-changes`):
// confirmed like every outward write, the header changes the moment the forge
// accepts, then a quiet re-read reconciles — the same pattern as Approve. A
// refusal (GitHub 409, GitLab 400) is shown in the forge's own words.
import { api } from '../../lib/api/client';
import type { PrDetail } from '../../lib/api/types';
import { confirmOutward } from '../../lib/confirmOutward';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';
import { invalidatePr } from './pr-cache';
import { withoutMyReview, type MyReview } from './pr-review-state';

export interface WithdrawDeps {
  repoId: string;
  number: number;
  /** "repo on GitHub · PR #n “title”" */
  where: string;
  author: string | null;
  get: () => PrDetail | null;
  patch: (pr: PrDetail) => void;
  reload: () => Promise<void>;
  isMe: (name: string) => boolean;
  /** Mark the header busy ('withdraw') / idle (''); false = the view went away. */
  setBusy: (b: string) => boolean;
}

export async function withdrawMyReview(kind: Exclude<MyReview, null>, d: WithdrawDeps): Promise<void> {
  const approval = kind === 'approved';
  const ok = await confirmOutward({
    verb: approval ? 'Withdraw approval' : 'Withdraw change request',
    title: approval ? `Withdraw your approval of PR #${d.number}?` : `Withdraw your change request on PR #${d.number}?`,
    where: d.where,
    what: approval ? 'Your approval is removed; the PR no longer counts it.' : 'Your “changes requested” review is withdrawn.',
    who: `${d.author ? `${d.author} (the author)` : 'The author'} and the PR’s reviewers can see the change.`,
  });
  if (!ok || !d.setBusy('withdraw')) return;
  try {
    invalidatePr(d.repoId, d.number);
    await api.del(`/repos/${d.repoId}/prs/${d.number}/${approval ? 'approve' : 'request-changes'}`);
    toasts.success(approval ? 'Approval withdrawn' : 'Change request withdrawn', `PR #${d.number}`);
    const pr = d.get();
    if (pr) d.patch(withoutMyReview(pr, kind, d.isMe));
    await d.reload();
  } catch (e) {
    toastError(approval ? 'Couldn’t withdraw the approval' : 'Couldn’t withdraw the change request', e);
  } finally {
    d.setBusy('');
  }
}
