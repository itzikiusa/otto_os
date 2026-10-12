<script lang="ts">
  // Full Git view for the Side panel. Auto-detects the repo for the panel's
  // folder — the focused session's cwd in an agent session, else the
  // workspace folder or the one picked in the panel (registers it
  // idempotently), then embeds the same
  // RepoView the Git module uses — branch + push/pull/fetch toolbar and the
  // Graph / Changes / History / Pull Requests tabs — with panel-local tab state
  // so it never navigates away from the page it sits beside. No Focus tab
  // here: that view is cross-repo, it lives on the Git page.
  import { git } from '../../lib/stores/git.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import { rightPanel } from '../../lib/stores/rightPanel.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import RepoView from './RepoView.svelte';

  let panelTab = $state('changes');

  // When the panel's folder changes (focused session, picked folder), detect
  // the repo at it.
  $effect(() => {
    const cwd = rightPanel.folder;
    const wsId = ws.currentId;
    if (cwd && wsId) void git.detectFor(wsId, cwd);
  });
</script>

<div class="gpanel">
  {#if git.detecting && !git.primary}
    <div class="gp-pad"><Skeleton rows={4} height={26} /></div>
  {:else if !git.primary}
    <div class="gp-empty">
      {#if git.reposError}
        <LoadState
          what="this workspace’s repositories"
          variant="compact"
          error={git.reposError}
          empty
          onretry={() => ws.currentId && void git.retryRepos(ws.currentId, rightPanel.folder)}
        />
      {:else if git.notARepo}
        <p class="dim">{rightPanel.session ? 'This session’s folder' : 'This folder'} isn’t a git repository.</p>
      {:else}
        <p class="dim">No repository for {rightPanel.session ? 'this session' : 'this workspace'} yet.</p>
      {/if}
      <button class="btn small" onclick={() => router.go('git')}>Open Git module</button>
    </div>
  {:else}
    {@const primary = git.primary}
    {#if primary}
      <!-- Keyed like GitPage: switching the focused session to another repo
           must remount, or repo A's in-flight merge status, commit draft and
           selection land on repo B. -->
      {#key primary.id}
        <RepoView repo={primary} tab={panelTab} embedded onTab={(t) => (panelTab = t)} />
      {/key}
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
    font-size: var(--fs-s);
    margin: 0 0 10px;
  }
</style>
