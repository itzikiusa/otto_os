// The in-app evidence behind a Run with Otto approval gate (S20-02): the gate
// shows counts ("3 findings (1 blocking) · proof failed"), and each count must
// lead to what it summarises so a person never approves blind. Pure, so the
// route mapping is unit-tested without mounting the panel.
import type { OttoRun } from '../../lib/api/types';
import type { IconName } from '../../lib/components/Icon.svelte';

export interface RunEvidenceLink {
  key: 'proof' | 'findings' | 'loop' | 'branch';
  label: string;
  title: string;
  icon: IconName;
  /** Hash route handed to `router.go`. */
  route: string;
}

const enc = encodeURIComponent;

export function runEvidenceLinks(
  run: Pick<OttoRun, 'proof_pack_id' | 'review_id' | 'goal_loop_id' | 'repo_id' | 'branch' | 'findings_total'>,
): RunEvidenceLink[] {
  const out: RunEvidenceLink[] = [];
  if (run.proof_pack_id) {
    out.push({
      key: 'proof',
      label: 'Open proof pack',
      title: 'Tests, diffs, CI and review evidence collected for this run',
      icon: 'shield',
      route: `proof/${enc(run.proof_pack_id)}`,
    });
  }
  // Review findings live on the repo's Review tab (one list per repo).
  if (run.repo_id && (run.review_id || run.findings_total > 0)) {
    out.push({
      key: 'findings',
      label: 'Open findings',
      title: 'The code-review findings for this run’s branch',
      icon: 'search',
      route: `git/${enc(run.repo_id)}/review`,
    });
  }
  if (run.goal_loop_id) {
    out.push({
      key: 'loop',
      label: 'Open goal loop',
      title: 'The goal loop that drove this run',
      icon: 'target',
      route: `loops/${enc(run.goal_loop_id)}`,
    });
  }
  if (run.repo_id && run.branch) {
    out.push({
      key: 'branch',
      label: 'View branch diff',
      title: `See ${run.branch} on the repository graph`,
      icon: 'branch',
      route: `git/${enc(run.repo_id)}/graph`,
    });
  }
  return out;
}
