<script lang="ts">
  // "Removed workspaces…": the archived workspaces (`?include_archived=true`)
  // with Restore (`PATCH {archived:false}`). Removing never touches the folder
  // on disk, and a restored workspace brings back the sessions left running.
  import { onMount } from 'svelte';
  import Modal from '../lib/components/Modal.svelte';
  import LoadState from '../lib/components/LoadState.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import { api } from '../lib/api/client';
  import type { Workspace, WorkspaceWithRole } from '../lib/api/types';
  import { loadErrorText } from '../lib/loadError';
  import { toasts } from '../lib/toast.svelte';
  import { toastError } from '../lib/toastError';
  import { ws } from '../lib/stores/workspace.svelte';

  interface Props {
    onclose: () => void;
  }
  let { onclose }: Props = $props();

  let rows = $state<WorkspaceWithRole[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let busy = $state<string | null>(null);

  async function load(): Promise<void> {
    loading = true;
    error = null;
    try {
      const all = await api.get<WorkspaceWithRole[]>('/workspaces?include_archived=true');
      rows = all.filter((w) => w.archived);
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      loading = false;
    }
  }
  onMount(() => void load());

  async function restore(w: WorkspaceWithRole): Promise<void> {
    busy = w.id;
    try {
      const back = await api.patch<Workspace>(`/workspaces/${w.id}`, { archived: false });
      rows = rows.filter((x) => x.id !== w.id);
      if (!ws.workspaces.some((x) => x.id === w.id)) ws.workspaces = [...ws.workspaces, { ...back, my_role: w.my_role }];
      void ws.refreshOtherSessions();
      toasts.success('Workspace restored', w.name);
    } catch (e) {
      toastError('Couldn’t restore the workspace', e);
    } finally {
      busy = null;
    }
  }
</script>

<Modal title="Removed workspaces" width={480} {onclose}>
  <LoadState what="removed workspaces" {loading} {error} empty={rows.length === 0} onretry={load} rows={3}>
    {#snippet emptyView()}
      <EmptyState icon="folder" variant="panel" title="No removed workspaces" body="A workspace you remove shows up here, ready to restore." />
    {/snippet}
    <ul class="rw-list" data-testid="removed-workspaces">
      {#each rows as w (w.id)}
        <li class="rw-row">
          <div class="rw-text">
            <span class="rw-name">{w.name}</span>
            <span class="rw-path" dir="ltr" title={w.root_path}>{w.root_path}</span>
          </div>
          {#if w.my_role === 'admin'}
            <button class="btn small" disabled={busy !== null} onclick={() => void restore(w)}>Restore</button>
          {:else}
            <span class="rw-note" title="Workspace admins can restore it">Admins only</span>
          {/if}
        </li>
      {/each}
    </ul>
  </LoadState>
</Modal>

<style>
  .rw-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: min(60vh, 420px);
    overflow-y: auto;
  }
  .rw-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-block: 6px;
    border-block-end: 1px solid var(--border);
  }
  .rw-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .rw-name {
    font-size: var(--fs-m);
    color: var(--text);
  }
  .rw-path,
  .rw-note {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
