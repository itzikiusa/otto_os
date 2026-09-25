<script lang="ts">
  // One file's history, opened from a diff header's ⋯ menu (via `gitBridge`).
  // `follow=true` walks THROUGH renames — a history that stops at the rename is
  // the reason the flag exists. Clicking a commit selects it in the graph.
  import type { CommitInfo } from '../../lib/api/types';
  import { api } from '../../lib/api/client';
  import { gitBridge } from './gitBridge.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import Skeleton from '../../lib/components/Skeleton.svelte';

  interface Props {
    repoId: string;
    path: string;
    onclose: () => void;
  }
  let { repoId, path, onclose }: Props = $props();

  let commits = $state<CommitInfo[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let retryRev = $state(0);

  $effect(() => {
    void retryRev;
    const id = repoId;
    const p = path;
    loading = true;
    error = null;
    api
      .get<CommitInfo[]>(
        `/repos/${id}/log?all=true&limit=200&follow=true&path=${encodeURIComponent(p)}`,
      )
      .then((rows) => {
        commits = rows;
      })
      .catch((e: unknown) => {
        commits = [];
        error = loadErrorText(e);
      })
      .finally(() => {
        loading = false;
      });
  });

  function when(date: string): string {
    const d = new Date(date);
    return Number.isNaN(d.getTime()) ? '' : d.toLocaleString();
  }
</script>

<section class="fh" aria-label="File history">
  <header class="fh-head">
    <Icon name="clock" size={13} />
    <span class="fh-title mono" title={path}>{path}</span>
    <span class="grow"></span>
    <button class="icon-btn" onclick={onclose} aria-label="Close file history" title="Close file history">
      <Icon name="x" size={14} />
    </button>
  </header>

  <div class="fh-body">
    {#if loading}
      <div class="fh-pad"><Skeleton rows={4} height={30} /></div>
    {:else if error}
      <div class="fh-pad"><LoadState what="this file’s history" {error} empty onretry={() => retryRev++} variant="compact" /></div>
    {:else if commits.length === 0}
      <p class="fh-msg">No commits touch this file.</p>
    {:else}
      <ul class="fh-list">
        {#each commits as c (c.sha)}
          <li>
            <button class="fh-row" onclick={() => gitBridge.focusCommit(repoId, c.sha)}>
              <span class="fh-subject" title={c.subject}>{c.subject}</span>
              <span class="fh-meta">
                <span class="mono">{c.short_sha}</span>
                · {c.author} · {when(c.date)}
              </span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</section>

<style>
  .fh {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--surface);
    border-inline-start: 1px solid var(--border);
  }
  .fh-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
  }
  .fh-title {
    font-size: var(--fs-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .grow {
    flex: 1;
  }
  .fh-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .fh-pad {
    padding: 10px;
  }
  .fh-msg {
    padding: 14px 12px;
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .fh-list {
    list-style: none;
    margin: 0;
    padding: 4px;
  }
  .fh-row {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    padding: 6px 8px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    text-align: start;
    cursor: pointer;
  }
  .fh-row:hover {
    background: var(--surface-2);
  }
  .fh-subject {
    font-size: var(--fs-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fh-meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
</style>
