<script lang="ts">
  import { onMount } from 'svelte';
  import LoopDetail from '../../src/modules/loops/LoopDetail.svelte';
  import RunDetail from '../../src/modules/run-with-otto/RunDetail.svelte';
  import ConfirmDialog from '../../src/lib/components/ConfirmDialog.svelte';
  import ContextMenu from '../../src/lib/components/ContextMenu.svelte';
  import type { OttoRun } from '../../src/lib/api/types';
  let target = $state('A');
  const kind = new URLSearchParams(location.search).get('kind');
  const run = $derived({ id: target, title: `Run ${target}`, status: 'completed', workspace_id: 'ws',
    source_kind: 'channel', source_ref: 'Fixture source', approval_decision: 'approved', proof_status: new URLSearchParams(location.search).get('proof') ?? 'passed',
    pr_draft_json: JSON.stringify({ title: `Run ${target}`, source_branch: 'review', target_branch: 'main' }),
    repo_id: 'repo', repo_path: '/fixture/repo', branch: 'review', base_branch: 'main',
    findings_total: 0, findings_blocking: 0, mode: 'single_agent', provider: 'codex',
  } as OttoRun);
  onMount(() => {
    const change = () => { target = 'B'; };
    window.addEventListener('review-target-change', change);
    return () => window.removeEventListener('review-target-change', change);
  });
</script>
{#if kind === 'run'}
  <RunDetail {run} onClose={() => {}} />
{:else}
  <LoopDetail id={target} onback={() => {}} />
{/if}
<ConfirmDialog /><ContextMenu />
