///<reference types="svelte" />
;
import { rowMenu } from '../../lib/rowMenu';
// Product Story Analysis page — left sidebar (Stories | Learnings toggle +
// story list + import) and a per-story workspace: ONE header band buckets the
// 13 sub-views into 4 workflow GROUPS (Story · Discover · Deliver · Log) —
// group tabs inline-start, the active group's sub-views as pills inline-end —
// with the selected sub-view's content below.
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import { toastError } from '../../lib/toastError';
import { tabKeys } from '../../lib/tabKeys';
import EmptyState from '../../lib/components/EmptyState.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import RelTime from '../../lib/components/RelTime.svelte';
import { storyStage } from '../../lib/status';
import { tick, untrack } from 'svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { recallSelection, rememberSelection } from '../../lib/lastSelection';
import { router } from '../../lib/router.svelte';
import { registry } from '../../lib/commands.svelte';
import { product, buildTree, type TreeNode } from '../../lib/stores/product.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import OverviewTab from './OverviewTab.svelte';
import AnalysisTab from './AnalysisTab.svelte';
import QuestionsTab from './QuestionsTab.svelte';
import NotesTab from './NotesTab.svelte';
import RewriteTab from './RewriteTab.svelte';
import TestCasesTab from './TestCasesTab.svelte';
import PlanTab from './PlanTab.svelte';
import DiscoveryTab from './DiscoveryTab.svelte';
import MockupsTab from './MockupsTab.svelte';
import RefineTab from './RefineTab.svelte';
import ChatTab from './ChatTab.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import LazyMount from '../../lib/components/LazyMount.svelte';
import { lazyComponent } from '../../lib/lazy-component.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import type { ProductStory, TreeKind } from './types';
import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
import PaneDivider from '../../lib/components/PaneDivider.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
function $$render() {
 const storyRow/*Ωignore_positionΩ*/ = (s: ProductStory, node?: TreeNode, depth: number = 0)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  
   {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {             "class":`story-row-wrap`,"style":depth ? `--depth:${depth}` : undefined,"role":`presentation`,"oncontextmenu":(e) => storyMenu(e, s, node),});product.selectedId === s.id;depth > 0;node?.isEpic;
    if(node?.isEpic){
       { svelteHTML.createElement("button", {         "class":`tree-toggle`,"onclick":() => toggleEpic(s.id),"aria-label":collapsedEpics[s.id] ? 'Expand epic' : 'Collapse epic',"title":collapsedEpics[s.id] ? 'Expand epic' : 'Collapse epic',"aria-expanded":!collapsedEpics[s.id],});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":collapsedEpics[s.id] ? 'chevronRight' : 'chevronDown',"size":12,}});}
       }
    }
     { svelteHTML.createElement("button", {          "class":`story-row`,"aria-current":product.selectedId === s.id ? 'true' : undefined,"onclick":() => selectStory(s),"title":s.source_kind === 'draft' ? s.title : `${s.source_key} · ${s.title}`,});product.selectedId === s.id;
       { svelteHTML.createElement("span", { "class":`story-icon`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":node?.isEpic ? 'folder' : s.tree_kind === 'doc' ? 'note' : sourceIcon(s.source_kind),"size":13,}});} }
       { svelteHTML.createElement("span", { "class":`story-info`,});
         { svelteHTML.createElement("span", { "class":`story-title`,});s.title; }
         { svelteHTML.createElement("span", { "class":`story-meta`,});
          if(node?.isEpic){
             { svelteHTML.createElement("span", { "class":`epic-badge`,});  node.childCount; }
          }else{
             { const $$_egdaBsutatS4C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS4C({ target: __sveltets_2_any(), props: {    "status":storyStage(s.stage),"variant":`text`,}});}
          }
          if(s.tree_kind === 'doc'){
             { svelteHTML.createElement("span", { "class":`draft-badge doc`,});  }
          } else if (s.source_kind === 'draft'){
            
            if(s.stage !== 'draft'){ { svelteHTML.createElement("span", { "class":`draft-badge`,});  }}
          }else{
             { svelteHTML.createElement("span", { "class":`story-key mono`,});s.source_key; }
          }
         }
        if(parseTags(s.tags).length > 0){
           { svelteHTML.createElement("span", { "class":`story-tags`,});
               for(let tag of __sveltets_2_ensureArray(parseTags(s.tags))){tag;
               { svelteHTML.createElement("span", { "class":`story-tag-chip`,});tag; }
            }
           }
        }
       }
     }
    
     { svelteHTML.createElement("button", {           "class":`row-menu-btn`,"onclick":(e) => storyMenu(e, s, node),"aria-label":`More actions for ${s.title}`,"title":`More actions`,"aria-haspopup":`menu`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":12,}});}
     }
   }}
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // Split-out tabs and the import dialog: one promise each, with a designed
  // loading / failed-chunk state (LazyMount) instead of a bare `{#await}`.
  const LearningsLazy = lazyComponent(() => import('./LearningsView.svelte'));
  const HistoryLazy = lazyComponent(() => import('./HistoryTab.svelte'));
  const InjectLazy = lazyComponent(() => import('./InjectTab.svelte'));
  const ImportLazy = lazyComponent(() => import('./ImportDialog.svelte'));

  let importOpen = $state(false);
  let draftCreating = $state(false);
  /** A failed story-list load (the store's `error`) — shown inline with Retry,
   *  never as "No stories yet". */
  const storiesError = $derived(product.error);
  /** A failed open of the selected story (the store's `detailError`) — inline
   *  in the content pane with Retry. */
  const detailError = $derived(product.detailError);

  async function loadStories(): Promise<void> {
    try {
      await product.loadStories();
    } catch {
      /* in product.error — rendered inline */
    }
  }
  /** Open a story; a failed detail load is shown inline (Retry), not swallowed. */
  async function openStory(id: string): Promise<void> {
    try {
      await product.select(id);
    } catch {
      /* in product.detailError — rendered inline */
    }
  }

  // ── Epic tree state (design §3.2) ─────────────────────────────────────────
  // Collapsed epics / folders (keys: epic id, `${epicId}/${folder}`). The tree
  // itself is derived from the (tag-filtered) flat list via the store's buildTree.
  let collapsedEpics = $state<Record<string, boolean>>({});
  let collapsedFolders = $state<Record<string, boolean>>({});

  async function createEpic(): Promise<void> {
    if (!(await product.mayLeaveDraft())) return;
    draftCreating = true;
    try {
      await product.createEpic();
      product.tab = 'overview';
      mobileSection = 'content';
    } catch (e) {
      toastError('Couldn’t create the epic', e);
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
    if (!(await product.mayLeaveDraft())) return;
    const title = await confirmer.promptText(`Title of the new ${kind} under “${epic.title}”:`, {
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
      toastError('Couldn’t add the child', e);
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
    items.push({ label: 'Delete…', icon: 'trash', danger: true, action: () => void deleteStory(s) });
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
      toastError('Couldn’t move the story', e);
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
      toastError('Couldn’t set the folder', e);
    }
  }
  async function detach(s: ProductStory): Promise<void> {
    try {
      await product.moveStory(s.id, null, '');
    } catch (e) {
      toastError('Couldn’t detach', e);
    }
  }
  async function mark(s: ProductStory, kind: TreeKind): Promise<void> {
    try {
      await product.setTreeKind(s.id, kind);
    } catch (e) {
      toastError('Couldn’t change the tree role', e);
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
  let mobileSection = $state<'list' | 'content'>(product.selectedId ? 'content' : 'list');

  async function createDraft(): Promise<void> {
    if (!(await product.mayLeaveDraft())) return;
    draftCreating = true;
    try {
      await product.createDraft();
      product.tab = 'overview';
      // On mobile, reveal the new draft's content panel right away.
      mobileSection = 'content';
    } catch (e) {
      toastError('Couldn’t create the draft', e);
    } finally {
      draftCreating = false;
    }
  }

  // Learnings is one full-width view (its own filter control): no list pane,
  // and on a phone its content section is the one that's open.
  $effect(() => {
    if (product.view === 'learnings') untrack(() => (mobileSection = 'content'));
  });

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
  const noStories = $derived(product.view === 'stories' && !product.loadingStories && !storiesError && product.stories.length === 0);

  // Reload stories whenever the workspace changes (mirrors DatabasePage pattern).
  $effect(() => {
    if (ws.currentId) {
      // A workspace switch leaves no artifact open: release the arena's cached
      // blob URLs / editor bases before the new list loads.
      untrack(() => {
        product.teardown();
        void loadStories();
      });
    }
  });

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
      // A routed story (`#/product/<id>`) opens from the route instead.
      if (router.parts[0] === 'product' && router.parts[1]) return;
      const last = recallSelection('product');
      const pick = product.stories.find((x) => x.id === last) ?? tree[0]?.story ?? product.stories[0];
      if (pick) void openStory(pick.id);
    });
  });
  // ── URL-addressable selection: `#/product/<story id>` ─────────────────────
  // The route drives the selection (a deep link / reload / back lands on that
  // story) and the selection writes the route back with `router.replace`, so
  // the URL always names the open story. Opening still goes through
  // `openStory` (the store's draft guard decides); `routePending` holds the
  // write-back while a routed open is in flight so it can't clobber the link.
  let routePending = $state<string | null>(null);
  $effect(() => {
    const [mod, storyId] = router.parts;
    // Workspace initialization clears selection; follow its reset before opening
    // a deep link, including when the workspace arrives after this page mounts.
    const workspaceId = ws.currentId;
    if (!workspaceId || mod !== 'product' || !storyId) return;
    untrack(() => {
      if (storyId === product.selectedId && product.view === 'stories') return;
      routePending = storyId;
      void (async () => {
        try {
          if (product.view !== 'stories') await product.changeView('stories');
          if (product.view !== 'stories') return;
          await openStory(storyId);
          if (product.selectedId !== storyId) return;
          rememberSelection('product', storyId);
          product.tab = 'overview';
          mobileSection = 'content';
        } finally {
          if (routePending === storyId) routePending = null;
        }
      })();
    });
  });
  $effect(() => {
    const id = product.view === 'stories' ? product.selectedId : null;
    if (!ws.currentId || routePending || router.module !== 'product') return;
    const want = id ? `product/${id}` : 'product';
    untrack(() => {
      if (router.parts.join('/') !== want) router.replace(want);
    });
  });

  // ⌘K: the page's verbs.
  $effect(() =>
    registry.register('product', [
      { id: 'product.new-draft', title: 'New draft story', group: 'Product', keywords: 'create story blank discovery', run: () => void createDraft() },
      { id: 'product.new-epic', title: 'New epic', group: 'Product', keywords: 'create group folder stories', run: () => void createEpic() },
      { id: 'product.import', title: 'Import story…', group: 'Product', keywords: 'jira confluence issue page', run: () => (importOpen = true) },
    ]),
  );

  /** The selected story's parent epic (breadcrumb) and epic-ness (Add child ▾). */
  const selectedStory = $derived(product.detail?.story ?? null);
  const selectedParent = $derived(product.parentOf(selectedStory));
  const selectedIsEpic = $derived(
    !!selectedStory && (selectedStory.tree_kind === 'epic' || product.childrenOf(selectedStory.id).length > 0),
  );

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
        { id: 'testcases', label: 'Test cases' },
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
  async function selectGroup(g: Group): Promise<void> {
    if (!g.subs.some((s) => s.id === product.tab)) {
      await product.changeTab(g.subs[0].id);
    }
  }

  let tabTransition = 0;
  async function activateProductTab(event: MouseEvent, activate: () => Promise<void>): Promise<void> {
    const list = (event.currentTarget as HTMLElement).closest<HTMLElement>('[role="tablist"]');
    const seq = ++tabTransition;
    await activate();
    // Dialog teardown queues its own focus restoration. Let that and the new
    // aria-selected state settle before focusing the actual accepted tab.
    await tick();
    await Promise.resolve();
    if (seq !== tabTransition || !list?.isConnected || list.closest('[inert]')) return;
    list.querySelector<HTMLButtonElement>('[role="tab"][aria-selected="true"]')?.focus();
  }

  // ←/→/Home/End only CLICK the target tab: the click handler owns async
  // activation and final focus. Moving focus first would make the discard
  // dialog restore an unaccepted destination.
  const onTabKey = tabKeys({ activate: 'click' });

  function sourceIcon(kind: string): IconName {
    switch (kind) {
      case 'jira': return 'ticket';
      case 'confluence': return 'globe';
      default: return 'file';
    }
  }

  async function selectStory(s: ProductStory): Promise<void> {
    await openStory(s.id);
    if (product.selectedId !== s.id) return;
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
    // Name what else goes (patterns.md §7): the story's Otto-side work is
    // deleted with it; children are re-parented to the top level, not deleted.
    const kids = product.childrenOf(s.id).length;
    const ok = await confirmer.ask(
      `Delete “${s.title}”? This removes it from Otto (the Jira/Confluence item is untouched).\n\n` +
        'Its versions, questions, notes, analyses, test cases, transcripts and attachments are deleted too.' +
        (kids > 0 ? ` Its ${kids} child stor${kids === 1 ? 'y moves' : 'ies move'} to the top level.` : ''),
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
      toastError(`Couldn’t delete “${s.title}”`, e);
      return;
    }
    if (wasOpen && next && product.stories.some((x) => x.id === next.id)) {
      product.tab = 'overview';
      rememberSelection('product', next.id);
      void openStory(next.id);
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
      ? [{ label: selectedParent.title, onclick: () => void openStory(selectedParent.id) }]
      : [],
  );

  // ── Sidebar width (drag-resizable, persisted) ─────────────────────────────
  // Mirrors the DatabasePage sidebar idiom: the chosen width survives reloads.
  // Applied via a CSS var so the phone accordion (full-width bands) still wins.
  let sideW = $state(loadPaneWidth('product.sideW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
;
async () => {



 { svelteHTML.createElement("div", { "class":`product-shell`,});
  { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {      "title":headerTitle,"crumbs":headerCrumbs,"subtitle":product.view === 'stories' && selectedStory ? undefined : 'Jira / Confluence stories — analyze, plan, test, publish back',leading:() => { async ()/*Ωignore_positionΩ*/ => {
    if(viewport.isPhone && product.view === 'stories' && mobileSection === 'content'){
       { svelteHTML.createElement("button", {       "class":`btn ghost`,"onclick":() => (mobileSection = 'list'),"aria-label":`Back to stories`,"title":`Back to stories`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":16,}});}  }
    }
  };return __sveltets_2_any(0)},badge:() => { async ()/*Ωignore_positionΩ*/ => {
    if(product.view === 'stories' && selectedStory){
      
      if(selectedIsEpic){
         { svelteHTML.createElement("span", { "class":`chip pp-epic-chip`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});}   product.childrenOf(selectedStory.id).length;  }
      } else if (selectedStory.tree_kind === 'doc'){
         { svelteHTML.createElement("span", { "class":`chip`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`note`,"size":12,}});}  }
      }else{
         { const $$_egdaBsutatS0C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS0C({ target: __sveltets_2_any(), props: {  "status":storyStage(selectedStory.stage),}});}
      }
      if(selectedStory.folder){
         { svelteHTML.createElement("span", {   "class":`chip pp-folder-chip`,"title":`Folder inside the epic`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});} selectedStory.folder; }
      }
      if(selectedStory.source_kind !== 'draft'){
         { svelteHTML.createElement("span", { "class":`pp-key mono`,});selectedStory.source_key; }
      }
    }
  };return __sveltets_2_any(0)},tabs:() => { async ()/*Ωignore_positionΩ*/ => {
    
     { svelteHTML.createElement("div", {         "class":`segmented m-view-toggle`,"role":`tablist`,"aria-label":`View`,"tabindex":-1,"onkeydown":onTabKey,});
       { svelteHTML.createElement("button", {          "role":`tab`,"aria-selected":product.view === 'stories',"tabindex":product.view === 'stories' ? 0 : -1,"onclick":(event) => void activateProductTab(event, () => product.changeView('stories')),});product.view === 'stories';  }
       { svelteHTML.createElement("button", {          "role":`tab`,"aria-selected":product.view === 'learnings',"tabindex":product.view === 'learnings' ? 0 : -1,"onclick":(event) => void activateProductTab(event, () => product.changeView('learnings')),});product.view === 'learnings';  }
     }
  };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
    if(product.view === 'stories'){
      if(selectedStory && selectedIsEpic){
         { svelteHTML.createElement("button", {       "class":`btn small add-child-btn`,"onclick":(e) => addChildMenu(e, selectedStory),"title":`Add a story or doc under this epic`,...__sveltets_2_empty({"data-label":`Add child…`}),});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}    { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
         }
      }
      
      if(!noStories){
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => (importOpen = true),"title":`Import an existing Jira issue / Confluence page`,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});} 
         }
      }
       { svelteHTML.createElement("button", {            "class":`btn small`,"onclick":newMenu,"title":`New: a blank draft (Discovery) or an epic that groups stories/docs in folders`,"disabled":draftCreating,...__sveltets_2_empty({"data-label":`New draft or epic…`}),});!noStories && !product.selectedId;
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} draftCreating ? 'Creating…' : 'New';  { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
       }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, badge, tabs, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
  
  
  
 PageHeader}
 { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
 { svelteHTML.createElement("div", {       "class":`product-page`,"style":`--product-side-w:${sideW}px`,});noStories;product.view === 'learnings';mobileSection === 'list';mobileSection === 'content';
  
   { svelteHTML.createElement("button", {       "class":`m-acc-head`,"aria-expanded":mobileSection === 'list',"onclick":() => (mobileSection = 'list'),});
     { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":mobileSection === 'list' ? 'chevronDown' : 'chevronRight',"size":14,}});}
     { svelteHTML.createElement("span", { "class":`m-acc-title`,});product.view === 'learnings' ? 'Learnings' : 'Stories'; }
    if(product.view === 'stories'){
       { svelteHTML.createElement("span", { "class":`m-acc-count`,});product.stories.length; }
    }
   }

  
   { svelteHTML.createElement("aside", { "class":`product-side`,});
    if(product.view === 'stories'){
      
      
      if(allTags.length > 0){
         { svelteHTML.createElement("div", { "class":`tag-filter-row`,});
           { svelteHTML.createElement("button", {        "class":`tag-filter-btn`,"aria-pressed":activeTagFilter === null,"onclick":() => (activeTagFilter = null),});activeTagFilter === null;  }
             for(let tag of __sveltets_2_ensureArray(allTags)){tag;
             { svelteHTML.createElement("button", {        "class":`tag-filter-btn`,"aria-pressed":activeTagFilter === tag,"onclick":() => (activeTagFilter = activeTagFilter === tag ? null : tag),});activeTagFilter === tag;tag; }
          }
         }
      }

       { svelteHTML.createElement("div", { "class":`story-list`,});
        if((product.loadingStories || storiesError) && product.stories.length === 0){
           { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {          "what":`stories`,"variant":`compact`,"loading":product.loadingStories,"error":storiesError,"empty":true,"onretry":() => void loadStories(),}});}
        } else if (product.stories.length === 0){
           { const $$_etatSytpmE5C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE5C({ target: __sveltets_2_any(), props: {      "icon":`file`,"title":`No stories yet`,"body":`Import one or start a blank draft from New.`,}});}
        } else if (storiesError){
          
           { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {          "what":`stories`,"variant":`compact`,"loading":product.loadingStories,"error":storiesError,"onretry":() => void loadStories(),}});}
        }
        if(product.stories.length === 0){
          
        } else if (filteredStories.length === 0){
           { const $$_etatSytpmE5C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE5C({ target: __sveltets_2_any(), props: {           "icon":`filter`,"title":`No stories tagged “${activeTagFilter}”`,"actionLabel":`Clear filter`,"actionKind":`secondary`,"onaction":() => (activeTagFilter = null),}});}
        }else{
             for(let node of __sveltets_2_ensureArray(tree)){node.story.id;
            ;__sveltets_2_ensureSnippet(storyRow(node.story, node));
            if(node.isEpic && !collapsedEpics[node.story.id]){
                 for(let f of __sveltets_2_ensureArray(node.folders)){f.name;
                if(f.name){
                   { svelteHTML.createElement("button", {         "class":`folder-head`,...__sveltets_2_empty({"data-epic-id":node.story.id}),"onclick":() => toggleFolder(node.story.id, f.name),"aria-expanded":!collapsedFolders[`${node.story.id}/${f.name}`],});
                     { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":collapsedFolders[`${node.story.id}/${f.name}`] ? 'chevronRight' : 'chevronDown',"size":12,}});}
                     { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});}
                     { svelteHTML.createElement("span", { "class":`folder-name`,});f.name;  }
                     { svelteHTML.createElement("span", { "class":`folder-count`,});f.children.length; }
                   }
                }
                if(!f.name || !collapsedFolders[`${node.story.id}/${f.name}`]){
                     for(let c of __sveltets_2_ensureArray(f.children)){c.id;
                    ;__sveltets_2_ensureSnippet(storyRow(c, undefined, f.name ? 2 : 1));
                  }
                }
              }
            }
          }
        }
       }

    }
   }
  if(product.view === 'stories' && !noStories){
     { const $$_rediviDenaP3C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP3 = new $$_rediviDenaP3C({ target: __sveltets_2_any(), props: {       width:sideW,"storageKey":`product.sideW`,"label":`Resize stories sidebar`,}});/*Ωignore_startΩ*/() => sideW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP3.$$bindings = 'width';}
  }

  
   { svelteHTML.createElement("button", {       "class":`m-acc-head`,"aria-expanded":mobileSection === 'content',"onclick":() => (mobileSection = 'content'),});
     { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":mobileSection === 'content' ? 'chevronDown' : 'chevronRight',"size":14,}});}
     { svelteHTML.createElement("span", { "class":`m-acc-title`,});
      if(product.view === 'learnings'){ } else if (product.selectedId && product.detail?.story){
        product.detail.story.title;
      } else if (product.selectedId){ }else{ }
     }
   }

  
   { svelteHTML.createElement("div", { "class":`product-main`,});
    
    if(product.view === 'stories' && product.selectedId){
       { svelteHTML.createElement("div", { "class":`product-header-row2`,});
         { svelteHTML.createElement("div", {         "class":`tab-strip`,"role":`tablist`,"aria-label":`Story tabs`,"tabindex":-1,"onkeydown":onTabKey,});
             for(let g of __sveltets_2_ensureArray(visibleGroups)){g.id;
             { svelteHTML.createElement("button", {            "class":`st`,"role":`tab`,"aria-selected":activeGroup.id === g.id,"tabindex":activeGroup.id === g.id ? 0 : -1,"onclick":(event) => void activateProductTab(event, () => selectGroup(g)),});activeGroup.id === g.id;
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":g.icon,"size":13,}});}
              g.label;
             }
          }
         }
        if(activeGroup.subs.length > 1){
          
           { svelteHTML.createElement("div", {         "class":`tab-strip sub-tab-strip`,"role":`tablist`,"aria-label":`${activeGroup.label} sub-tabs`,"tabindex":-1,"onkeydown":onTabKey,});
               for(let s of __sveltets_2_ensureArray(activeGroup.subs)){s.id;
               { svelteHTML.createElement("button", {            "class":`st`,"role":`tab`,"aria-selected":product.tab === s.id,"tabindex":product.tab === s.id ? 0 : -1,"onclick":(event) => void activateProductTab(event, () => product.changeTab(s.id)),});product.tab === s.id;s.label; }
            }
           }
        }
       }
    }

    
     { svelteHTML.createElement("div", { "class":`product-body`,});
      if(product.view === 'learnings'){
         { const $$_tnuoMyzaL5C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL5C({ target: __sveltets_2_any(), props: {      "lazy":LearningsLazy,"what":`the learnings`,"variant":`panel`,}});}
      } else if (storiesError && product.stories.length === 0){
         { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {          "what":`stories`,"variant":`page`,"loading":product.loadingStories,"error":storiesError,"empty":true,"onretry":() => void loadStories(),}});}
      } else if (noStories){
         { const $$_etatSytpmE5C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE5C({ target: __sveltets_2_any(), props: {               "variant":`page`,"icon":`file`,"title":`No stories yet`,"body":`Import a Jira / Confluence issue, then ask questions, draft a plan and test cases, and publish back. Or start a blank draft from New.`,"actionLabel":`Import story`,"actionIcon":`plus`,"onaction":() => (importOpen = true),}});}
      } else if (product.selectedId && detailError && product.detail?.story.id !== product.selectedId){
         { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {          "what":`this story`,"variant":`page`,"loading":product.loadingDetail,"error":detailError,"empty":true,"onretry":() => void openStory(product.selectedId!),}});}
      } else if (!product.selectedId){
        
         { svelteHTML.createElement("section", {   "class":`pp-summary`,"aria-label":`Stories summary`,});
           { svelteHTML.createElement("h2", { "class":`pp-summary-title`,});
            product.stories.length; product.stories.length === 1 ? 'story' : 'stories';
           }
          if(stageCounts.length){
             { svelteHTML.createElement("div", { "class":`pp-summary-stages`,});
                 for(let [st, n] of __sveltets_2_ensureArray(stageCounts)){st;
                 { const $$_egdaBsutatS7C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS7C({ target: __sveltets_2_any(), props: {    "status":storyStage(st),"label":`${n} ${storyStage(st).label.toLowerCase()}`,}});}
              }
             }
          }
           { svelteHTML.createElement("div", { "class":`section-title`,});  }
           { svelteHTML.createElement("ul", { "class":`pp-recent`,});
               for(let r of __sveltets_2_ensureArray(recentStories)){r.id;
               { svelteHTML.createElement("li", {});
                 { svelteHTML.createElement("button", {   "class":`pp-recent-row`,"onclick":() => selectStory(r),});
                   { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":r.tree_kind === 'doc' ? 'note' : r.tree_kind === 'epic' ? 'folder' : sourceIcon(r.source_kind),"size":13,}});}
                   { svelteHTML.createElement("span", { "class":`pp-recent-title`,});r.title; }
                   { const $$_egdaBsutatS9C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS9C({ target: __sveltets_2_any(), props: {    "status":storyStage(r.stage),"variant":`text`,}});}
                   { const $$_emiTleR9C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR9C({ target: __sveltets_2_any(), props: {    "iso":r.updated_at,"class":`pp-recent-time`,}});}
                 }
               }
            }
           }
         }
      } else if (product.tab === 'overview'){
        
        product.selectedId; {
           { const $$_baTweivrevO5C = __sveltets_2_ensureComponent(OverviewTab); new $$_baTweivrevO5C({ target: __sveltets_2_any(), props: {}});}
        }
      } else if (product.tab === 'chat'){
         { const $$_baTtahC5C = __sveltets_2_ensureComponent(ChatTab); new $$_baTtahC5C({ target: __sveltets_2_any(), props: {}});}
      } else if (product.tab === 'analysis'){
         { const $$_baTsisylanA5C = __sveltets_2_ensureComponent(AnalysisTab); new $$_baTsisylanA5C({ target: __sveltets_2_any(), props: {}});}
      } else if (product.tab === 'questions'){
         { const $$_baTsnoitseuQ5C = __sveltets_2_ensureComponent(QuestionsTab); new $$_baTsnoitseuQ5C({ target: __sveltets_2_any(), props: {}});}
      } else if (product.tab === 'notes'){
         { const $$_baTsetoN5C = __sveltets_2_ensureComponent(NotesTab); new $$_baTsetoN5C({ target: __sveltets_2_any(), props: {}});}
      } else if (product.tab === 'rewrite'){
        product.selectedId; {
           { const $$_baTetirweR5C = __sveltets_2_ensureComponent(RewriteTab); new $$_baTetirweR5C({ target: __sveltets_2_any(), props: {}});}
        }
      } else if (product.tab === 'testcases'){
         { const $$_baTsesaCtseT5C = __sveltets_2_ensureComponent(TestCasesTab); new $$_baTsesaCtseT5C({ target: __sveltets_2_any(), props: {}});}
      } else if (product.tab === 'plan'){
         { const $$_baTnalP5C = __sveltets_2_ensureComponent(PlanTab); new $$_baTnalP5C({ target: __sveltets_2_any(), props: {}});}
      } else if (product.tab === 'history'){
         { const $$_tnuoMyzaL5C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL5C({ target: __sveltets_2_any(), props: {      "lazy":HistoryLazy,"what":`the story history`,"variant":`panel`,}});}
      } else if (product.tab === 'inject'){
         { const $$_tnuoMyzaL5C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL5C({ target: __sveltets_2_any(), props: {      "lazy":InjectLazy,"what":`the inject tab`,"variant":`panel`,}});}
      } else if (product.tab === 'discovery'){
         { const $$_baTyrevocsiD5C = __sveltets_2_ensureComponent(DiscoveryTab); new $$_baTyrevocsiD5C({ target: __sveltets_2_any(), props: {}});}
      } else if (product.tab === 'refine'){
         { const $$_baTenifeR5C = __sveltets_2_ensureComponent(RefineTab); new $$_baTenifeR5C({ target: __sveltets_2_any(), props: {}});}
      } else if (product.tab === 'mockups'){
         { const $$_baTspukcoM5C = __sveltets_2_ensureComponent(MockupsTab); new $$_baTspukcoM5C({ target: __sveltets_2_any(), props: {}});}
      }
     }
   }
 }
 PageBody}
 }

if(importOpen){
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {          "lazy":ImportLazy,"what":`the import dialog`,"quiet":true,"onerror":() => (importOpen = false),"props":{ onclose: () => (importOpen = false), onimported: () => (mobileSection = 'content') },}});}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ProductPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ProductPage__SvelteComponent_ = ReturnType<typeof ProductPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default ProductPage__SvelteComponent_;