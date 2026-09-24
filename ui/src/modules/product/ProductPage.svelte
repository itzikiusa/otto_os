<script lang="ts">
  // Product Story Analysis page — left sidebar (Stories | Learnings toggle +
  // story list + import) and a per-story workspace: ONE header band buckets the
  // 13 sub-views into 4 workflow GROUPS (Story · Discover · Deliver · Log) —
  // group tabs inline-start, the active group's sub-views as pills inline-end —
  // with the selected sub-view's content below.
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
  import { storyStage } from '../../lib/status';
  import { untrack } from 'svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { recallSelection, rememberSelection } from '../../lib/lastSelection';
  import { product, buildTree, type TreeNode } from '../../lib/stores/product.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import ImportDialog from './ImportDialog.svelte';
  import OverviewTab from './OverviewTab.svelte';
  import AnalysisTab from './AnalysisTab.svelte';
  import QuestionsTab from './QuestionsTab.svelte';
  import NotesTab from './NotesTab.svelte';
  import RewriteTab from './RewriteTab.svelte';
  import TestCasesTab from './TestCasesTab.svelte';
  import PlanTab from './PlanTab.svelte';
  import HistoryTab from './HistoryTab.svelte';
  import InjectTab from './InjectTab.svelte';
  import DiscoveryTab from './DiscoveryTab.svelte';
  import MockupsTab from './MockupsTab.svelte';
  import RefineTab from './RefineTab.svelte';
  import ChatTab from './ChatTab.svelte';
  import LearningsView from './LearningsView.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { ProductStory, TreeKind } from './types';

  let importOpen = $state(false);
  let draftCreating = $state(false);

  // ── Epic tree state (design §3.2) ─────────────────────────────────────────
  // Collapsed epics / folders (keys: epic id, `${epicId}/${folder}`). The tree
  // itself is derived from the (tag-filtered) flat list via the store's buildTree.
  let collapsedEpics = $state<Record<string, boolean>>({});
  let collapsedFolders = $state<Record<string, boolean>>({});

  async function createEpic(): Promise<void> {
    draftCreating = true;
    try {
      await product.createEpic();
      product.tab = 'overview';
      mobileSection = 'content';
    } catch (e) {
      toasts.error('Could not create the epic', product.errMsg(e));
    } finally {
      draftCreating = false;
    }
  }

  /** Toolbar **New ▾** → Draft · Epic. */
  function newMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Draft', icon: 'file', action: () => void createDraft() },
      { label: 'Epic', icon: 'folder', action: () => void createEpic() },
    ]);
  }

  /** Inside an epic: **Add child ▾** → Story · Doc (a folder is asked for). */
  function addChildMenu(e: MouseEvent, epic: ProductStory): void {
    ctxMenu.show(e, [
      { label: 'Story (full tab strip)', icon: 'file', action: () => void addChild(epic, 'story') },
      { label: 'Doc (lightweight note / spec section)', icon: 'note', action: () => void addChild(epic, 'doc') },
    ]);
  }
  async function addChild(epic: ProductStory, kind: TreeKind): Promise<void> {
    const title = await confirmer.promptText(`Title of the new ${kind} under "${epic.title}":`, {
      title: kind === 'doc' ? 'Add doc' : 'Add story', confirmLabel: 'Create', placeholder: 'e.g. Tier ladder screens',
    });
    if (!title) return;
    const existing = [...new Set(product.childrenOf(epic.id).map((c) => c.folder).filter(Boolean))];
    const folder = await confirmer.promptText(
      existing.length ? `Folder (existing: ${existing.join(', ')}) — leave empty for none:` : 'Folder (e.g. Design, PO) — leave empty for none:',
      { title: 'Folder', confirmLabel: 'Create', placeholder: 'Design' },
    );
    try {
      await product.createChild(epic.id, { title, tree_kind: kind, folder: folder ?? '' });
      collapsedEpics = { ...collapsedEpics, [epic.id]: false };
      product.tab = 'overview';
      mobileSection = 'content';
    } catch (e) {
      toasts.error('Could not add the child', product.errMsg(e));
    }
  }

  /** Right-click / ⋯ on a story row: Move to epic… · Set folder… · Mark as epic ·
   *  Detach from epic · Delete. */
  function storyMenu(e: MouseEvent | KeyboardEvent, s: ProductStory, node?: TreeNode): void {
    const isEpicRow = !!node?.isEpic;
    const items: MenuItem[] = [];
    // Nested menus: ContextMenu runs `action()` THEN closes, so a menu opened
    // synchronously from an action would be closed on the same tick — defer it.
    if (isEpicRow) {
      items.push({ label: 'Add child…', icon: 'plus', action: () => setTimeout(() => addChildMenu(e as MouseEvent, s), 0) });
      items.push({ separator: true });
    }
    if (!isEpicRow || node?.childCount === 0) {
      items.push({ label: 'Move to epic…', icon: 'folder', action: () => setTimeout(() => moveToEpicMenu(e, s), 0) });
    }
    if (s.parent_id) {
      items.push({ label: 'Set folder…', icon: 'edit', action: () => void setFolder(s) });
      items.push({ label: 'Detach from epic', icon: 'x', action: () => void detach(s) });
    }
    if (!s.parent_id) {
      items.push(
        s.tree_kind === 'epic'
          ? { label: 'Unmark as epic', icon: 'folder', disabled: (node?.childCount ?? 0) > 0, action: () => void mark(s, 'story') }
          : { label: 'Mark as epic', icon: 'folder', action: () => void mark(s, 'epic') },
      );
    } else {
      items.push(
        s.tree_kind === 'doc'
          ? { label: 'Make a full story', icon: 'file', action: () => void mark(s, 'story') }
          : { label: 'Make a doc', icon: 'note', action: () => void mark(s, 'doc') },
      );
    }
    items.push({ separator: true });
    items.push({ label: 'Delete', icon: 'trash', danger: true, action: () => void deleteStory(s) });
    ctxMenu.show(e, items);
  }
  /** Picker: every top-level row can become the parent (epics first). The menu is
   *  filterable + capped, so a long story list stays scrollable/reachable. */
  function moveToEpicMenu(e: MouseEvent | KeyboardEvent, s: ProductStory): void {
    const tops = product.tree.filter((n) => n.story.id !== s.id);
    const epics = tops.filter((n) => n.isEpic);
    const others = tops.filter((n) => !n.isEpic && !n.story.parent_id);
    const item = (n: TreeNode): MenuItem => ({
      label: `${n.story.title}${n.isEpic ? ` (epic · ${n.childCount})` : ''}`,
      icon: n.isEpic ? 'folder' : 'file',
      action: () => void moveTo(s, n.story),
    });
    const items: MenuItem[] = [...epics.map(item)];
    if (epics.length && others.length) items.push({ separator: true, pinned: true });
    items.push(...others.map(item));
    if (!items.length) items.push({ label: 'No other top-level story to move under', disabled: true });
    // Re-anchor at the same spot (the first menu closed on click).
    ctxMenu.show(e, items, { filter: true, filterPlaceholder: 'Find an epic…', maxVisible: 12 });
  }
  async function moveTo(s: ProductStory, epic: ProductStory): Promise<void> {
    try {
      await product.moveStory(s.id, epic.id, s.folder || '');
      collapsedEpics = { ...collapsedEpics, [epic.id]: false };
    } catch (e) {
      toasts.error('Move failed', product.errMsg(e));
    }
  }
  async function setFolder(s: ProductStory): Promise<void> {
    const existing = s.parent_id ? [...new Set(product.childrenOf(s.parent_id).map((c) => c.folder).filter(Boolean))] : [];
    const folder = await confirmer.promptText(
      existing.length ? `Folder (existing: ${existing.join(', ')}). Empty = unfiled:` : 'Folder name (e.g. Design, PO). Empty = unfiled:',
      { title: 'Set folder', confirmLabel: 'Save', initial: s.folder, placeholder: 'Design' },
    );
    if (folder === s.folder) return;
    try {
      await product.patchStory(s.id, { folder: folder ?? '' });
    } catch (e) {
      toasts.error('Could not set the folder', product.errMsg(e));
    }
  }
  async function detach(s: ProductStory): Promise<void> {
    try {
      await product.moveStory(s.id, null, '');
    } catch (e) {
      toasts.error('Detach failed', product.errMsg(e));
    }
  }
  async function mark(s: ProductStory, kind: TreeKind): Promise<void> {
    try {
      await product.setTreeKind(s.id, kind);
    } catch (e) {
      toasts.error('Could not change the tree role', product.errMsg(e));
    }
  }
  function toggleEpic(id: string): void {
    collapsedEpics = { ...collapsedEpics, [id]: !collapsedEpics[id] };
  }
  function toggleFolder(epicId: string, folder: string): void {
    const k = `${epicId}/${folder}`;
    collapsedFolders = { ...collapsedFolders, [k]: !collapsedFolders[k] };
  }
  const tabsHiddenForDoc = new Set(['analysis', 'discovery', 'plan', 'testcases', 'inject']);

  // ── Mobile (≤640px) accordion state ───────────────────────────────────────
  // On a phone the two panels (story list + the per-story content) stack and
  // each becomes an independently-scrollable, collapsible section. Exactly one
  // is expanded at a time so the open panel gets the full remaining height to
  // scroll in; the other shows just its tappable header. This is a no-op on
  // desktop/tablet where the CSS for these classes is never applied.
  let mobileSection = $state<'list' | 'content'>('list');

  async function createDraft(): Promise<void> {
    draftCreating = true;
    try {
      await product.createDraft();
      product.tab = 'overview';
      // On mobile, reveal the new draft's content panel right away.
      mobileSection = 'content';
    } catch (e) {
      console.error('createDraft failed', e);
    } finally {
      draftCreating = false;
    }
  }

  let learningsFilter = $state<'all' | 'pattern' | 'avoid'>('all');

  // Tag filter state
  let activeTagFilter = $state<string | null>(null);

  /** Parse csv tags → deduplicated, trimmed, non-empty array.
   *  Null-safe: a missing/garbage `tags` value must never throw, or it would
   *  break the `allTags`/`filteredStories` deriveds and freeze the whole page. */
  function parseTags(csv: string | null | undefined): string[] {
    if (!csv) return [];
    return [...new Set(csv.split(',').map((t) => t.trim()).filter(Boolean))];
  }

  /** All distinct tags across all stories, sorted. */
  const allTags = $derived(
    [...new Set(product.stories.flatMap((s) => parseTags(s.tags)))].sort(),
  );

  /** Stories shown after applying the tag filter. */
  const filteredStories = $derived(
    activeTagFilter === null
      ? product.stories
      : product.stories.filter((s) => parseTags(s.tags).includes(activeTagFilter!)),
  );
  /** The epic tree over the FILTERED list (the tag filter applies to the
   *  flattened list; a matching child whose epic didn't match shows at top level). */
  const tree = $derived(buildTree(filteredStories));

  /** Stories view with nothing at all to list: the empty list pane is hidden
   *  (≥641px) and the page EmptyState owns the ONE import CTA. */
  const noStories = $derived(product.view === 'stories' && !product.loadingStories && product.stories.length === 0);

  // List/detail never opens onto an empty "pick one" pane: once a workspace's
  // stories are in, restore the last story opened here (or the first in the
  // tree). Once per workspace, so closing a story is respected. Not on phone —
  // there the accordion's list panel is the landing view.
  let autoPickedFor: string | null = null;
  $effect(() => {
    const w = ws.currentId;
    if (!w || autoPickedFor === w || product.loadingStories || product.view !== 'stories') return;
    if (product.stories.length === 0) return;
    untrack(() => {
      autoPickedFor = w;
      if (product.selectedId || viewport.isPhone) return;
      const last = recallSelection('product');
      const pick = product.stories.find((x) => x.id === last) ?? tree[0]?.story ?? product.stories[0];
      if (pick) void product.select(pick.id);
    });
  });
  /** The selected story's parent epic (breadcrumb) and epic-ness (Add child ▾). */
  const selectedStory = $derived(product.detail?.story ?? null);
  const selectedParent = $derived(product.parentOf(selectedStory));
  const selectedIsEpic = $derived(
    !!selectedStory && (selectedStory.tree_kind === 'epic' || product.childrenOf(selectedStory.id).length > 0),
  );

  // Reload stories whenever the workspace changes (mirrors DatabasePage pattern).
  $effect(() => {
    if (ws.currentId) {
      // A workspace switch leaves no artifact open: release the arena's cached
      // blob URLs / editor bases before the new list loads.
      product.teardown();
      void product.loadStories();
    }
  });

  // ── Workflow groups ───────────────────────────────────────────────────────
  // The 13 per-story sub-views are bucketed into 4 workflow groups. The top bar
  // shows the 4 group labels; below it a secondary sub-nav lists the active
  // group's sub-views (only when the group has more than one). The render
  // cascade below stays keyed on the flat `product.tab` (the sub id), so every
  // existing tab component renders unchanged — only the navigation is regrouped.
  type Sub = { id: string; label: string };
  // `icon` is purely cosmetic (the merged header band renders it beside the
  // group label) — `id`/`label`/`subs` stay byte-for-byte what they were: the
  // sub `id`s ARE the `product.tab` values that E2E + deep links depend on.
  type Group = { id: string; label: string; icon: IconName; subs: Sub[] };
  const GROUPS: Group[] = [
    {
      id: 'story',
      label: 'Story',
      icon: 'file',
      subs: [
        { id: 'overview', label: 'Overview' },
        { id: 'rewrite', label: 'Rewrite' },
        { id: 'mockups', label: 'Design' },
      ],
    },
    {
      id: 'discover',
      label: 'Discover',
      icon: 'search',
      subs: [
        { id: 'chat', label: 'Chat' },
        { id: 'analysis', label: 'Analysis' },
        { id: 'questions', label: 'Questions' },
        { id: 'notes', label: 'Notes' },
        { id: 'discovery', label: 'Discovery' },
        { id: 'refine', label: 'Refine' },
      ],
    },
    {
      id: 'deliver',
      label: 'Deliver',
      icon: 'send',
      subs: [
        { id: 'plan', label: 'Plan' },
        { id: 'testcases', label: 'Test Cases' },
        { id: 'inject', label: 'Inject' },
      ],
    },
    {
      id: 'log',
      label: 'Log',
      icon: 'clock',
      subs: [{ id: 'history', label: 'History' }],
    },
  ];

  /** A `tree_kind:'doc'` child is a lightweight note: it hides the analysis /
   *  plan / delivery tabs (design §2.1); `story` children get the full strip. */
  const visibleGroups = $derived.by(() => {
    if (selectedStory?.tree_kind !== 'doc') return GROUPS;
    return GROUPS.map((g) => ({ ...g, subs: g.subs.filter((s) => !tabsHiddenForDoc.has(s.id)) })).filter((g) => g.subs.length > 0);
  });
  // A doc landing on a hidden tab (deep link / kind change) falls back to overview.
  $effect(() => {
    if (selectedStory?.tree_kind === 'doc' && tabsHiddenForDoc.has(product.tab)) product.tab = 'overview';
  });

  /** The group that owns the currently-active sub (`product.tab`). Falls back to
   *  the first group if the tab is somehow unknown (keeps the top bar coherent). */
  const activeGroup = $derived(
    visibleGroups.find((g) => g.subs.some((s) => s.id === product.tab)) ?? visibleGroups[0],
  );

  /** Click a group: if the current sub isn't already inside it, land on the
   *  group's first sub (otherwise keep the current sub so re-clicking is a no-op). */
  function selectGroup(g: Group): void {
    if (!g.subs.some((s) => s.id === product.tab)) {
      product.tab = g.subs[0].id;
    }
  }

  function sourceIcon(kind: string): IconName {
    switch (kind) {
      case 'jira': return 'ticket';
      case 'confluence': return 'globe';
      default: return 'file';
    }
  }

  function selectStory(s: ProductStory): void {
    void product.select(s.id);
    rememberSelection('product', s.id);
    // Reset to overview whenever a new story is selected.
    product.tab = 'overview';
    // On mobile, switch to the content panel so the picked story is visible.
    mobileSection = 'content';
  }

  /** The list in the order the tree shows it (epics followed by their
   *  children), so "the next story" after a delete is the row below it. */
  function visibleOrder(): ProductStory[] {
    const out: ProductStory[] = [];
    for (const n of tree) {
      out.push(n.story);
      for (const f of n.folders) out.push(...f.children);
    }
    return out;
  }

  async function deleteStory(s: ProductStory): Promise<void> {
    const ok = await confirmer.ask(
      `Delete "${s.title}"? This removes it from Otto (the Jira/Confluence item is untouched).`,
      { title: 'Delete story', confirmLabel: 'Delete', danger: true },
    );
    if (!ok) return;
    // Deleting the open story lands on its neighbour (the row below, else the
    // one above) — never on an empty "pick one" pane.
    const wasOpen = product.selectedId === s.id;
    const order = visibleOrder();
    const i = order.findIndex((x) => x.id === s.id);
    const gone = new Set([s.id, ...product.childrenOf(s.id).map((c) => c.id)]);
    const next =
      order.slice(i + 1).find((x) => !gone.has(x.id)) ??
      order.slice(0, Math.max(i, 0)).reverse().find((x) => !gone.has(x.id)) ??
      null;
    try {
      await product.deleteStory(s.id);
    } catch (e) {
      toasts.error(`Couldn't delete "${s.title}"`, product.errMsg(e));
      return;
    }
    if (wasOpen && next && product.stories.some((x) => x.id === next.id)) {
      product.tab = 'overview';
      rememberSelection('product', next.id);
      void product.select(next.id);
    }
  }

  /** Collection summary (shown only when no story is open): counts per stage
   *  and the most recently touched stories, each one click away. */
  const stageCounts = $derived.by(() => {
    const m = new Map<string, number>();
    for (const s of product.stories) if (s.tree_kind !== 'doc') m.set(s.stage, (m.get(s.stage) ?? 0) + 1);
    return [...m.entries()].sort((a, b) => b[1] - a[1]);
  });
  const recentStories = $derived(
    [...product.stories].sort((a, b) => (b.updated_at ?? '').localeCompare(a.updated_at ?? '')).slice(0, 8),
  );

  /** Header title: the open story (Stories view), else the module. The parent
   *  epic is the one crumb, so a child reads `Epic / Child`. */
  const headerTitle = $derived(
    product.view === 'stories' && selectedStory ? selectedStory.title : 'Product',
  );
  const headerCrumbs = $derived(
    product.view === 'stories' && selectedParent
      ? [{ label: selectedParent.title, onclick: () => void product.select(selectedParent.id) }]
      : [],
  );

  // ── Sidebar width (drag-resizable, persisted) ─────────────────────────────
  // Mirrors the DatabasePage sidebar idiom: the chosen width survives reloads.
  // Applied via a CSS var so the phone accordion (full-width bands) still wins.
  const SIDE_W_DEFAULT = 260;
  let sideW = $state(loadSideW());
  function loadSideW(): number {
    if (typeof localStorage === 'undefined') return SIDE_W_DEFAULT;
    const v = Number(localStorage.getItem('product.sideW'));
    return Number.isFinite(v) && v >= 200 ? v : SIDE_W_DEFAULT;
  }
  function persistSideW(): void {
    try {
      localStorage.setItem('product.sideW', String(Math.round(sideW)));
    } catch {
      /* storage unavailable — non-fatal */
    }
  }
  function startSideResize(e: PointerEvent): void {
    e.preventDefault();
    const startX = e.clientX;
    const startW = sideW;
    const onMove = (ev: PointerEvent): void => {
      // The sidebar is pinned to the LEFT edge, so dragging RIGHT widens it.
      sideW = Math.max(200, Math.min(480, startW + (ev.clientX - startX)));
    };
    const onUp = (): void => {
      persistSideW();
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  }
  function resetSideW(): void {
    sideW = SIDE_W_DEFAULT;
    persistSideW();
  }
</script>

{#snippet storyRow(s: ProductStory, node?: TreeNode, depth: number = 0)}
  <!-- One tree row: an epic (▾/▸ + 🗂 + count), a top-level story (unchanged
       look) or an indented child (depth 1 = unfiled, 2 = inside a folder). -->
  <div
    class="story-row-wrap"
    class:active={product.selectedId === s.id}
    class:child={depth > 0}
    class:epic={node?.isEpic}
    style={depth ? `--depth:${depth}` : undefined}
    role="presentation"
    oncontextmenu={(e) => storyMenu(e, s, node)}
  >
    {#if node?.isEpic}
      <button class="tree-toggle" onclick={() => toggleEpic(s.id)} aria-label={collapsedEpics[s.id] ? 'Expand epic' : 'Collapse epic'} aria-expanded={!collapsedEpics[s.id]}>
        <Icon name={collapsedEpics[s.id] ? 'chevronRight' : 'chevronDown'} size={12} />
      </button>
    {/if}
    <button
      class="story-row"
      class:active={product.selectedId === s.id}
      onclick={() => selectStory(s)}
      title={s.source_key}
    >
      <span class="story-icon"><Icon name={node?.isEpic ? 'folder' : s.tree_kind === 'doc' ? 'note' : sourceIcon(s.source_kind)} size={13} /></span>
      <span class="story-info">
        <span class="story-title">{s.title}</span>
        <span class="story-meta">
          {#if node?.isEpic}
            <span class="epic-badge">epic · {node.childCount}</span>
          {:else}
            <StatusBadge status={storyStage(s.stage)} variant="text" />
          {/if}
          {#if s.tree_kind === 'doc'}
            <span class="draft-badge doc">DOC</span>
          {:else if s.source_kind === 'draft'}
            <!-- A draft story's stage badge already says "draft". -->
            {#if s.stage !== 'draft'}<span class="draft-badge">Draft</span>{/if}
          {:else}
            <span class="story-key mono">{s.source_key}</span>
          {/if}
        </span>
        {#if parseTags(s.tags).length > 0}
          <span class="story-tags">
            {#each parseTags(s.tags) as tag (tag)}
              <span class="story-tag-chip">{tag}</span>
            {/each}
          </span>
        {/if}
      </span>
    </button>
    <button
      class="row-menu-btn"
      onclick={(e) => storyMenu(e, s, node)}
      aria-label="Story menu"
      title="Move to epic, set folder, mark as epic…"
    >
      <Icon name="grip" size={12} />
    </button>
    <button
      class="delete-btn"
      onclick={() => deleteStory(s)}
      aria-label="Delete story"
      title="Delete story"
    >
      <Icon name="trash" size={12} />
    </button>
  </div>
{/snippet}

<div class="product-shell">
<PageHeader
  title={headerTitle}
  crumbs={headerCrumbs}
  subtitle={product.view === 'stories' && selectedStory ? undefined : 'Analyse Jira / Confluence stories — questions, plans and test cases, published back.'}
>
  {#snippet badge()}
    {#if product.view === 'stories' && selectedStory}
      <!-- Open story: its stage (or tree role), folder and source key sit beside
           the title — the old breadcrumb row is folded into the header. -->
      {#if selectedIsEpic}
        <span class="chip pp-epic-chip"><Icon name="folder" size={10} /> Epic · {product.childrenOf(selectedStory.id).length} children</span>
      {:else if selectedStory.tree_kind === 'doc'}
        <span class="chip"><Icon name="note" size={10} /> Doc</span>
      {:else}
        <StatusBadge status={storyStage(selectedStory.stage)} />
      {/if}
      {#if selectedStory.folder}
        <span class="chip pp-folder-chip" title="Folder inside the epic"><Icon name="folder" size={10} /> {selectedStory.folder}</span>
      {/if}
      {#if selectedStory.source_kind !== 'draft'}
        <span class="pp-key mono">{selectedStory.source_key}</span>
      {/if}
    {/if}
  {/snippet}
  {#snippet tabs()}
    <!-- The ONE Stories|Learnings toggle, at every breakpoint. -->
    <div class="segmented m-view-toggle" role="tablist" aria-label="View">
      <button
        class:active={product.view === 'stories'}
        role="tab"
        aria-selected={product.view === 'stories'}
        onclick={() => (product.view = 'stories')}
      >Stories</button>
      <button
        class:active={product.view === 'learnings'}
        role="tab"
        aria-selected={product.view === 'learnings'}
        onclick={() => (product.view = 'learnings')}
      >Learnings</button>
    </div>
  {/snippet}
  {#snippet actions()}
    {#if product.view === 'stories'}
      {#if selectedStory && selectedIsEpic}
        <button class="btn add-child-btn" onclick={(e) => addChildMenu(e, selectedStory)} title="Add a story or doc under this epic" data-label="Add child…">
          <Icon name="plus" size={12} /> Add child <Icon name="chevronDown" size={10} />
        </button>
      {/if}
      <button
        class="btn"
        onclick={newMenu}
        title="New: a blank draft (Discovery) or an epic that groups stories/docs in folders"
        disabled={draftCreating}
        data-label="New draft or epic…"
      >
        <Icon name="plus" size={12} /> {draftCreating ? 'Creating…' : 'New'} <Icon name="chevronDown" size={10} />
      </button>
      <!-- The ONE import affordance (the empty state owns it while the list is empty). -->
      {#if !noStories}
        <button class="btn primary" onclick={() => (importOpen = true)} title="Import an existing Jira issue / Confluence page">
          <Icon name="plus" size={12} /> Import
        </button>
      {/if}
    {/if}
  {/snippet}
</PageHeader>
<div class="product-page" class:no-stories={noStories} class:m-list-open={mobileSection === 'list'} class:m-content-open={mobileSection === 'content'} style={`--product-side-w:${sideW}px`}>
  <!-- ── Mobile accordion header for the list panel (phone only) ───────── -->
  <button
    class="m-acc-head"
    aria-expanded={mobileSection === 'list'}
    onclick={() => (mobileSection = 'list')}
  >
    <Icon name={mobileSection === 'list' ? 'chevronDown' : 'chevronRight'} size={14} />
    <span class="m-acc-title">{product.view === 'learnings' ? 'Learnings' : 'Stories'}</span>
    {#if product.view === 'stories'}
      <span class="m-acc-count">{product.stories.length}</span>
    {/if}
  </button>

  <!-- ── Left sidebar — always rendered to avoid layout jump ───────────── -->
  <aside class="product-side">
    {#if product.view === 'stories'}
      <!-- Stories sidebar (the header's Stories|Learnings toggle names it). -->
      <!-- Tag filter row (only when tags exist) -->
      {#if allTags.length > 0}
        <div class="tag-filter-row">
          <button
            class="tag-filter-btn"
            class:active={activeTagFilter === null}
            onclick={() => (activeTagFilter = null)}
          >All</button>
          {#each allTags as tag (tag)}
            <button
              class="tag-filter-btn"
              class:active={activeTagFilter === tag}
              onclick={() => (activeTagFilter = activeTagFilter === tag ? null : tag)}
            >{tag}</button>
          {/each}
        </div>
      {/if}

      <div class="story-list">
        {#if product.loadingStories}
          <div class="list-empty">Loading…</div>
        {:else if product.stories.length === 0}
          <div class="list-empty">No stories yet.</div>
        {:else if filteredStories.length === 0}
          <div class="list-empty">No stories match the selected tag.</div>
        {:else}
          {#each tree as node (node.story.id)}
            {@render storyRow(node.story, node)}
            {#if node.isEpic && !collapsedEpics[node.story.id]}
              {#each node.folders as f (f.name)}
                {#if f.name}
                  <button
                    class="folder-head"
                    onclick={() => toggleFolder(node.story.id, f.name)}
                    aria-expanded={!collapsedFolders[`${node.story.id}/${f.name}`]}
                  >
                    <Icon name={collapsedFolders[`${node.story.id}/${f.name}`] ? 'chevronRight' : 'chevronDown'} size={10} />
                    <Icon name="folder" size={11} />
                    <span class="folder-name">{f.name}/</span>
                    <span class="folder-count">{f.children.length}</span>
                  </button>
                {/if}
                {#if !f.name || !collapsedFolders[`${node.story.id}/${f.name}`]}
                  {#each f.children as c (c.id)}
                    {@render storyRow(c, undefined, f.name ? 2 : 1)}
                  {/each}
                {/if}
              {/each}
            {/if}
          {/each}
        {/if}
      </div>

    {:else}
      <!-- Learnings sidebar — filter nav -->
      <div class="learn-nav">
        {#each ([
          { value: 'all', label: 'All' },
          { value: 'pattern', label: 'Patterns to follow' },
          { value: 'avoid', label: 'Cases to avoid' },
        ] as const) as opt (opt.value)}
          <button
            class="learn-filter-btn"
            class:active={learningsFilter === opt.value}
            onclick={() => (learningsFilter = opt.value)}
          >{opt.label}</button>
        {/each}
      </div>
    {/if}

    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="side-resizer"
      role="separator"
      aria-orientation="vertical"
      aria-label="Drag to resize the sidebar (double-click to reset)"
      title="Drag to resize · double-click to reset"
      ondblclick={resetSideW}
      onpointerdown={startSideResize}
    ></div>
  </aside>

  <!-- ── Mobile accordion header for the content panel (phone only) ────── -->
  <button
    class="m-acc-head"
    aria-expanded={mobileSection === 'content'}
    onclick={() => (mobileSection = 'content')}
  >
    <Icon name={mobileSection === 'content' ? 'chevronDown' : 'chevronRight'} size={14} />
    <span class="m-acc-title">
      {#if product.view === 'learnings'}
        Knowledge base
      {:else if product.selectedId && product.detail?.story}
        {product.detail.story.title}
      {:else if product.selectedId}
        Story
      {:else}
        Get started
      {/if}
    </span>
  </button>

  <!-- ── Main area ──────────────────────────────────────────────────────── -->
  <div class="product-main">
    <!-- Header band: per-story navigation, shown only when a story is selected
         in Stories view. ONE row — the 4 workflow-group tabs (icon + label,
         segmented) inline-start, the active group's sub-views as smaller pills
         inline-end (wrapping on a narrow window); a single-sub group (Log)
         shows no pills, since the group click already navigates there. -->
    {#if product.view === 'stories' && product.selectedId}
      <div class="product-header-row2">
        <div class="tab-strip" role="tablist" aria-label="Story tabs">
          {#each visibleGroups as g (g.id)}
            <button
              class="st"
              class:active={activeGroup.id === g.id}
              role="tab"
              aria-selected={activeGroup.id === g.id}
              onclick={() => selectGroup(g)}
            >
              <Icon name={g.icon} size={13} />
              {g.label}
            </button>
          {/each}
        </div>
        {#if activeGroup.subs.length > 1}
          <!-- Keeps the `tab-strip` class too (on top of `sub-tab-strip`): the
               product-mockups E2E locates a sub-view button via the generic
               `.tab-strip .st` selector, matching whichever strip has it. -->
          <div class="tab-strip sub-tab-strip" role="tablist" aria-label="{activeGroup.label} sub-tabs">
            {#each activeGroup.subs as s (s.id)}
              <button
                class="st"
                class:active={product.tab === s.id}
                role="tab"
                aria-selected={product.tab === s.id}
                onclick={() => (product.tab = s.id)}
              >{s.label}</button>
            {/each}
          </div>
        {/if}
      </div>
    {/if}

    <!-- Content -->
    <div class="product-body">
      {#if product.view === 'learnings'}
        <LearningsView filter={learningsFilter} />
      {:else if noStories}
        <EmptyState
          variant="page"
          icon="file"
          title="Analyse a story"
          body="Import a Jira / Confluence issue, then ask questions, draft a plan and test cases, and publish back. Or start a blank draft from New."
          actionLabel="Import story"
          actionIcon="plus"
          onaction={() => (importOpen = true)}
        />
      {:else if !product.selectedId}
        <!-- No story open (a phone, or the list is still loading): a summary
             of the collection with every recent story one click away. -->
        <section class="pp-summary" aria-label="Stories summary">
          <h2 class="pp-summary-title">
            {product.stories.length} {product.stories.length === 1 ? 'story' : 'stories'}
          </h2>
          {#if stageCounts.length}
            <div class="pp-summary-stages">
              {#each stageCounts as [st, n] (st)}
                <StatusBadge status={storyStage(st)} label="{n} {storyStage(st).label.toLowerCase()}" />
              {/each}
            </div>
          {/if}
          <div class="section-title">Recently updated</div>
          <ul class="pp-recent">
            {#each recentStories as r (r.id)}
              <li>
                <button class="pp-recent-row" onclick={() => selectStory(r)}>
                  <Icon name={r.tree_kind === 'doc' ? 'note' : r.tree_kind === 'epic' ? 'folder' : sourceIcon(r.source_kind)} size={13} />
                  <span class="pp-recent-title">{r.title}</span>
                  <StatusBadge status={storyStage(r.stage)} variant="text" />
                  <RelTime iso={r.updated_at} class="pp-recent-time" />
                </button>
              </li>
            {/each}
          </ul>
        </section>
      {:else if product.tab === 'overview'}
        <OverviewTab />
      {:else if product.tab === 'chat'}
        <ChatTab />
      {:else if product.tab === 'analysis'}
        <AnalysisTab />
      {:else if product.tab === 'questions'}
        <QuestionsTab />
      {:else if product.tab === 'notes'}
        <NotesTab />
      {:else if product.tab === 'rewrite'}
        <RewriteTab />
      {:else if product.tab === 'testcases'}
        <TestCasesTab />
      {:else if product.tab === 'plan'}
        <PlanTab />
      {:else if product.tab === 'history'}
        <HistoryTab />
      {:else if product.tab === 'inject'}
        <InjectTab />
      {:else if product.tab === 'discovery'}
        <DiscoveryTab />
      {:else if product.tab === 'refine'}
        <RefineTab />
      {:else if product.tab === 'mockups'}
        <MockupsTab />
      {/if}
    </div>
  </div>
</div>
</div>

{#if importOpen}
  <ImportDialog onclose={() => (importOpen = false)} />
{/if}

<style>
  .product-shell {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .product-page {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  /* Nothing to list → no empty list pane beside the empty state (the phone
     accordion keeps its two sections). */
  @media (min-width: 641px) {
    .product-page.no-stories .product-side {
      display: none;
    }
  }

  /* ── Sidebar ─────────────────────────────────────────────────── */
  .product-side {
    /* Default width; an inline `--product-side-w` (drag-resizable, persisted)
       overrides it. The phone accordion below falls back to full width. */
    width: var(--product-side-w, 260px);
    flex-shrink: 0;
    border-inline-end: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
    position: relative; /* anchors the drag handle on the inline-end edge */
  }
  /* Draggable divider between the sidebar and the main area. Straddles the
     sidebar's inline-end border; a hit-area wider than the border line makes
     it easy to grab. */
  .side-resizer {
    position: absolute;
    inset-block: 0;
    inset-inline-end: -3px;
    width: 6px;
    cursor: col-resize;
    background: transparent;
    z-index: 2;
    touch-action: none;
  }
  .side-resizer:hover {
    background: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  /* Tree-role / draft markers are metadata, not selection: neutral. */
  .draft-badge {
    font-size: var(--fs-xs);
    font-weight: 600;
    padding: 0 6px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .story-list {
    flex: 1;
    overflow-y: auto;
    padding: 6px 8px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-height: 0;
  }
  .list-empty {
    font-size: 11.5px;
    color: var(--text-dim);
    padding: 8px 4px;
    line-height: 1.5;
  }
  /* Wrapper handles hover background + shows delete btn */
  .story-row-wrap {
    display: flex;
    align-items: center;
    border-radius: var(--radius-s);
    transition: background 100ms ease-out;
    position: relative;
  }
  .story-row-wrap:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .story-row-wrap.active {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
  }
  .story-row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    flex: 1;
    min-width: 0;
    padding: 7px 8px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: start;
  }
  /* Selection is the row tint (set on the wrap); text stays --text. */
  .story-row.active .story-title {
    font-weight: 600;
  }
  /* Delete button — hidden until row is hovered or active */
  .delete-btn {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 22px;
    height: 22px;
    margin-inline-end: 6px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: transparent;
    cursor: pointer;
    transition: color 100ms, background 100ms;
    padding: 0;
  }
  .story-row-wrap:hover .delete-btn,
  .story-row-wrap.active .delete-btn {
    color: var(--text-dim);
  }
  .delete-btn:hover {
    background: color-mix(in srgb, var(--danger) 15%, transparent) !important;
    color: var(--danger) !important;
  }
  /* ── Epic tree rows ─────────────────────────────────────────── */
  .row-menu-btn {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 22px;
    height: 22px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: transparent;
    cursor: pointer;
    padding: 0;
    transition: color 100ms, background 100ms;
  }
  .story-row-wrap:hover .row-menu-btn,
  .story-row-wrap.active .row-menu-btn {
    color: var(--text-dim);
  }
  .row-menu-btn:hover {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent-text);
  }
  .tree-toggle {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 18px;
    height: 22px;
    margin-inline-start: 2px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0;
  }
  .story-row-wrap.epic > .story-row {
    padding-inline-start: 2px;
  }
  /* Children indent by depth (1 = unfiled under the epic, 2 = inside a folder). */
  .story-row-wrap.child {
    margin-inline-start: calc(var(--depth, 1) * 14px);
  }
  .story-row-wrap.child .story-row {
    padding-block: 5px;
  }
  .story-row-wrap.child .story-title {
    font-size: 12px;
  }
  .folder-head {
    display: flex;
    align-items: center;
    gap: 4px;
    width: calc(100% - 14px);
    margin-inline-start: 14px;
    padding: 4px 6px 2px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 700;
    letter-spacing: 0.03em;
    cursor: pointer;
    text-align: start;
  }
  .folder-head:hover {
    color: var(--text);
  }
  .folder-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .folder-count {
    font-weight: 600;
    opacity: 0.8;
  }
  .epic-badge {
    font-size: var(--fs-xs);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .story-icon {
    flex-shrink: 0;
    color: var(--text-dim);
    margin-top: 2px;
  }
  .story-row-wrap.active .story-icon {
    color: var(--accent-text);
  }
  .story-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .story-title {
    font-size: var(--fs-m);
    font-weight: 500;
    line-height: 1.3;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    line-clamp: 2;
    overflow: hidden;
  }
  .story-meta {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .story-key {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  /* ── Tag filter row ─────────────────────────────────────────── */
  .tag-filter-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;
    padding: 4px 8px 4px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .tag-filter-btn {
    padding: 1px 7px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    cursor: pointer;
    transition: background 100ms, color 100ms, border-color 100ms;
    white-space: nowrap;
  }
  .tag-filter-btn:hover {
    border-color: var(--accent);
    color: var(--accent-text);
  }
  .tag-filter-btn.active {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    border-color: var(--accent);
    color: var(--accent-text);
    font-weight: 600;
  }

  /* ── Story tag chips (inline in list row) ───────────────────── */
  .story-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 3px;
    margin-top: 1px;
  }
  /* Tags are metadata, not selection: neutral chips. */
  .story-tag-chip {
    font-size: var(--fs-xs);
    padding: 0 6px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
  }


  /* ── Main area ───────────────────────────────────────────────── */
  .product-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  /* The ONE header band: the 4 workflow-group tabs (a segmented pill strip,
     `.tab-strip`) inline-start, the active group's sub-view pills
     (`.sub-tab-strip`) inline-end — wrapping onto their own line on a narrow
     window rather than clipping. `.tab-strip`/`.sub-tab-strip`/`.st` keep
     their old class names: several E2E specs (product-discovery,
     product-mockups, product-sweep) locate tabs by them directly. */
  .product-header-row2 {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px 10px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    padding: 6px 14px;
  }
  .tab-strip {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 3px;
    background: color-mix(in srgb, var(--text-dim) 7%, transparent);
    border-radius: var(--radius-m, 8px);
    overflow-x: auto;
    white-space: nowrap;
    scrollbar-width: none;
    flex-shrink: 0;
  }
  .tab-strip::-webkit-scrollbar {
    display: none;
  }
  .st {
    height: 26px;
    padding: 0 10px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .st:hover {
    color: var(--text);
  }
  /* Selection is the surface lift (components.md §3), not the accent. */
  .tab-strip .st.active {
    background: var(--surface);
    color: var(--text);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.18);
  }
  /* Secondary sub-nav: smaller, dimmer pills, no shared background — a
     sub-level reading subordinate to the segmented group strip beside it.
     Also carries the `.tab-strip` class (see the template comment), so this
     resets everything `.tab-strip` set that doesn't apply here: no container
     background/padding/radius, and pills WRAP instead of horizontal-scrolling. */
  .sub-tab-strip {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 2px 4px;
    margin-inline-start: auto;
    padding: 0 0 0 10px;
    background: none;
    border-radius: 0;
    overflow: visible;
    white-space: normal;
  }
  .sub-tab-strip .st {
    height: 24px;
    padding: 0 9px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    border: 1px solid transparent;
  }
  .sub-tab-strip .st:hover {
    color: var(--text);
    border-color: var(--border);
  }
  .sub-tab-strip .st.active {
    color: var(--text);
    background: var(--surface-2);
    border-color: var(--border);
    font-weight: 600;
  }
  .product-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 16px;
    display: flex;
    flex-direction: column;
  }
  .mono {
    font-family: var(--font-mono, monospace);
  }

  /* ── Learnings sidebar nav ───────────────────────────────────────── */
  .learn-nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 6px 8px;
    flex: 1;
    min-height: 0;
  }
  .learn-filter-btn {
    display: flex;
    align-items: center;
    padding: 7px 10px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
    text-align: start;
    transition: background 100ms, color 100ms;
  }
  .learn-filter-btn:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    color: var(--text);
  }
  .learn-filter-btn.active {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent-text);
    font-weight: 600;
  }

  /* ── Collection summary (no story open) ──────────────────────────── */
  .pp-summary {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-width: 720px;
    width: 100%;
  }
  .pp-summary-title {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .pp-summary-stages {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .pp-recent {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .pp-recent li + li {
    border-top: 1px solid var(--border);
  }
  .pp-recent-row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-height: 32px;
    padding: 4px 10px;
    border: none;
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
  }
  .pp-recent-row:hover {
    background: var(--hover);
  }
  .pp-recent-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pp-recent-row :global(.pp-recent-time) {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .pp-key {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }

  /* ── Mobile accordion headers — hidden on desktop/tablet ──────────────── */
  .m-acc-head {
    display: none;
  }

  @media (max-width: 640px) {
    /* The page becomes a vertical accordion: two tappable section headers,
       each followed by its panel. Exactly one panel is expanded at a time and
       takes all the remaining height to scroll inside; the other collapses to
       just its header. */
    .product-page {
      flex-direction: column;
    }

    /* Slightly bigger touch target for the segmented view toggle. */
    .m-view-toggle > button {
      height: 30px;
      font-size: var(--fs-m);
      padding: 0 12px;
    }

    .m-acc-head {
      display: flex;
      align-items: center;
      gap: 8px;
      width: 100%;
      flex-shrink: 0;
      padding: 12px 14px;
      border: none;
      border-bottom: 1px solid var(--border);
      background: var(--bg-sidebar, var(--surface));
      color: var(--text);
      font-size: 15px;
      font-weight: 600;
      cursor: pointer;
      text-align: start;
      -webkit-tap-highlight-color: transparent;
    }
    .m-acc-head:active {
      background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    }
    .m-acc-title {
      flex: 1;
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .m-acc-count {
      flex-shrink: 0;
      font-size: 12px;
      font-weight: 600;
      color: var(--text-dim);
      background: color-mix(in srgb, var(--text-dim) 14%, transparent);
      border-radius: 999px;
      padding: 1px 9px;
    }

    /* Panels: collapsed by default; the open one gets the remaining height and
       scrolls on its own. */
    .product-side {
      width: 100%;
      border-inline-end: none;
      max-height: none;
      min-height: 0;
      overflow: hidden;
      flex: 0 0 0;
      height: 0;
    }
    /* Full-width accordion band — nothing to drag. */
    .side-resizer {
      display: none;
    }
    .product-main {
      min-height: 0;
      overflow: hidden;
      flex: 0 0 0;
      height: 0;
    }
    .m-list-open .product-side {
      flex: 1 1 auto;
      height: auto;
      overflow-y: auto;
    }
    .m-content-open .product-main {
      flex: 1 1 auto;
      height: auto;
      overflow: hidden; /* inner .product-body owns the scroll */
    }

    /* The list's own internal scroller fills the expanded panel. */
    .m-list-open .story-list,
    .m-list-open .learn-nav {
      flex: 1 1 auto;
    }

    /* ── Bigger, more legible text on phones ───────────────────────────── */
    .list-empty {
      font-size: 14px;
    }
    .story-title {
      font-size: 15px;
    }
    .story-key,
    .story-meta {
      font-size: 12.5px;
    }
    .tag-filter-btn,
    .story-tag-chip {
      font-size: 12px;
    }
    .st {
      height: 38px;
      font-size: 14px;
      padding: 0 13px;
    }
    /* Keep the sub-nav touch-friendly but still a notch smaller than the groups. */
    .sub-tab-strip .st {
      height: 34px;
      font-size: 13px;
      padding: 0 11px;
    }
    .learn-filter-btn {
      font-size: 14.5px;
      padding: 10px 12px;
    }
    .product-body {
      padding: 14px;
    }
    /* Comfortable touch target for the per-row delete button. */
    .delete-btn {
      width: 30px;
      height: 30px;
    }
    .story-row {
      padding: 10px 8px;
    }
  }
</style>
