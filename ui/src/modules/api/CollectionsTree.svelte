<script lang="ts">
  // Collections + nested folders (parent_id) + saved requests. Click a request
  // to load it into the builder; new/rename/delete collections; export a
  // collection to an OpenAPI 3 document.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { api } from '../../lib/api/client';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { detectAndParse } from '../../lib/api/importers';
  import type { ApiCollection, ApiRequest, Repo } from '../../lib/api/types';

  async function importFile(input: HTMLInputElement): Promise<void> {
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    try {
      const parsed = detectAndParse(await file.text(), file.name);
      await apiClient.importParsed(parsed);
    } catch (e) {
      toasts.error('Import failed', e instanceof Error ? e.message : String(e));
    }
  }

  // ── Git sync ────────────────────────────────────────────────────────────────
  let gitOpen = $state(false);
  let repos = $state<Repo[]>([]);
  let repoId = $state('');
  let commitMsg = $state('Update API collections');
  let branch = $state('');
  let syncing = $state(false);

  async function toggleGit(): Promise<void> {
    gitOpen = !gitOpen;
    if (gitOpen && repos.length === 0 && ws.currentId) {
      try {
        repos = await api.get<Repo[]>(`/workspaces/${ws.currentId}/repos`);
        if (repos.length && !repoId) repoId = repos[0].id;
      } catch (e) {
        toasts.error('Could not load repos', e instanceof Error ? e.message : String(e));
      }
    }
  }
  async function gitPull(): Promise<void> {
    if (!repoId) return;
    syncing = true;
    await apiClient.gitPullCollections(repoId);
    syncing = false;
  }
  async function gitPush(): Promise<void> {
    if (!repoId) return;
    syncing = true;
    const ok = await apiClient.gitPushCollections(repoId, commitMsg, branch.trim() || null);
    syncing = false;
    if (ok) gitOpen = false;
  }

  interface TreeNode {
    col: ApiCollection;
    items: ApiRequest[];
    children: TreeNode[];
  }

  let collapsed: Record<string, boolean> = $state({});
  const canEdit = $derived(ws.myRole !== 'viewer');

  function buildTree(parentId: string | null): TreeNode[] {
    return apiClient.collections
      .filter((c) => (c.parent_id ?? null) === parentId)
      .sort((a, b) => a.position - b.position || a.name.localeCompare(b.name))
      .map((col) => ({
        col,
        items: apiClient.requests
          .filter((r) => r.collection_id === col.id)
          .sort((a, b) => a.position - b.position || a.name.localeCompare(b.name)),
        children: buildTree(col.id),
      }));
  }
  const tree = $derived(buildTree(null));
  const ungrouped = $derived(
    apiClient.requests.filter((r) => !r.collection_id).sort((a, b) => a.name.localeCompare(b.name)),
  );

  function toggle(id: string): void {
    collapsed[id] = !collapsed[id];
  }

  async function newCollection(parentId: string | null): Promise<void> {
    if (!canEdit) return;
    const name = await confirmer.promptText(parentId ? 'Folder name' : 'Collection name', {
      title: parentId ? 'New folder' : 'New collection',
      confirmLabel: 'Create',
    });
    if (!name) return;
    await apiClient.saveCollection({ name, parent_id: parentId }, undefined);
  }

  async function renameCollection(col: ApiCollection): Promise<void> {
    if (!canEdit) return;
    const name = await confirmer.promptText('Rename collection', {
      title: 'Rename collection',
      confirmLabel: 'Rename',
      initial: col.name,
    });
    if (!name || name === col.name) return;
    await apiClient.saveCollection({ name, parent_id: col.parent_id }, col.id);
  }

  async function deleteCollection(col: ApiCollection): Promise<void> {
    if (!canEdit) return;
    if (!(await confirmer.ask(
      `Delete collection “${col.name}”? Folders inside are removed too; their requests become ungrouped.`,
      { title: 'Delete collection' },
    ))) return;
    await apiClient.deleteCollection(col.id);
  }

  async function deleteRequest(r: ApiRequest): Promise<void> {
    if (!canEdit) return;
    if (!(await confirmer.ask(`Delete request “${r.name}”?`, { title: 'Delete request' }))) return;
    await apiClient.deleteRequest(r.id);
  }

  function openRequest(r: ApiRequest): void {
    apiClient.loadRequestIntoDraft(r);
  }

  // Export the collection to OpenAPI: fetch the JSON and download it.
  async function exportOpenApi(col: ApiCollection): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    try {
      const spec = await api.get<unknown>(
        `/workspaces/${wsId}/api-client/collections/${col.id}/openapi`,
      );
      const blob = new Blob([JSON.stringify(spec, null, 2)], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `${col.name.replace(/[^\w.-]+/g, '_') || 'collection'}.openapi.json`;
      document.body.appendChild(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
      toasts.success('Exported OpenAPI', col.name);
    } catch (e) {
      toasts.error('Export failed', e instanceof Error ? e.message : String(e));
    }
  }
</script>

<div class="tree-wrap">
  <div class="tree-head">
    <span class="tree-title">Collections</span>
    <div class="row">
      <button class="icon-btn" title="New request" aria-label="New request" onclick={() => apiClient.newDraft()}>
        <Icon name="plus" size={13} />
      </button>
      {#if canEdit}
        <button class="icon-btn" title="New collection" aria-label="New collection" onclick={() => newCollection(null)}>
          <Icon name="folder" size={13} />
        </button>
        <label class="icon-btn" title="Import (Postman / OpenAPI / HAR)" aria-label="Import collection">
          <Icon name="external" size={13} />
          <input type="file" accept=".json,.har,.yaml,.yml" hidden onchange={(e) => importFile(e.currentTarget as HTMLInputElement)} />
        </label>
        <button class="icon-btn" class:active={gitOpen} title="Sync with Git" aria-label="Sync collections with Git" onclick={toggleGit}>
          <Icon name="branch" size={13} />
        </button>
      {/if}
    </div>
  </div>

  {#if gitOpen}
    <div class="git-sync">
      {#if repos.length === 0}
        <div class="git-empty">No git repos connected in this workspace. Add one in the Git tab.</div>
      {:else}
        <select class="input git-repo" value={repoId} onchange={(e) => (repoId = (e.currentTarget as HTMLSelectElement).value)} aria-label="Repository">
          {#each repos as r (r.id)}<option value={r.id}>{r.name}</option>{/each}
        </select>
        <div class="git-row">
          <input class="input git-msg" bind:value={commitMsg} placeholder="Commit message" />
          <input class="input git-branch" bind:value={branch} placeholder="branch (optional)" />
        </div>
        <div class="git-actions">
          <button class="btn small ghost" onclick={gitPull} disabled={syncing}>
            <Icon name="refresh" size={11} />Pull
          </button>
          <button class="btn small primary" onclick={gitPush} disabled={syncing}>
            <Icon name="branch" size={11} />Commit &amp; Push
          </button>
        </div>
        <div class="git-hint">Collections are stored as Postman files under <code>collections/</code>. Use a branch to open a PR afterwards from the Git tab.</div>
      {/if}
    </div>
  {/if}

  {#if apiClient.collections.length === 0 && apiClient.requests.length === 0}
    <EmptyState
      icon="folder"
      title="No saved requests"
      body="Save the current request from the builder, or create a collection to organize them."
      actionLabel={canEdit ? 'New collection' : undefined}
      onaction={canEdit ? () => newCollection(null) : undefined}
    />
  {:else}
    <div class="tree">
      {#each tree as node (node.col.id)}
        {@render collectionNode(node, 0)}
      {/each}

      {#if ungrouped.length > 0}
        <div class="col-head plain">
          <span class="caret-spacer"></span>
          <Icon name="box" size={12} />
          <span class="col-name grow">Ungrouped</span>
          <span class="count">{ungrouped.length}</span>
        </div>
        {#each ungrouped as r (r.id)}
          {@render requestRow(r, 1)}
        {/each}
      {/if}
    </div>
  {/if}
</div>

{#snippet collectionNode(node: TreeNode, depth: number)}
  {@const isOpen = !collapsed[node.col.id]}
  <div class="col-head" style="padding-left: {depth * 14 + 4}px">
    <button class="caret" onclick={() => toggle(node.col.id)} aria-label="Toggle collection">
      <Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} />
    </button>
    <Icon name="folder" size={13} />
    <span class="col-name grow ellipsis" title={node.col.name}>{node.col.name}</span>
    <span class="count">{node.items.length}</span>
    {#if canEdit}
      <button class="icon-btn" title="New folder" aria-label="New folder" onclick={() => newCollection(node.col.id)}><Icon name="plus" size={12} /></button>
    {/if}
    <button class="icon-btn" title="Export OpenAPI" aria-label="Export OpenAPI" onclick={() => exportOpenApi(node.col)}><Icon name="external" size={12} /></button>
    {#if canEdit}
      <button class="icon-btn" title="Rename" aria-label="Rename" onclick={() => renameCollection(node.col)}><Icon name="edit" size={12} /></button>
      <button class="icon-btn" title="Delete" aria-label="Delete" onclick={() => deleteCollection(node.col)}><Icon name="trash" size={12} /></button>
    {/if}
  </div>
  {#if isOpen}
    {#each node.items as r (r.id)}
      {@render requestRow(r, depth + 1)}
    {/each}
    {#each node.children as child (child.col.id)}
      {@render collectionNode(child, depth + 1)}
    {/each}
  {/if}
{/snippet}

{#snippet requestRow(r: ApiRequest, depth: number)}
  <div
    class="req-row"
    class:active={apiClient.draft.requestId === r.id}
    style="padding-left: {depth * 14 + 8}px"
  >
    <button class="req-open grow" onclick={() => openRequest(r)} title={r.url}>
      <span class="rm rm-{r.method.toLowerCase()}">{r.method}</span>
      <span class="rname ellipsis">{r.name}</span>
    </button>
    {#if canEdit}
      <button class="icon-btn row-del" title="Delete" aria-label="Delete request" onclick={() => deleteRequest(r)}><Icon name="trash" size={11} /></button>
    {/if}
  </div>
{/snippet}

<style>
  .git-sync {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
  }
  .git-empty,
  .git-hint {
    font-size: 11px;
    color: var(--text-dim);
  }
  .git-repo {
    width: 100%;
  }
  .git-row {
    display: flex;
    gap: 6px;
  }
  .git-msg {
    flex: 1;
    min-width: 0;
  }
  .git-branch {
    flex: 0 0 110px;
    min-width: 0;
  }
  .git-actions {
    display: flex;
    gap: 6px;
    justify-content: flex-end;
  }
  .icon-btn.active {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .tree-wrap {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .tree-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 2px 6px;
  }
  .tree-title {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .tree {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .col-head {
    display: flex;
    align-items: center;
    gap: 5px;
    height: 28px;
    padding-right: 4px;
    border-radius: var(--radius-s);
  }
  .col-head:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .col-head.plain {
    margin-top: 6px;
  }
  .col-name {
    font-size: 12px;
    font-weight: 600;
  }
  .col-head.plain .col-name {
    text-transform: uppercase;
    font-size: 11px;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .caret {
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    flex-shrink: 0;
  }
  .caret-spacer {
    width: 16px;
    flex-shrink: 0;
  }
  .count {
    font-size: 10px;
    color: var(--text-dim);
    min-width: 14px;
    text-align: center;
  }
  .req-row {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 26px;
    padding-right: 6px;
    border-radius: var(--radius-s);
  }
  .req-row:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .req-row.active {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .req-open {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: left;
    height: 100%;
  }
  .rm {
    font-size: 9.5px;
    font-weight: 700;
    font-family: var(--font-mono);
    color: var(--text-dim);
    width: 38px;
    flex-shrink: 0;
  }
  .rm-get { color: var(--status-working); }
  .rm-post { color: var(--accent); }
  .rm-put,
  .rm-patch { color: #d2691e; }
  .rm-delete { color: var(--status-exited); }
  .rname {
    font-size: 12px;
    min-width: 0;
  }
  .row-del {
    opacity: 0;
  }
  .req-row:hover .row-del {
    opacity: 1;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
