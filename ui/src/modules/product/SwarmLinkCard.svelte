<script lang="ts">
  // Cross-link card: shows the swarm project created from this product story
  // (via Plan → Swarm), including task counts, run count, and accumulated cost.
  // Calls GET /product/stories/{sid}/swarm on mount and whenever the story id
  // changes. Always renders 200 (null project = no link yet). Clicking "Open in
  // Swarm" navigates to the swarm Kanban for that project.
  import Icon from '../../lib/components/Icon.svelte';
  import { api } from '../../lib/api/client';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { router } from '../../lib/router.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type { StorySwarmLink } from './types';

  interface Props {
    storyId: string;
  }
  let { storyId }: Props = $props();

  // ── Fetch ─────────────────────────────────────────────────────────────────
  let link = $state<StorySwarmLink | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);

  $effect(() => {
    if (!storyId) return;
    load(storyId);
  });

  async function load(sid: string): Promise<void> {
    loading = true;
    error = null;
    try {
      link = await api.get<StorySwarmLink>(`/product/stories/${sid}/swarm`);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  // ── Navigation ─────────────────────────────────────────────────────────────
  async function openInSwarm(): Promise<void> {
    if (!link?.project || !ws.currentId) return;
    await swarm.openProject(ws.currentId, link.project.swarm_id, link.project.id);
    router.go('swarm');
  }

  // ── Derived display values ─────────────────────────────────────────────────
  const doneTasks = $derived(
    link?.tasks.filter((t) => t.status === 'done' || t.status === 'cancelled').length ?? 0,
  );
  const totalTasks = $derived(link?.tasks.length ?? 0);
  const runCount = $derived(link?.runs.length ?? 0);
  const costFmt = $derived(
    link && link.cost_usd > 0
      ? `$${link.cost_usd < 0.01 ? link.cost_usd.toFixed(4) : link.cost_usd.toFixed(2)}`
      : null,
  );
  const hasData = $derived(link != null && link.project != null);
</script>

{#if loading}
  <div class="swarm-link-card dim">Loading swarm link…</div>
{:else if error}
  <!-- silently swallow errors — the card is supplementary, not load-blocking -->
{:else if hasData && link}
  <div class="swarm-link-card">
    <div class="slc-head">
      <Icon name="split" size={13} />
      <span class="slc-title">Swarm project</span>
      <span class="slc-name">{link.project!.name}</span>
      <span class="grow"></span>
      {#if costFmt}
        <span class="chip dim">{costFmt}</span>
      {/if}
      <button class="btn small ghost slc-open" onclick={openInSwarm} title="Open this project in the Agent Swarm board">
        Open in Swarm
      </button>
    </div>

    <div class="slc-stats">
      <span class="stat">
        <Icon name="check" size={11} />
        {doneTasks}/{totalTasks} tasks
      </span>
      {#if runCount > 0}
        <span class="stat">
          <Icon name="clock" size={11} />
          {runCount} run{runCount === 1 ? '' : 's'}
        </span>
      {/if}
      {#if link.prs.length > 0}
        <span class="stat">
          <Icon name="git-pull-request" size={11} />
          {link.prs.length} PR{link.prs.length === 1 ? '' : 's'}
        </span>
      {/if}
      {#if link.artifacts.length > 0}
        <span class="stat">
          <Icon name="note" size={11} />
          {link.artifacts.length} artifact{link.artifacts.length === 1 ? '' : 's'}
        </span>
      {/if}
    </div>

    {#if link.tasks.length > 0}
      <div class="task-pills">
        {#each link.tasks.slice(0, 6) as t (t.id)}
          <span class="task-pill {t.status === 'done' || t.status === 'cancelled' ? 'done' : t.status === 'in_progress' ? 'active' : ''}"
            title={t.title}
          >{t.title}</span>
        {/each}
        {#if link.tasks.length > 6}
          <span class="task-pill dim">+{link.tasks.length - 6} more</span>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .swarm-link-card {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 12px;
    font-size: 12.5px;
  }
  .swarm-link-card.dim {
    color: var(--text-dim);
    font-size: 12px;
    padding: 8px 12px;
  }
  .slc-head {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .slc-title {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .slc-name {
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 220px;
  }
  .slc-open {
    flex: none;
  }
  .slc-stats {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
  }
  .stat {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--text-dim);
    font-size: 11.5px;
  }
  .task-pills {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .task-pill {
    font-size: 11px;
    padding: 1px 8px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text-dim);
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .task-pill.done {
    background: color-mix(in srgb, var(--status-done, green) 15%, transparent);
    color: var(--status-done, green);
    text-decoration: line-through;
    opacity: 0.75;
  }
  .task-pill.active {
    background: color-mix(in srgb, var(--status-working) 18%, transparent);
    color: var(--status-working);
  }
  .grow {
    flex: 1;
  }
</style>
