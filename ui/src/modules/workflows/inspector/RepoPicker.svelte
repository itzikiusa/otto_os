<script lang="ts">
  // A node's `repo_id` param as a picker over the workspace's repositories (it
  // used to be a free-text "Repo ID" to copy from the Git tab). A value that
  // isn't in the list — another workspace's repo, or a stale id — stays
  // selectable so opening the node never silently rewrites it.
  import { untrack } from 'svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import type { WorkflowNode } from '../../../lib/api/types';
  import type { InspectorOptions } from './options.svelte';
  import { paramStr, type SetParam } from './util';

  let { id, emptyLabel, node, set, wsId, pickers }: {
    id: string; emptyLabel: string; node: WorkflowNode; set: SetParam; wsId: string; pickers: InspectorOptions;
  } = $props();
  // Fetched once per workspace, only when a node with a repo picker is open.
  $effect(() => {
    const w = wsId;
    untrack(() => pickers.repos.ensure(w));
  });
  const value = $derived(paramStr(node, 'repo_id'));
  const repos = $derived(pickers.repos.rows(wsId) ?? []);
  const loading = $derived(pickers.repos.loading(wsId));
  const error = $derived(pickers.repos.error(wsId));
</script>

<select {id} {value} disabled={loading && repos.length === 0} onchange={(e) => set('repo_id', e.currentTarget.value)}>
  <option value="">{loading && repos.length === 0 ? 'Loading repositories…' : emptyLabel}</option>
  {#each repos as r (r.id)}
    <option value={r.id} title={r.path}>{r.name}</option>
  {/each}
  {#if value && !repos.some((r) => r.id === value)}
    <option {value}>{value} (not in this workspace)</option>
  {/if}
</select>
{#if error}
  <LoadState what="repositories" variant="compact" {error} empty onretry={() => pickers.repos.retry(wsId)} />
{/if}
