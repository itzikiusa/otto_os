<script lang="ts">
  // Saved requests, grouped into collections and nested folders (parent_id).
  // Click a request to open it (an already-open or edited tab is never
  // overwritten — see apiClient.placeDraft). Row actions live in one ⋯ /
  // right-click menu per row instead of four always-visible icons.
  import Icon from '../../lib/components/Icon.svelte';
  import { toastError } from '../../lib/toastError';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import VirtualList from '../../lib/components/VirtualList.svelte';
  import MethodTag, { methodWord } from './MethodTag.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { api } from '../../lib/api/client';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import type { ApiCollection, ApiRequestItem as ApiRequest } from '../../lib/api/types';

  interface Props {
    /** A request was opened / created (the page shows the editor). */
    onopen?: () => void;
  }
  let { onopen }: Props = $props();

  interface TreeNode {
    col: ApiCollection;
    items: ApiRequest[];
    children: TreeNode[];
    /** Requests in this node + all descendants (as shown — filtered while searching). */
    count: number;
  }
  /** One flattened, fixed-height row of the windowed tree. */
  type Row =
    | { kind: 'col'; id: string; node: TreeNode; depth: number; open: boolean }
    | { kind: 'req'; id: string; r: ApiRequest; depth: number }
    | { kind: 'empty'; id: string; col: ApiCollection; depth: number }
    | { kind: 'section'; id: string };
  const ROW_H = 29; // 28 px row + 1 px gap
  /** ⌘F text of a tree row — what it shows, in DOM order. */
  function rowFindText(row: Row): string {
    if (row.kind === 'col') return `${row.node.col.name}\n${row.node.count}`;
    if (row.kind === 'req') return `${methodWord(row.r.method)}\n${row.r.name}`;
    if (row.kind === 'empty') return 'Empty — add a request';
    return 'Ungrouped';
  }

  let collapsed: Record<string, boolean> = $state({});
  const canEdit = $derived(ws.myRole !== 'viewer');
  // Rows compare against this derived, NOT `apiClient.draft.requestId`: the
  // draft is replaced on every keystroke in the builder, and a derived only
  // notifies when its value changes — so typing no longer re-runs every row.
  const activeRequestId = $derived(apiClient.draft.requestId ?? null);

  // Search: every whitespace-separated token must match the request's
  // method/name/url (so "get users staging" narrows by all three). A collection
  // whose own name matches keeps all of its requests. The input is debounced
  // into `query`, and each request's haystack is lowercased once per list
  // change (not per keystroke × request).
  let search = $state('');
  let query = $state('');
  let _searchTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const v = search;
    clearTimeout(_searchTimer);
    // Clearing is instant; typing settles for 120 ms.
    if (!v.trim()) query = '';
    else _searchTimer = setTimeout(() => { query = v; }, 120);
    return () => clearTimeout(_searchTimer);
  });
  const tokens = $derived(query.trim().toLowerCase().split(/\s+/).filter(Boolean));
  function matchesTokens(hay: string): boolean {
    return tokens.every((t) => hay.includes(t));
  }
  // Haystacks aligned with `apiClient.requests` (a raw array: plain reads).
  const hay = $derived(apiClient.requests.map((r) => `${r.method} ${r.name} ${r.url}`.toLowerCase()));
  // The matching request ids, computed ONCE per query in one plain loop — the
  // tree filter below then only does Set lookups (it used to read two derived
  // signals per request per filter pass: ~25 ms of a 50 ms frame at 3k).
  const matched = $derived.by(() => {
    if (tokens.length === 0) return null;
    const toks = tokens;
    const h = hay;
    const reqs = apiClient.requests;
    const out = new Set<string>();
    for (let i = 0; i < reqs.length; i++) {
      const s = h[i];
      let ok = true;
      for (const t of toks) if (!s.includes(t)) { ok = false; break; }
      if (ok) out.add(reqs[i].id);
    }
    return out;
  });

  // Index once per list change: O(R log R + C log C), instead of re-filtering
  // every request and collection for every collection (O(C·(R+C))).
  const byOrder = <T extends { position: number; name: string }>(a: T, b: T): number =>
    a.position - b.position || a.name.localeCompare(b.name);
  const childCols = $derived.by(() => {
    const m = new Map<string | null, ApiCollection[]>();
    for (const c of apiClient.collections) {
      const k = c.parent_id ?? null;
      const list = m.get(k);
      if (list) list.push(c);
      else m.set(k, [c]);
    }
    for (const list of m.values()) list.sort(byOrder);
    return m;
  });
  const reqsByCol = $derived.by(() => {
    const m = new Map<string, ApiRequest[]>();
    for (const r of apiClient.requests) {
      if (!r.collection_id) continue;
      const list = m.get(r.collection_id);
      if (list) list.push(r);
      else m.set(r.collection_id, [r]);
    }
    for (const list of m.values()) list.sort(byOrder);
    return m;
  });

  function buildTree(parentId: string | null, seen: Set<string>): TreeNode[] {
    return (childCols.get(parentId) ?? []).flatMap((col) => {
      if (seen.has(col.id)) return []; // a parent_id cycle must not recurse forever
      seen.add(col.id);
      const items = reqsByCol.get(col.id) ?? [];
      const children = buildTree(col.id, seen);
      return [{ col, items, children, count: items.length + children.reduce((n, c) => n + c.count, 0) }];
    });
  }
  // Prune the tree to matching branches: keep a node when its own name matches
  // (all items kept), when any of its requests match (only those kept), or when
  // a descendant survives — so ancestor folders stay visible as context.
  function filterTree(nodes: TreeNode[], hit: Set<string>): TreeNode[] {
    return nodes.flatMap((node) => {
      if (matchesTokens(node.col.name.toLowerCase())) return [node];
      const items = node.items.filter((r) => hit.has(r.id));
      const children = filterTree(node.children, hit);
      if (items.length === 0 && children.length === 0) return [];
      return [{ col: node.col, items, children, count: items.length + children.reduce((n, c) => n + c.count, 0) }];
    });
  }
  const fullTree = $derived(buildTree(null, new Set()));
  const tree = $derived(matched ? filterTree(fullTree, matched) : fullTree);
  const ungrouped = $derived(
    apiClient.requests
      .filter((r) => !r.collection_id && (matched === null || matched.has(r.id)))
      .sort((a, b) => a.name.localeCompare(b.name)),
  );
  const isEmpty = $derived(apiClient.collections.length === 0 && apiClient.requests.length === 0);
  // Big workspaces open with folders collapsed (an explicit toggle wins).
  const defaultCollapsed = $derived(apiClient.requests.length > 500);
  function isCollapsed(id: string): boolean {
    return collapsed[id] ?? defaultCollapsed;
  }

  // Flatten the visible tree for the windowed list (only ~viewport rows mount).
  const rows = $derived.by(() => {
    const out: Row[] = [];
    const filtering = tokens.length > 0;
    const walk = (nodes: TreeNode[], depth: number): void => {
      for (const node of nodes) {
        // While filtering, force branches open so matches are never hidden.
        const open = filtering || !isCollapsed(node.col.id);
        out.push({ kind: 'col', id: `c:${node.col.id}`, node, depth, open });
        if (!open) continue;
        walk(node.children, depth + 1);
        for (const r of node.items) out.push({ kind: 'req', id: `r:${r.id}`, r, depth: depth + 1 });
        if (node.items.length === 0 && node.children.length === 0 && !filtering) {
          out.push({ kind: 'empty', id: `e:${node.col.id}`, col: node.col, depth: depth + 1 });
        }
      }
    };
    walk(tree, 0);
    if (ungrouped.length > 0) {
      out.push({ kind: 'section', id: 'ungrouped' });
      for (const r of ungrouped) out.push({ kind: 'req', id: `r:${r.id}`, r, depth: 0 });
    }
    return out;
  });
  const reqCount = $derived(rows.reduce((n, row) => n + (row.kind === 'req' ? 1 : 0), 0));
  // Keep the open request's row mounted (and its highlight) while scrolled away.
  const activeIndex = $derived(activeRequestId ? rows.findIndex((row) => row.kind === 'req' && row.r.id === activeRequestId) : -1);

  function toggle(id: string): void {
    collapsed[id] = !isCollapsed(id);
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

  // The tree holds summaries; the full row is fetched on open (perf2 N1).
  async function openRequest(r: ApiRequest): Promise<void> {
    if (await apiClient.openRequest(r.id)) onopen?.();
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
      toastError('Couldn’t export the collection', e);
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
      { label: 'Open', icon: 'external', action: () => void openRequest(r) },
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
  {:else if tokens.length > 0 && rows.length === 0}
    <div class="state no-match">
      No requests match “{query.trim()}”.
      <button class="btn ghost small" onclick={() => (search = '')}>Clear search</button>
    </div>
  {:else}
    <VirtualList items={rows} estimateHeight={ROW_H} class="tree" key={(r) => r.id} pinnedIndex={activeIndex} findText={rowFindText}>
      {#snippet row(item: Row)}
        {#if item.kind === 'col'}
          {@render collectionNode(item.node, item.depth, item.open)}
        {:else if item.kind === 'req'}
          {@render requestRow(item.r, item.depth)}
        {:else if item.kind === 'empty'}
          <div class="vrow">
            <button class="empty-folder" style:padding-inline-start="{item.depth * 14 + 22}px" onclick={() => newRequestIn(item.col)}>
              Empty — add a request
            </button>
          </div>
        {:else}
          <div class="vrow"><div class="section-title ungrouped">Ungrouped</div></div>
        {/if}
      {/snippet}
    </VirtualList>
    {#if tokens.length > 0}<span class="sr-only" role="status">{reqCount} matching requests</span>{/if}
  {/if}
</div>

{#snippet collectionNode(node: TreeNode, depth: number, isOpen: boolean)}
  <div class="vrow">
    <div class="col-head" style:padding-inline-start="{depth * 14 + 2}px">
      <button class="col-toggle" onclick={() => toggle(node.col.id)} oncontextmenu={(e) => collectionMenu(e, node.col)} aria-expanded={isOpen}>
        <Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} />
        <Icon name="folder" size={14} />
        <span class="col-name" title={node.col.name}>{node.col.name}</span>
        <span class="count" title="{node.count} requests">{node.count}</span>
      </button>
      <button class="icon-btn row-more" title="Actions for {node.col.name}" aria-label="Actions for {node.col.name}" onclick={(e) => collectionMenu(e, node.col)}>
        <Icon name="more" size={14} />
      </button>
    </div>
  </div>
{/snippet}

{#snippet requestRow(r: ApiRequest, depth: number)}
  <div class="vrow">
    <div class="req-row" class:active={activeRequestId === r.id} style:padding-inline-start="{depth * 14 + 20}px">
      <button class="req-open" onclick={() => openRequest(r)} oncontextmenu={(e) => requestMenu(e, r)} title="{r.method} {r.url}" aria-current={activeRequestId === r.id ? 'true' : undefined}>
        <MethodTag method={r.method} fixed />
        <span class="rname">{r.name}</span>
      </button>
      <button class="icon-btn row-more" title="Actions for {r.name}" aria-label="Actions for {r.name}" onclick={(e) => requestMenu(e, r)}>
        <Icon name="more" size={14} />
      </button>
    </div>
  </div>
{/snippet}

<style>
  .tree-wrap {
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 1;
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
    padding-block: 0; padding-inline: 8px 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .search:focus-within {
    border-color: var(--accent-text);
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
  /* Windowed list: every row is a fixed 29 px slot (28 px + 1 px gap). */
  .tree-wrap :global(.tree) {
    flex: 1;
    min-height: 0;
  }
  .vrow {
    height: 29px;
    padding-bottom: 1px;
    box-sizing: border-box;
  }
  .ungrouped {
    display: flex;
    align-items: flex-end;
    height: 100%;
    margin: 0;
    padding-inline-start: 4px;
    padding-bottom: 4px;
    box-sizing: border-box;
  }
  .col-head,
  .req-row {
    display: flex;
    align-items: center;
    height: 28px;
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
    width: 100%;
    height: 28px;
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
