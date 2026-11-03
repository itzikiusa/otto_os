<script lang="ts">
  // The refs sidebar's dialogs, opened through `actions.dialogs`: the stash
  // browser and the bulk branch delete. Both are code-split (loaded on first
  // open, not with the Git page) and hand the fresh repo status back to the
  // graph's refresh.
  import type { RefsResp, RepoStatusResp } from '../../../lib/api/types';
  import LazyMount from '../../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
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

  const StashBrowser = lazyComponent(() => import('../StashBrowser.svelte'));
  const BranchCleanup = lazyComponent(() => import('../BranchCleanup.svelte'));
</script>

{#if actions.dialogs.stash !== null}
  <LazyMount
    lazy={StashBrowser}
    what="the stash browser"
    quiet
    onerror={() => (actions.dialogs.stash = null)}
    props={{
      repoId,
      initialSha: actions.dialogs.stash || null,
      onclose: () => (actions.dialogs.stash = null),
      onchanged: (s: RepoStatusResp) => void actions.refreshAfter(s),
    }}
  />
{/if}
{#if actions.dialogs.branches !== null && refs}
  <LazyMount
    lazy={BranchCleanup}
    what="the branch cleanup"
    quiet
    onerror={() => (actions.dialogs.branches = null)}
    props={{
      repoId,
      refs,
      currentBranch: status.branch,
      inWorktree: index.worktreeBranches,
      preselect: actions.dialogs.branches,
      onclose: () => (actions.dialogs.branches = null),
      ondone: (s: RepoStatusResp) => void actions.refreshAfter(s),
    }}
  />
{/if}
