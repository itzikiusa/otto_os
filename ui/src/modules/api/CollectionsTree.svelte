<script lang="ts">
  // Saved requests, grouped into collections and nested folders (parent_id).
  // Click a request to open it (an already-open or edited tab is never
  // overwritten — see apiClient.placeDraft). Row actions live in one ⋯ /
  // right-click menu per row instead of four always-visible icons.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import MethodTag from './MethodTag.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { api } from '../../lib/api/client';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import type { ApiCollection, ApiRequest } from '../../lib/api/types';

  interface Props {
    /** A request was opened / created (the page shows the editor). */
    onopen?: () => void;
  }
  let { onopen }: Props = $props();

  interface TreeNode {
    col: ApiCollection;
    items: ApiRequest[];
    children: TreeNode[];
  }

  let collapsed: Record<string, boolean> = $state({});
  const canEdit = $derived(ws.myRole !== 'viewer');

  // Search: every whitespace-separated token must match the request's
  // method/name/url (so "get users staging" narrows by all three). A collection
  // whose own name matches keeps all of its requests.
  let search = $state('');
  const tokens = $derived(search.trim().toLowerCase().split(/\s+/).filter(Boolean));
  function matchesTokens(hay: string): boolean {
    return tokens.every((t) => hay.includes(t));
  }
  function requestMatches(r: ApiRequest): boolean {
    return matchesTokens(`${r.method} ${r.name} ${r.url}`.toLowerCase());
  }

  function countAll(node: TreeNode): number {
    return node.items.length + node.children.reduce((n, c) => n + countAll(c), 0);
  }
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
  // Prune the tree to matching branches: keep a node when its own name matches
  // (all items kept), when any of its requests match (only those kept), or when
  // a descendant survives — so ancestor folders stay visible as context.
  function filterTree(nodes: TreeNode[]): TreeNode[] {
    return nodes.flatMap((node) => {
      if (matchesTokens(node.col.name.toLowerCase())) return [node];
      const items = node.items.filter(requestMatches);
      const children = filterTree(node.children);
      if (items.length === 0 && children.length === 0) return [];
      return [{ col: node.col, items, children }];
    });
  }
  const tree = $derived.by(() => {
    const full = buildTree(null);
    return tokens.length ? filterTree(full) : full;
  });
  const ungrouped = $derived(
    apiClient.requests
      .filter((r) => !r.collection_id && (tokens.length === 0 || requestMatches(r)))
      .sort((a, b) => a.name.localeCompare(b.name)),
  );
  const isEmpty = $derived(apiClient.collections.length === 0 && apiClient.requests.length === 0);

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
    const saved = await apiClient.saveCollection({ name, parent_id: parentId }, undefined);
    if (saved && parentId) collapsed[parentId] = false;
  }

  async function renameCollection(col: ApiCollection): Promise<void> {
    const name = await confirmer.promptText('Name', {
      title: col.parent_id ? 'Rename folder' : 'Rename collection',
      confirmLabel: 'Rename',
      initial: col.name,
    });
    if (!name || name === col.name) return;
    await apiClient.saveCollection({ name, parent_id: col.parent_id }, col.id);
  }

  async function deleteCollection(col: ApiCollection): Promise<void> {
    const kind = col.parent_id ? 'folder' : 'collection';
    if (!(await confirmer.ask(
      `Delete ${kind} “${col.name}”? Folders inside it are deleted too. Its requests are kept and move to “Ungrouped”.`,
      { title: `Delete ${kind}` },
    ))) return;
    await apiClient.deleteCollection(col.id);
  }

  async function deleteRequest(r: ApiRequest): Promise<void> {
    if (!(await confirmer.ask(`Delete the saved request “${r.name}”? Its stored credentials are removed from the Keychain. History entries stay.`, { title: 'Delete request' }))) return;
    await apiClient.deleteRequest(r.id);
  }

  function openRequest(r: ApiRequest): void {
    apiClient.loadRequestIntoDraft(r);
    onopen?.();
  }

  function newRequestIn(col: ApiCollection): void {
    collapsed[col.id] = false;
    apiClient.newDraft(col.id);
    onopen?.();
  }

  // Export the collection to OpenAPI: fetch the JSON and download it.
  async function exportOpenApi(col: ApiCollection): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    try {
      const spec = await api.get<unknown>(`/workspaces/${wsId}/api-client/collections/${col.id}/openapi`);
      const blob = new Blob([JSON.stringify(spec, null, 2)], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `${col.name.replace(/[^\w.-]+/g, '_') || 'collection'}.openapi.json`;
      document.body.appendChild(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
      toasts.success('Exported as OpenAPI', a.download);
    } catch (e) {
      toasts.error('Couldn’t export the collection', e instanceof Error ? e.message : String(e));
    }
  }

  function collectionMenu(e: MouseEvent | KeyboardEvent, col: ApiCollection): void {
    const items: MenuItem[] = [
      { label: 'New request here', icon: 'plus', action: () => newRequestIn(col) },
      ...(canEdit ? [{ label: 'New folder…', icon: 'folder', action: () => void newCollection(col.id) }] : []),
      { label: 'Export as OpenAPI', icon: 'download', action: () => void exportOpenApi(col) },
      ...(canEdit
        ? [
            { separator: true },
            { label: 'Rename…', icon: 'edit', action: () => void renameCollection(col) },
            { label: 'Delete…', icon: 'trash', danger: true, action: () => void deleteCollection(col) },
          ]
        : []),
    ];
    ctxMenu.show(e, items);
  }

  function requestMenu(e: MouseEvent | KeyboardEvent, r: ApiRequest): void {
    const items: MenuItem[] = [
      { label: 'Open', icon: 'external', action: () => openRequest(r) },
      ...(canEdit
        ? [{ separator: true }, { label: 'Delete…', icon: 'trash', danger: true, action: () => void deleteRequest(r) }]
        : []),
    ];
    ctxMenu.show(e, items);
  }
</script>

<div class="tree-wrap">
  <div class="tree-tools">
    <label class="search">
      <Icon name="search" size={12} />
      <input
        placeholder="Search requests"
        bind:value={search}
        aria-label="Search collections and requests"
        disabled={isEmpty}
      />
      {#if search}
        <button class="icon-btn clear" onclick={() => (search = '')} aria-label="Clear search" title="Clear search"><Icon name="x" size={12} /></button>
      {/if}
    </label>
    {#if canEdit}
      <button class="icon-btn" title="New collection" aria-label="New collection" onclick={() => newCollection(null)}>
        <Icon name="folder" size={14} />
      </button>
    {/if}
  </div>

  {#if apiClient.requestsLoadError && isEmpty}
    <div class="state err" role="alert">
      <Icon name="warning" size={14} />
      <div class="grow">
        <div>Couldn’t load saved requests.</div>
        <div class="dim-line">{apiClient.requestsLoadError}</div>
      </div>
      <button class="btn small" onclick={() => void apiClient.loadAll()}>Retry</button>
    </div>
  {:else if apiClient.loading && isEmpty}
    <div class="state dim" role="status">Loading saved requests…</div>
  {:else if isEmpty}
    <EmptyState
      icon="folder"
      title="No saved requests yet"
      body="Press ⌘S in a request to save it here. Collections group requests by API or feature."
    >
      {#if canEdit}
        <button class="btn ghost small" onclick={() => newCollection(null)}><Icon name="folder" size={12} />New collection</button>
      {/if}
    </EmptyState>
  {:else if tokens.length > 0 && tree.length === 0 && ungrouped.length === 0}
    <div class="state no-match">
      No requests match “{search.trim()}”.
      <button class="btn ghost small" onclick={() => (search = '')}>Clear search</button>
    </div>
  {:else}
    <div class="tree">
      {#each tree as node (node.col.id)}
        {@render collectionNode(node, 0)}
      {/each}

      {#if ungrouped.length > 0}
        <div class="section-title ungrouped">Ungrouped</div>
        {#each ungrouped as r (r.id)}
          {@render requestRow(r, 0)}
        {/each}
      {/if}
    </div>
  {/if}
</div>

{#snippet collectionNode(node: TreeNode, depth: number)}
  <!-- While filtering, force branches open so matches are never hidden. -->
  {@const isOpen = tokens.length > 0 || !collapsed[node.col.id]}
  <div class="col-head" style:padding-inline-start="{depth * 14 + 2}px">
    <button class="col-toggle" onclick={() => toggle(node.col.id)} oncontextmenu={(e) => collectionMenu(e, node.col)} aria-expanded={isOpen}>
      <Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} />
      <Icon name="folder" size={14} />
      <span class="col-name" title={node.col.name}>{node.col.name}</span>
      <span class="count" title="{countAll(node)} requests">{countAll(node)}</span>
    </button>
    <button class="icon-btn row-more" title="Actions for {node.col.name}" aria-label="Actions for {node.col.name}" onclick={(e) => collectionMenu(e, node.col)}>
      <Icon name="more" size={14} />
    </button>
  </div>
  {#if isOpen}
    {#each node.children as child (child.col.id)}
      {@render collectionNode(child, depth + 1)}
    {/each}
    {#each node.items as r (r.id)}
      {@render requestRow(r, depth + 1)}
    {/each}
    {#if node.items.length === 0 && node.children.length === 0 && tokens.length === 0}
      <button class="empty-folder" style:padding-inline-start="{(depth + 1) * 14 + 22}px" onclick={() => newRequestIn(node.col)}>
        Empty — add a request
      </button>
    {/if}
  {/if}
{/snippet}

{#snippet requestRow(r: ApiRequest, depth: number)}
  <div class="req-row" class:active={apiClient.draft.requestId === r.id} style:padding-inline-start="{depth * 14 + 20}px">
    <button class="req-open" onclick={() => openRequest(r)} oncontextmenu={(e) => requestMenu(e, r)} title="{r.method} {r.url}" aria-current={apiClient.draft.requestId === r.id ? 'true' : undefined}>
      <MethodTag method={r.method} fixed />
      <span class="rname">{r.name}</span>
    </button>
    <button class="icon-btn row-more" title="Actions for {r.name}" aria-label="Actions for {r.name}" onclick={(e) => requestMenu(e, r)}>
      <Icon name="more" size={14} />
    </button>
  </div>
{/snippet}

<style>
  .tree-wrap {
    display: flex;
    flex-direction: column;
    min-height: 0;
    gap: 8px;
  }
  .tree-tools {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .search {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 27px;
    padding: 0 4px 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-m);
    outline: none;
  }
  .search input::placeholder {
    color: var(--text-dim);
  }
  .clear {
    width: 20px;
    height: 20px;
  }
  .state {
    font-size: var(--fs-s);
    padding: 8px 4px;
  }
  .state.dim,
  .no-match {
    color: var(--text-dim);
  }
  .no-match {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
  }
  .state.err {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    color: var(--text);
  }
  .state.err :global(svg) {
    color: var(--danger);
    margin-top: 2px;
  }
  .dim-line {
    color: var(--text-dim);
    font-size: var(--fs-xs);
    word-break: break-word;
  }
  .tree {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .ungrouped {
    margin: 12px 0 4px 4px;
  }
  .col-head,
  .req-row {
    display: flex;
    align-items: center;
    min-height: 28px;
    padding-inline-end: 2px;
    border-radius: var(--radius-s);
  }
  .col-head:hover,
  .req-row:hover {
    background: var(--hover);
  }
  .req-row.active {
    background: var(--accent-soft);
  }
  .col-toggle,
  .req-open {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 4px;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: start;
    font-size: var(--fs-m);
    border-radius: var(--radius-s);
  }
  .col-toggle {
    color: var(--text-dim);
  }
  .col-name {
    color: var(--text);
    font-weight: 500;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    margin-inline-start: auto;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .rname {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row-more {
    opacity: 0;
  }
  .col-head:hover .row-more,
  .req-row:hover .row-more,
  .row-more:focus-visible,
  .req-row.active .row-more {
    opacity: 1;
  }
  .empty-folder {
    height: 26px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
    border-radius: var(--radius-s);
  }
  .empty-folder:hover {
    color: var(--accent-text);
    background: var(--hover);
  }
  @media (max-width: 1024px) {
    .row-more {
      opacity: 1;
    }
  }
</style>
