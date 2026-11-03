<script lang="ts">
  // Right-panel "Pull request" surface: the open PR from the thread's branch —
  // title, state, checks, branches — with Open in Git and Open on the forge.
  import type { Repo } from '../../lib/api/types';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { openExternal } from '../../lib/external';
  import { router } from '../../lib/router.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { threadPrs } from './threadPrs.svelte';
  import { forgeLabel } from './threadsHelpers';

  interface Props {
    repo: Repo | null;
    branch: string | null;
  }
  let { repo, branch }: Props = $props();

  const pr = $derived(threadPrs.prFor(repo, branch));
  const checked = $derived(threadPrs.checked(repo, branch));
  const forge = $derived(forgeLabel(repo));
</script>

<div class="tps" data-testid="threads-pr-surface">
  {#if !checked}
    <div class="tps-pad"><Skeleton rows={4} height={22} label="the pull request" /></div>
  {:else if !pr || !repo}
    <EmptyState
      variant="panel"
      icon="pr"
      title="No open pull request"
      body={branch ? `Nothing open from ${branch}${forge ? ` on ${forge}` : ''}.` : 'This thread has no branch.'}
    />
  {:else}
    <div class="tps-pad">
      <p class="tps-num mono">#{pr.number} · {pr.state}{pr.draft ? ' · draft' : ''}</p>
      <h3 class="tps-title" dir="auto">{pr.title}</h3>
      <dl class="tps-meta">
        <dt>Branches</dt>
        <dd class="mono" dir="ltr">{pr.source_branch} → {pr.target_branch}</dd>
        <dt>Author</dt>
        <dd>{pr.author}</dd>
        {#if pr.ci_status}
          <dt>Checks</dt>
          <dd>{pr.ci_status}</dd>
        {/if}
        <dt>Updated</dt>
        <dd>{rel(pr.updated_at)}</dd>
      </dl>
      <div class="tps-actions">
        <button type="button" class="btn small" onclick={() => router.go(`git/${encodeURIComponent(repo!.id)}/pr/${pr!.number}`)}>
          <Icon name="pr" size={12} /> Open in Git
        </button>
        <button type="button" class="btn small ghost" onclick={() => void openExternal(pr!.url)}>
          <Icon name="external" size={12} /> {forge ?? 'Forge'}
        </button>
        <button
          type="button"
          class="icon-btn"
          aria-label="Refresh pull request"
          title="Refresh"
          onclick={() => threadPrs.forget(repo, branch)}
        >
          <Icon name="refresh" size={12} />
        </button>
      </div>
    </div>
  {/if}
</div>

<style>
  .tps {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .tps-pad {
    padding: 12px;
  }
  .tps-num {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .tps-title {
    margin: 4px 0 10px;
    font-size: var(--fs-l);
    font-weight: 500;
  }
  .tps-meta {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 12px;
    margin: 0 0 12px;
    font-size: var(--fs-s);
  }
  .tps-meta dt {
    color: var(--text-dim);
  }
  .tps-meta dd {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .tps-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }
</style>
