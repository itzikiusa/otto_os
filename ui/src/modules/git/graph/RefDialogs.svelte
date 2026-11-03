<script lang="ts">
  // The refs sidebar's dialogs, opened through `actions.dialogs`: the stash
  // browser and the bulk branch delete. Both hand the fresh repo status back
  // to the graph's refresh.
  import type { RefsResp, RepoStatusResp } from '../../../lib/api/types';
  import StashBrowser from '../StashBrowser.svelte';
  import BranchCleanup from '../BranchCleanup.svelte';
  import type { GraphActions } from './graphActions.svelte';
  import type { GraphRefIndex } from './graphRefIndex.svelte';

  interface Props {
    repoId: string;
    refs: RefsResp | null;
    status: RepoStatusResp;
    index: GraphRefIndex;
    actions: GraphActions;
  }
  let { repoId, refs, status, index, actions }: Props = $props();
</script>

{#if actions.dialogs.stash !== null}
  <StashBrowser
    {repoId}
    initialSha={actions.dialogs.stash || null}
    onclose={() => (actions.dialogs.stash = null)}
    onchanged={(s) => void actions.refreshAfter(s)}
  />
{/if}
{#if actions.dialogs.branches !== null && refs}
  <BranchCleanup
    {repoId}
    {refs}
    currentBranch={status.branch}
    inWorktree={index.worktreeBranches}
    preselect={actions.dialogs.branches}
    onclose={() => (actions.dialogs.branches = null)}
    ondone={(s) => void actions.refreshAfter(s)}
  />
{/if}
