// The toolbar's push gesture. A plain push stays confirm-free (it is the most
// routine git action), but a push the remote REJECTS — it has commits this
// branch lacks — no longer dead-ends on git's "! [rejected] … (fetch first)":
// the user picks Pull (the usual fix) or, after a deliberate rewrite (amend,
// rebase), "Force push with lease". Force is never sent without that explicit
// pick, and it is always the lease form, which the daemon refuses when the
// remote moved since it was last fetched and integrated.
import { api, ApiError } from '../../lib/api/client';
import type { PushReq, PushTarget, RepoStatusResp } from '../../lib/api/types';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { runPull } from './pullFlow';
import { pushRefusal } from './push-errors';

/** What the rejection dialog resolved to — exported for tests. */
export type RejectedChoice = 'pull' | 'force' | null;

/** Ask what to do about a rejected push. Copy names the branch and the
 *  upstream so a force push says exactly what it overwrites. */
export async function askRejectedPush(branch: string, upstream: string | null, allowForce = true, sourceSha?: string): Promise<RejectedChoice> {
  const target = upstream ?? `origin/${branch}`;
  const { value } = await confirmer.choose(
    `${target} has commits that ${branch} doesn’t. Pull them in first, then push again.\n\n` +
      (allowForce ? `If you rewrote ${branch} on purpose (amend, rebase, squash), force push with lease ` +
      `replaces ${target} with your local branch — commits only on the remote are dropped ` +
      `from it. The lease refuses if someone pushed since you last fetched.` +
      (sourceSha ? `\n\nSource commit: ${sourceSha.slice(0, 12)}. If the source or destination changed, refresh and confirm again.` : '') :
      'The force-push target could not be verified. Refresh the repository or review its push configuration before retrying a rewrite.'),
    {
      title: 'Push rejected',
      options: [
        { label: 'Pull', value: 'pull', kind: 'primary' },
        ...(allowForce ? [{ label: 'Force push with lease', value: 'force', kind: 'danger' as const }] : []),
      ],
    },
  );
  return value === 'pull' || value === 'force' ? value : null;
}

/** Push (or publish) the current branch. Never throws — every outcome is a
 *  toast; returns true when something was pushed. */
export async function runPush(
  repoId: string,
  status: Pick<RepoStatusResp, 'branch' | 'upstream'>,
  onstatus: (s: RepoStatusResp) => void,
  opts?: { forceWithLease?: boolean; target?: PushTarget },
): Promise<boolean> {
  const body: PushReq = {};
  if (opts?.forceWithLease) {
    body.force_with_lease = true;
    body.expected_target = opts.target;
  }
  // Capture before the first request. An unsupported multi-ref configuration
  // can still do a normal push, but cannot offer an unbound force retry.
  const target = opts?.target ?? await api.get<PushTarget>(`/repos/${repoId}/push-target`).catch(() => null);
  try {
    const s = await api.post<RepoStatusResp>(`/repos/${repoId}/push`, body);
    onstatus(s);
    toasts.success(
      opts?.forceWithLease ? 'Force pushed' : status.upstream ? 'Pushed' : 'Branch published',
      s.upstream ?? status.branch,
    );
    return true;
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    const refusal = pushRefusal(e instanceof ApiError ? e.status : undefined, msg);
    if (refusal === 'lease') {
      toasts.error(
        'Force push refused',
        'The remote branch changed since you last fetched and integrated it — nothing was overwritten. Fetch, review the new commits, then decide.',
      );
      return false;
    }
    if (refusal === 'rejected' && !opts?.forceWithLease) {
      const canForce = target !== null && target.branch === status.branch;
      const destination = target ? `${target.remote}/${target.destination_ref.replace(/^refs\/heads\//, '')}` : status.upstream;
      const choice = await askRejectedPush(status.branch, destination, canForce, target?.source_sha);
      if (choice === 'pull') {
        await runPull(repoId, onstatus);
        return false;
      }
      if (choice === 'force' && canForce) return runPush(repoId, status, onstatus, { forceWithLease: true, target });
      return false;
    }
    toasts.error('Couldn’t push', msg);
    return false;
  }
}
