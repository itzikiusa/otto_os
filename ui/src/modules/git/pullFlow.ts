// The one pull gesture, shared by the toolbar, the branch bar and the graph's
// `Pull <branch>` menu entries. Pulling is now a DELIBERATE action everywhere —
// a branch switch never pulls — so its reporting (conflicted merge, dirty-tree
// retry, ff-only refusal) must read identically from all three call sites.
import { api, isDirtyGitRefusal } from '../../lib/api/client';
import type { PullMode, PullReq, PullResp, RepoStatusResp } from '../../lib/api/types';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';

/** Apply a pull response: propagate the status, then say what happened.
 *  A pull whose merge conflicted comes back 200 with unmerged paths (the daemon
 *  leaves the merge in progress) — that is not a failure, it needs the
 *  resolver, and RepoView raises the banner off the same status. */
export function reportPull(r: PullResp, onstatus: (s: RepoStatusResp) => void): void {
  onstatus(r.status);
  const conflicts = r.status.changes.filter((c) => c.kind === 'conflicted').length;
  if (conflicts > 0) {
    toasts.warn(
      'Pulled with conflicts',
      r.note ??
        `${conflicts} file${conflicts === 1 ? '' : 's'} need resolution — open "Resolve conflicts"`,
    );
  } else {
    toasts.success('Pulled', r.note ?? undefined);
  }
}

/** `git pull` for `repoId`: POSTs `/pull`, offers "Stash, pull & restore" on a
 *  dirty-tree 409 and retries with `auto_stash:true`, and turns an `ff_only`
 *  refusal into an actionable toast instead of git's bare line. Never throws —
 *  every outcome is reported through a toast. */
export async function runPull(
  repoId: string,
  onstatus: (s: RepoStatusResp) => void,
  opts?: { mode?: PullMode; autoStash?: boolean },
): Promise<void> {
  const body: PullReq = {};
  if (opts?.mode) body.mode = opts.mode;
  if (opts?.autoStash) body.auto_stash = true;
  try {
    reportPull(await api.post<PullResp>(`/repos/${repoId}/pull`, body), onstatus);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    // `ff_only` on a diverged branch is a 409 the user can act on — say which
    // actions actually resolve it.
    if (/not possible to fast-forward/i.test(msg)) {
      toasts.error(
        'Pull failed',
        'Not possible to fast-forward — pull with merge or rebase, or push first.',
      );
      return;
    }
    // Dirty-tree refusal (409) → offer the stash → pull → restore retry instead
    // of dead-ending on git's message.
    if (isDirtyGitRefusal(e) && !opts?.autoStash) {
      const ok = await confirmer.ask(
        'Your uncommitted changes are in the way of the pull. Stash them, pull, then restore them?',
        { title: 'Stash, pull & restore', confirmLabel: 'Stash & pull' },
      );
      if (ok) await runPull(repoId, onstatus, { ...opts, autoStash: true });
      return;
    }
    toasts.error('Pull failed', msg);
  }
}
