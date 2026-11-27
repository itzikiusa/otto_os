<script lang="ts">
  import type { Snippet } from 'svelte';
  import { rowMenu } from '../../lib/rowMenu';
  // Saved requests, grouped into collections and nested folders (parent_id).
  // Click a request to open it (an already-open or edited tab is never
  // overwritten — see apiClient.placeDraft). Row actions live in one ⋯ /
  // right-click menu per row instead of four always-visible icons.
  import Icon from '../../lib/components/Icon.svelte';
  import { plural } from '../../lib/plural';
  import { toastError } from '../../lib/toastError';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import VirtualList from '../../lib/components/VirtualList.svelte';
  import ListPane from '../../lib/components/side/ListPane.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import MethodTag, { methodWord } from './MethodTag.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { api } from '../../lib/api/client';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import type { ApiCollection, ApiRequestItem as ApiRequest } from '../../lib/api/types';

  interface Props {
    /** The pane's views (Collections | Automations | History), its header. */
    views?: Snippet;
    /** A request was opened / created (the page shows the editor). */
    onopen?: () => void;
  }
  let { views, onopen }: Props = $props();

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
  // The shared row (30 px, 40 px on touch screens) + a 1 px gap.
  const ROW_H = $derived(viewport.isMobile ? 41 : 31);
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

{#snippet searchField()}
  <label class="input-group">
    <Icon name="search" size={12} />
    <input dir="auto" placeholder="Filter requests…" bind:value={search} aria-label="Search collections and requests" />
    {#if search}
      <button class="icon-btn clear" onclick={() => (search = '')} aria-label="Clear search" title="Clear search"><Icon name="x" size={12} /></button>
    {/if}
  </label>
{/snippet}

<ListPane
  {views}
  menu={canEdit ? () => [{ label: 'New collection…', icon: 'folder', action: () => void newCollection(null) }] : undefined}
  searchRow={isEmpty ? undefined : searchField}
  loading={isEmpty && apiClient.listsPending}
  error={isEmpty ? apiClient.requestsLoadError : null}
  onretry={() => void apiClient.loadAll()}
>
  <div class="tree-wrap" role="tree" aria-label="Collections" style:--request-row-height={`${ROW_H - 1}px`}>
    {#if isEmpty}
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
            <div class="vrow" role="none">
              <button class="side-row-main empty-folder" data-side-row style:--side-depth={item.depth + 1} onclick={() => newRequestIn(item.col)}>
                Empty — add a request
              </button>
            </div>
          {:else}
            <div class="vrow" role="none"><div class="side-section-head ungrouped"><span class="ssh-title">Ungrouped</span></div></div>
          {/if}
        {/snippet}
      </VirtualList>
    {/if}
    <!-- Always mounted (A11y-5-15): a live region inserted WITH its text is silent. -->
    <span class="sr-only" role="status">{tokens.length > 0 ? plural(reqCount, 'matching request') : ''}</span>
  </div>
</ListPane>

{#snippet collectionNode(node: TreeNode, depth: number, isOpen: boolean)}
  <div class="vrow" role="none">
    <div class="side-row col-head" style:--side-depth={depth}>
      <button use:rowMenu class="side-row-main col-toggle" data-side-row data-side-toggle role="treeitem" aria-level={depth + 1} aria-selected="false" aria-expanded={isOpen} onclick={() => toggle(node.col.id)} oncontextmenu={(e) => collectionMenu(e, node.col)}>
        <Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} />
        <Icon name="folder" size={14} />
        <span class="side-row-title col-name" title={node.col.name}>{node.col.name}</span>
        <span class="side-row-trailing count" title="{node.count} requests">{node.count}</span>
      </button>
      <button class="icon-btn side-row-more" tabindex="-1" title="Actions for {node.col.name}" aria-label="Actions for {node.col.name}" onclick={(e) => collectionMenu(e, node.col)}>
        <Icon name="more" size={14} />
      </button>
    </div>
  </div>
{/snippet}

{#snippet requestRow(r: ApiRequest, depth: number)}
  <div class="vrow" role="none">
    <div class="side-row req-row" class:selected={activeRequestId === r.id} style:--side-depth={depth + 1}>
      <button use:rowMenu class="side-row-main req-open" data-side-row role="treeitem" aria-level={depth + 1} aria-selected={activeRequestId === r.id} onclick={() => openRequest(r)} oncontextmenu={(e) => requestMenu(e, r)} title="{r.method} {r.url}">
        <MethodTag method={r.method} fixed />
        <span class="side-row-title rname">{r.name}</span>
      </button>
      <button class="icon-btn side-row-more" tabindex="-1" title="Actions for {r.name}" aria-label="Actions for {r.name}" onclick={(e) => requestMenu(e, r)}>
        <Icon name="more" size={14} />
      </button>
    </div>
  </div>
{/snippet}

<style>
  .tree-wrap {
    display: flex;
    flex-direction: column;
    block-size: 100%;
    min-height: 0;
  }
  .clear {
    inline-size: 20px;
    block-size: 20px;
  }
  @media (pointer: coarse) {
    .clear {
      inline-size: var(--hit-min);
      block-size: var(--hit-min);
    }
  }
  .state {
    font-size: var(--fs-s);
    padding: 8px;
  }
  .no-match {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    color: var(--text-dim);
  }
  /* Windowed list: every row is a fixed slot (the row + 1 px gap), the row's
     height set here so it always matches the list's estimate. */
  .tree-wrap :global(.tree) {
    flex: 1;
    min-height: 0;
  }
  .vrow {
    height: calc(var(--request-row-height) + 1px);
    padding-bottom: 1px;
    box-sizing: border-box;
  }
  .vrow .side-row-main {
    block-size: var(--request-row-height);
    min-block-size: 0;
  }
  .ungrouped {
    align-items: flex-end;
    block-size: 100%;
    padding-block-end: 4px;
    box-sizing: border-box;
  }
  .col-name {
    font-weight: 500;
  }
  .count {
    margin-inline-start: auto;
  }
  .empty-folder {
    inline-size: 100%;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .empty-folder:hover {
    color: var(--accent-text);
    background: var(--hover);
  }
</style>
