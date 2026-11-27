<script lang="ts">
  // The Side panel's Git tab: the working tree with the commit card on top
  // (CommitCard — the changed files, a diff on click, ONE primary: Commit and
  // push → Push → Create pull request → Open pull request), not the whole Git
  // page squeezed into 300 px; "Open in Git" goes to the full page. Auto-detects
  // the repo for the panel's folder — the focused session's cwd in an agent
  // session, else the workspace folder or the one picked in the panel
  // (registers it idempotently). Mounted only while the panel shows this tab,
  // so nothing is read while it is closed.
  import { git } from '../../lib/stores/git.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import { rightPanel } from '../../lib/stores/rightPanel.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  const CommitCard = lazyComponent(() => import('./CommitCard.svelte'));
  const repoRoute = (id: string): string => `git/${encodeURIComponent(id)}`;

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
  {:else if !git.primary && git.notARepo && (rightPanel.session || rightPanel.threadFolder)}
    <!-- The details card above (a terminal's or a chat pane's) already says "Not a git repository": one line, no button. -->
  {:else if !git.primary && git.reposError}
    <LoadState
      what="this workspace’s repositories"
      variant="compact"
      error={git.reposError}
      empty
      onretry={() => ws.currentId && void git.retryRepos(ws.currentId, rightPanel.folder)}
    />
  {:else if !git.primary}
    <EmptyState
      icon="branch"
      title={git.notARepo ? 'Not a git repository' : 'No repository yet'}
      body={git.notARepo
        ? `${rightPanel.session ? 'This session’s folder' : 'This folder'} isn’t a git repository.`
        : `No repository for ${rightPanel.session ? 'this session' : 'this workspace'} yet.`}
      actionLabel="Open Git module"
      actionKind="secondary"
      onaction={() => router.go('git')}
    />
  {:else}
    {@const primary = git.primary}
    {#if primary}
      <!-- Keyed like GitPage: switching the focused session to another repo
           must remount, or repo A's in-flight merge status, commit draft and
           selection land on repo B. -->
      <!-- In a session the details card above names the folder and has Open in › Git. -->
      {#if !rightPanel.session}
      <div class="gp-head">
        <span class="ellipsis" title={primary.path}>{primary.name}</span>
        <button type="button" class="btn small ghost" data-testid="git-panel-open" onclick={() => router.go(repoRoute(primary.id))}>Open in Git</button>
      </div>
      {/if}
      {#key primary.id}
        <LazyMount lazy={CommitCard} what="the changes" variant="panel" props={{ repo: primary, layout: 'sheet', onopenpr: (n: number) => router.go(`${repoRoute(primary.id)}/pr/${n}`), onresolve: () => router.go(repoRoute(primary.id)) }} />
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
  .gp-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    padding: var(--sp-2) var(--sp-4);
    font-size: var(--fs-s);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .gpanel > :global(:last-child) {
    flex: 1;
    min-height: 0;
  }
  .gp-pad {
    padding: 12px;
  }
</style>
