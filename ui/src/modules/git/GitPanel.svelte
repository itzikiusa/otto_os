<script lang="ts">
  // Full Git view for the agent-mode right panel. Auto-detects the repo for the
  // focused session's cwd (registers it idempotently), then embeds the same
  // RepoView the Git module uses — branch + push/pull/fetch toolbar and the
  // Graph / Changes / History / Pull Requests tabs — with panel-local tab state
  // so it never navigates away from agent mode.
  import { git } from '../../lib/stores/git.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import RepoView from './RepoView.svelte';

  let panelTab = $state('changes');

  // When the focused session changes, detect the repo at its working dir.
  $effect(() => {
    const cwd = ws.activeSession?.cwd;
    const wsId = ws.currentId;
    if (cwd && wsId) void git.detectFor(wsId, cwd);
  });
</script>

<div class="gpanel">
  {#if git.detecting && !git.primary}
    <div class="gp-pad"><Skeleton rows={4} height={26} /></div>
  {:else if !git.primary}
    <div class="gp-empty">
      {#if git.notARepo}
        <p class="dim">This session's folder isn't a git repository.</p>
      {:else}
        <p class="dim">No repository for this session yet.</p>
      {/if}
      <button class="btn small" onclick={() => router.go('git')}>Open Git module</button>
    </div>
  {:else}
    {@const primary = git.primary}
    {#if primary}
      <RepoView repo={primary} tab={panelTab} embedded onTab={(t) => (panelTab = t)} />
    {/if}
  {/if}
</div>

<style>
  .gpanel {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .gp-pad {
    padding: 12px;
  }
  .gp-empty {
    padding: 16px 12px;
    text-align: center;
  }
  .gp-empty p {
    font-size: 12px;
    margin: 0 0 10px;
  }
</style>
