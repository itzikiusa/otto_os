<script lang="ts">
  // Commit log + click → commit diff.
  import { api } from '../../lib/api/client';
  import type { CommitInfo, DiffResp } from '../../lib/api/types';
  import DiffViewer from './DiffViewer.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';

  interface Props {
    repoId: string;
  }
  let { repoId }: Props = $props();

  let commits: CommitInfo[] = $state([]);
  let loading = $state(true);
  let selected: string | null = $state(null);
  let diff: DiffResp | null = $state(null);
  let diffLoading = $state(false);
  let exhausted = $state(false);

  const PAGE = 50;

  $effect(() => {
    const id = repoId;
    loading = true;
    commits = [];
    selected = null;
    diff = null;
    exhausted = false;
    void api
      .get<CommitInfo[]>(`/repos/${id}/log?limit=${PAGE}&skip=0`)
      .then((c) => {
        commits = c;
        exhausted = c.length < PAGE;
      })
      .catch(() => (commits = []))
      .finally(() => (loading = false));
  });

  async function loadMore(): Promise<void> {
    const more = await api.get<CommitInfo[]>(
      `/repos/${repoId}/log?limit=${PAGE}&skip=${commits.length}`,
    );
    commits = [...commits, ...more];
    if (more.length < PAGE) exhausted = true;
  }

  async function select(sha: string): Promise<void> {
    selected = sha;
    diffLoading = true;
    try {
      diff = await api.get<DiffResp>(`/repos/${repoId}/diff?target=${encodeURIComponent(`commit:${sha}`)}`);
    } catch {
      diff = { files: [] };
    } finally {
      diffLoading = false;
    }
  }

  function fmtDate(iso: string): string {
    return new Date(iso).toLocaleDateString([], { month: 'short', day: 'numeric', year: 'numeric' });
  }
</script>

<div class="history">
  <div class="hist-side">
    {#if loading}
      <div style="padding: 10px"><Skeleton rows={8} height={34} /></div>
    {:else}
      {#each commits as c (c.sha)}
        <button class="commit" class:selected={selected === c.sha} onclick={() => select(c.sha)}>
          <div class="c-subject">{c.subject}</div>
          <div class="c-meta">
            <span class="mono c-sha">{c.short_sha}</span>
            <span class="dim">{c.author}</span>
            <span class="grow"></span>
            <span class="dim">{fmtDate(c.date)}</span>
          </div>
        </button>
      {:else}
        <div class="dim" style="padding: 16px; font-size: 12px">No commits.</div>
      {/each}
      {#if !exhausted && commits.length > 0}
        <button class="btn ghost" style="margin: 8px" onclick={loadMore}>Load more</button>
      {/if}
    {/if}
  </div>

  <div class="hist-diff">
    {#if selected === null}
      <EmptyState icon="commit" title="Select a commit" body="Pick a commit on the left to see its diff." />
    {:else if diffLoading}
      <Skeleton rows={6} height={28} />
    {:else if diff}
      <DiffViewer {diff} />
    {/if}
  </div>
</div>

<style>
  .history {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .hist-side {
    width: 320px;
    flex-shrink: 0;
    overflow-y: auto;
    border-right: 1px solid var(--border);
    padding: 6px;
    display: flex;
    flex-direction: column;
  }
  .commit {
    text-align: left;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    padding: 7px 9px;
    cursor: pointer;
    color: var(--text);
    transition: background 120ms ease-out;
  }
  .commit:hover {
    background: var(--surface-2);
  }
  .commit.selected {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .c-subject {
    font-size: 12.5px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .c-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 2px;
    font-size: 10.5px;
  }
  .c-sha {
    color: var(--accent);
    font-size: 10.5px;
  }
  .hist-diff {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 12px 14px;
  }
</style>
