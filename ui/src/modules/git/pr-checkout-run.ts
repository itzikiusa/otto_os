// The PR header's "Check out": fetch, switch with local edits stashed around
// it, then tell the truth about the local tip (pr-checkout.ts decides). A
// branch left behind by an earlier checkout gets a warning with a Pull action
// (fast-forward only — it never merges on the user's behalf).
import { api } from '../../lib/api/client';
import { git } from '../../lib/stores/git.svelte';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { toastError } from '../../lib/toastError';
import type { RepoStatusResp } from '../../lib/api/types';
import { checkoutToast, checkoutVerdict, updateConfirm } from './pr-checkout';

/** Throws on a failed fetch / checkout (the caller toasts it). `quiet()` true
 *  = the view went away: the work still lands, only the toast is skipped. */
export async function checkoutPrBranch(repoId: string, branch: string, quiet: () => boolean): Promise<void> {
  await api.post(`/repos/${repoId}/fetch`);
  const v = checkoutVerdict(await git.checkout(repoId, { branch, auto_stash: true }));
  if (quiet()) return;
  const t = checkoutToast(branch, v);
  if (v.kind === 'diverged') {
    const update = async (): Promise<void> => {
      const c = updateConfirm(branch, v);
      if (!(await confirmer.ask(c.message, { title: c.title, confirmLabel: c.confirmLabel, danger: false }))) return;
      try {
        const r = await api.post<{ status: RepoStatusResp; backup: string; also_kept?: string }>(`/repos/${repoId}/branch/update-to-upstream`, { branch, ahead: v.ahead });
        git.setStatus(repoId, r.status);
        git.bumpRefs(repoId);
        toasts.success(`Updated ${branch} to the pull request`, `Your earlier commits are on ${r.backup}.${r.also_kept ? ` A commit made meanwhile is on ${r.also_kept}.` : ''}`);
      } catch (e) {
        toastError(`Couldn’t update ${branch}`, e);
      }
    };
    toasts.push(t.level, t.title, t.body, 15000, { action: { label: 'Update…', run: update } });
    return;
  }
  if (v.kind !== 'behind') {
    toasts.push(t.level, t.title, t.body);
    return;
  }
  const pull = async (): Promise<void> => {
    const r = await git.pull(repoId, true, 'ff_only');
    toasts.success(`Pulled ${branch}`, r.note ?? undefined);
  };
  toasts.push(t.level, t.title, t.body, 12000, { action: { label: 'Pull', run: pull } });
}
