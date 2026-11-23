<script lang="ts">
  // The Threads sidebar as an inbox (T3 Code): one search row (filter · add
  // project · new thread), Pinned when there are pins, then ONE list across
  // projects — needs you, failed, finished unread, then the most recent
  // (threadInbox.ts) — with workers nested under their lead; "Group by
  // project" (filter menu) brings back the per-project folders. Below: the
  // other modules' groups, "Working N" (open) and "History N" (folded), and a
  // quiet footer (Settings · Git · Usage · Sync).
  import Icon from '../../lib/components/Icon.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type { Session } from '../../lib/api/types';
  import { personalAgentName } from '../../lib/personalAgentBadges';
  import { canonicalIdePath, groupThreadsByIdeProject } from './ideProjectModel';
  import { ideProjects, openProjectInIde } from './ideProjects.svelte';
  import type { OrderedIdeProjectGroup } from './ThreadIdeProjectsSection.svelte';
  import ThreadSidebarRow from './ThreadSidebarRow.svelte';
  import ThreadProjectGroups from './ThreadProjectGroups.svelte';
  import ThreadSidebarFooter from './ThreadSidebarFooter.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { openAdjacentThread, openNextNeedsYou, THREAD_KEYS, threadNav } from './threadNav.svelte';
  import { listKeys, sidebarOrder } from './threadSidebarNav';
  import { originGroups, threadOrigin } from './threadOrigins';
  import { inboxOrder } from './threadFamilies';
  import { inboxClass, inboxScore, inboxSections } from './threadInbox';
  import { inboxLive } from './threadInbox.svelte';
  import { threadRowState } from './threadNav.svelte';
  import {
    delegatedParentId,
    isThreadSession,
    projectNameFromCwd,
    sessionBranch,
  } from './threadsHelpers';
  import { FilterPopover, FolderPickerModal, OriginGroupsView } from './threadLazy.svelte';
  import { threadsChrome } from './threadsChrome.svelte';
  import { buildSidebarLegacyThreadMenu, reviveLegacyThread } from './threadMenu';
  import { promptNewScript } from './threadGitMenus';
  import { runtimeThreadMenu } from './runtime/runtimeMenu';
  import { isRuntimeThread, runtimeThreads } from './runtime/runtimeThreads.svelte';
  import { threads } from './threadsStore.svelte';

  interface Props {
    onselect?: () => void;
    onstartroom: (sessionId: string) => void;
  }
  let { onselect, onstartroom }: Props = $props();

  let search = $state('');
  let historyExpanded = $state(false);
  let workingExpanded = $state(true);
  $effect(() => inboxLive.mount());
  let addingProject = $state(false);

  // Lazy-load archived sessions when History is expanded or when searching.
  $effect(() => {
    if ((historyExpanded || search.trim() !== '') && !ws.archivedLoaded && !ws.archivedLoading) {
      void ws.loadArchived();
    }
  });

  const homePath = $derived(ws.scratch?.root_path ?? null);
  /** A personal agent's folder is `…/personal/<agent id>`: its threads are
   *  grouped under the agent's NAME (from the thread meta / title), never
   *  the folder's ULID. cwd → name. */
  const agentFolderNames = $derived.by(() => {
    const m = new Map<string, string>();
    for (const s of runtimeThreads.rows(false).concat(runtimeThreads.rows(true))) {
      const name = personalAgentName(s.meta as Record<string, unknown> | undefined, s.title);
      const c = (s.cwd ?? '').replace(/\/+$/, '');
      if (name && c && !m.has(c)) m.set(c, name);
    }
    return m;
  });
  const folderLabel = (cwd: string, fallback?: string): string => agentFolderNames.get(cwd) ?? projectNameFromCwd(cwd, fallback);
  // Id index over live + archived sessions: a delegated worker's parent
  // lookup is O(1) instead of a scan of every session per row.
  const allPool = $derived(new Map([...ws.archivedSessions, ...ws.otherWsSessions, ...ws.sessions].map((s) => [s.id, s] as const)));
  // Every workspace's threads (the desktop and the phone may have different
  // workspaces open; the same person sees the same threads on both).
  const allLiveThreads = $derived([
    ...[...ws.sessions, ...ws.otherWsSessions].filter((s) => !s.archived && isThreadSession(s, allPool)),
    ...runtimeThreads.rows(),
  ]);

  // Distinct project roots across workspaces, repos and threads for the filter pill.
  const projectOptions = $derived.by(() => {
    const map = new Map<string, { key: string; label: string; cwd: string }>();
    if (ws.current?.root_path) {
      const root = ws.current.root_path.replace(/\/+$/, '');
      map.set(root, { key: root, label: ws.current.name || projectNameFromCwd(root), cwd: root });
    }
    for (const r of git.repos) {
      const rp = r.path.replace(/\/+$/, '');
      if (!map.has(rp)) map.set(rp, { key: rp, label: r.name || projectNameFromCwd(rp), cwd: rp });
    }
    for (const s of allLiveThreads) {
      const c = (s.cwd ?? '').replace(/\/+$/, '');
      if (c && !map.has(c)) map.set(c, { key: c, label: folderLabel(c), cwd: c });
    }
    for (const c of threads.extraProjects) {
      if (!map.has(c)) map.set(c, { key: c, label: projectNameFromCwd(c), cwd: c });
    }
    return [...map.values()];
  });

  const activeFilterLabel = $derived.by(() => {
    const keys = threads.projectFilters;
    if (keys.length === 0) return 'All projects';
    if (keys.length > 1) return `${keys.length} projects`;
    return projectOptions.find((p) => p.key === keys[0])?.label ?? folderLabel(keys[0]);
  });

  function matchesFilters(s: Session): boolean {
    if (!threads.inProjectFilter(s.cwd)) return false;
    const q = search.trim().toLowerCase();
    if (!q) return true;
    const br = sessionBranch(s, git.repos, git.statusById) ?? '';
    const proj = folderLabel((s.cwd ?? '').replace(/\/+$/, ''));
    return (
      s.title.toLowerCase().includes(q) ||
      s.provider.toLowerCase().includes(q) ||
      (s.cwd ?? '').toLowerCase().includes(q) ||
      proj.toLowerCase().includes(q) ||
      br.toLowerCase().includes(q)
    );
  }

  // Agent runs and other modules' helpers fold into their own groups
  // (threadOrigins.ts), out of the projects and History.
  const isMain = (s: Session): boolean => threadOrigin(s) === 'main' && matchesFilters(s);
  const filteredLive = $derived(allLiveThreads.filter(isMain));
  const sideGroups = $derived(originGroups([...allLiveThreads, ...runtimeThreads.rows(true)].filter(matchesFilters)));

  const pinnedThreads = $derived.by(() => {
    const byId = new Map<string, Session>();
    for (const s of allLiveThreads) byId.set(s.id, s);
    for (const s of ws.archivedSessions) if (isThreadSession(s, allPool)) byId.set(s.id, s);
    return threads.pinnedIds
      .map((id) => byId.get(id))
      .filter((s): s is Session => !!s && matchesFilters(s));
  });

  // The inbox (threadInbox.ts): every live thread that is not pinned or in
  // History, split into the main list and "Working"; a family moves together
  // and its children nest under it, foldable.
  /** A thread's subagents start folded under it; `false` = opened here. */
  let foldedFamilies = $state<Record<string, boolean>>({});
  /** The focused thread's parents: their subagents open, so it stays in view. */
  const focusedParents = $derived.by(() => {
    const out = new Set<string>();
    let cur = threads.focusedSessionId ? allPool.get(threads.focusedSessionId) ?? runtimeThreads.rows().find((r) => r.id === threads.focusedSessionId) : undefined;
    for (let p = cur ? delegatedParentId(cur) : null; p && !out.has(p); p = cur ? delegatedParentId(cur) : null) {
      out.add(p);
      cur = allPool.get(p) ?? runtimeThreads.rows().find((r) => r.id === p);
    }
    return out;
  });
  const isFolded = (id: string): boolean => foldedFamilies[id] !== false && !focusedParents.has(id);
  /** The class a row shows (ThreadSidebarRow derives the same). */
  function classOf(s: Session): ReturnType<typeof inboxClass> {
    const st = threadRowState(s);
    const focused = threads.focusedSessionId === s.id && !threads.newDraftMode;
    const unread = !focused && st.key !== 'needs-you' && (ws.unread[s.id] === true || runtimeThreads.unread[s.id] === true);
    return inboxClass(st.key, unread);
  }
  const score = (s: Session): number => inboxScore(classOf(s));

  const unsettledLive = $derived(filteredLive.filter((s) => !threads.isSettled(s.id) && !threads.isPinned(s.id)));
  const sections = $derived(inboxSections(unsettledLive, classOf, delegatedParentId, isFolded));
  /** The threads of the main list, folded rows included (grouped by project). */
  /** Unfolded: what next / previous and "Next that needs you" walk. */
  const allSections = $derived(inboxSections(unsettledLive, classOf, delegatedParentId));
  const mainPool = $derived.by(() => {
    const workingIds = new Set(allSections.working.map((r) => r.session.id));
    return unsettledLive.filter((s) => !workingIds.has(s.id));
  });
  const grouped = $derived(threads.groupByProject);
  const ideDiscoveredProjects = $derived(ideProjects.projectsFor(allLiveThreads));
  const ideGrouping = $derived(groupThreadsByIdeProject(ideDiscoveredProjects, grouped ? mainPool : []));

  const ideProjectGroups = $derived.by((): OrderedIdeProjectGroup[] => {
    const q = search.trim().toLowerCase();
    if (!grouped) return [];
    return ideGrouping.groups
      .filter((grp) => {
        if (
          threads.projectFilters.length > 0 &&
          grp.items.length === 0 &&
          !grp.project.folders.some((f) => threads.inProjectFilter(f))
        ) {
          return false;
        }
        if (q && grp.items.length === 0) {
          return (
            grp.project.name.toLowerCase().includes(q) ||
            grp.project.primaryRoot.toLowerCase().includes(q)
          );
        }
        return true;
      })
      .map((grp) => ({
        ...grp,
        ordered: inboxOrder(grp.items, score, delegatedParentId, isFolded),
      }));
  });

  // Projects → their unsettled threads (excluding threads already grouped under IDE Projects).
  const projectGroups = $derived.by(() => {
    if (!grouped) return [];
    const groups = new Map<string, { key: string; label: string; cwd: string; items: Session[] }>();
    const claimedIdeFolders = new Set(
      ideDiscoveredProjects.flatMap((p) => p.folders.map((f) => canonicalIdePath(f))),
    );
    const ensure = (cwd: string) => {
      let g = groups.get(cwd);
      if (!g) {
        g = { key: cwd, label: folderLabel(cwd, ws.current?.name ?? 'Workspace'), cwd, items: [] };
        groups.set(cwd, g);
      }
      return g;
    };
    for (const s of mainPool) {
      if (ideGrouping.matchedSessionIds.has(s.id)) continue;
      ensure((s.cwd ?? ws.current?.root_path ?? '~').replace(/\/+$/, '') || '~').items.push(s);
    }
    if (!search.trim()) {
      // First open with no threads: the open workspace's project, ready for one.
      const root = ws.current?.root_path?.replace(/\/+$/, '');
      if (root && groups.size === 0 && ideGrouping.groups.length === 0 && threads.inProjectFilter(root)) ensure(root);
      for (const p of threads.extraProjects) {
        if (claimedIdeFolders.has(canonicalIdePath(p))) continue;
        if (threads.inProjectFilter(p)) ensure(p);
      }
    }
    // Pinned projects first (in pin order), the rest keep first-seen order.
    const pinRank = (k: string): number => {
      const i = threads.pinnedProjects.indexOf(k);
      return i < 0 ? Number.MAX_SAFE_INTEGER : i;
    };
    return [...groups.values()]
      .sort((a, b) => pinRank(a.key) - pinRank(b.key))
      .map((g) => ({ ...g, ordered: inboxOrder(g.items, score, delegatedParentId, isFolded) }));
  });

  // History combines settled threads + archived daemon threads.
  const historyItems = $derived.by(() => {
    const settled = filteredLive.filter((s) => threads.isSettled(s.id));
    const archived = [
      ...ws.archivedSessions.filter((s) => isThreadSession(s, allPool)),
      ...runtimeThreads.rows(true),
    ].filter(isMain);
    const seen = new Set<string>();
    const combined: { session: Session; kind: 'settled' | 'archived' }[] = [];
    for (const s of settled) {
      seen.add(s.id);
      combined.push({ session: s, kind: 'settled' });
    }
    for (const s of archived) {
      if (!seen.has(s.id)) combined.push({ session: s, kind: 'archived' });
    }
    return combined;
  });

  // The order next / previous thread and "Next that needs you" walk (threadNav).
  $effect(() => {
    const lists = grouped ? [...ideProjectGroups.map((g) => ({ items: g.ordered })), ...projectGroups.map((g) => ({ items: g.ordered }))] : [{ items: allSections.main }];
    threadNav.order = sidebarOrder(pinnedThreads, [...lists, { items: allSections.working }], sideGroups);
  });
  $effect(() =>
    registry.register('threads-nav', [
      { id: 'threads.next-needs-you', title: 'Next thread that needs you', group: 'Threads', shortcut: THREAD_KEYS.needsYou, keywords: 'waiting approval question attention', run: () => void openNextNeedsYou() },
      { id: 'threads.next', title: 'Next thread', group: 'Threads', shortcut: THREAD_KEYS.next, run: () => void openAdjacentThread(1) },
      { id: 'threads.previous', title: 'Previous thread', group: 'Threads', shortcut: THREAD_KEYS.previous, run: () => void openAdjacentThread(-1) },
    ]),
  );

  // The filter popover (lazy; multi-select, pin, per-project options).
  let filterAnchor = $state<HTMLElement | null>(null);


  function showProjectMenu(e: MouseEvent, grp: { key: string; label: string; cwd: string }): void {
    e.preventDefault();
    const added = threads.extraProjects.includes(grp.key);
    ctxMenu.show(e, [
      ...(grp.cwd && grp.cwd !== '~'
        ? [{ label: 'Open in IDE', icon: 'ide' as const, action: () => openProjectInIde(grp.cwd, null) }]
        : []),
      { label: 'New thread here', icon: 'edit', action: () => newThreadIn(grp.cwd) },
      {
        label: threads.isProjectPinned(grp.key) ? 'Unpin project' : 'Pin project',
        icon: 'star',
        action: () => threads.toggleProjectPin(grp.key),
      },
      {
        label: threads.projectFilters.includes(grp.key) ? 'Remove from filter' : 'Add to filter',
        icon: 'filter',
        action: () => threads.toggleProjectFilter(grp.key),
      },
      { label: 'Add project script…', icon: 'play', action: () => void promptNewScript(grp.cwd) },
      ...(added ? [{ separator: true }, { label: 'Remove from sidebar', icon: 'x', action: () => threads.removeProject(grp.key) }] : []),
    ]);
  }

  function newThreadIn(cwd: string | null): void {
    threads.startNewThread({ workspaceId: ws.currentId, cwd });
    onselect?.();
  }

  async function reopenArchived(s: Session, beside = false): Promise<void> {
    threads.unsettle(s.id);
    // A closed runtime thread opens read-only: reading it starts no agent.
    // Its view's "Revive" bar, a new message or ⋯ › Reopen thread bring it back.
    if (s.archived && !isRuntimeThread(s)) {
      if (!(await reviveLegacyThread(s, false))) return;
    }
    if (beside) threads.openBeside(s.id);
    else threads.selectThread(s.id);
    onselect?.();
  }

  function showThreadMenu(e: MouseEvent, s: Session): void {
    e.preventDefault();
    e.stopPropagation();
    if (isRuntimeThread(s)) {
      ctxMenu.show(e, runtimeThreadMenu(s.id));
      return;
    }
    ctxMenu.show(
      e,
      buildSidebarLegacyThreadMenu(s, {
        onreopen: (sess, beside) => void reopenArchived(sess, beside),
        onstartroom,
        onselect,
      }),
    );
  }

  function handleRowClick(e: MouseEvent, s: Session): void {
    const split = e.metaKey || e.ctrlKey;
    if (s.archived) {
      void reopenArchived(s, split);
      return;
    }
    threads.unsettle(s.id);
    // A terminal thread of another workspace opens there (its pane reads the
    // open workspace's sessions).
    if (!ws.getSession(s.id) && !isRuntimeThread(s)) {
      void ws.select(s.workspace_id).then((ok) => {
        if (ok) threads.selectThread(s.id);
      });
    } else threads.selectThread(s.id, { split });
    onselect?.();
  }
  function toggleFamily(id: string): void {
    foldedFamilies = { ...foldedFamilies, [id]: !isFolded(id) };
  }
  let searchInputEl = $state<HTMLInputElement | null>(null);
  function clearSearch(): void {
    search = '';
    searchInputEl?.focus();
  }
</script>

{#snippet familyRows(rows: readonly { session: Session; child: boolean; kids: string[]; detached: boolean }[])}
  {#each rows as item (item.session.id)}
    <ThreadSidebarRow
      session={item.session}
      variant="active"
      child={item.child}
      family={{ kids: item.kids, detached: item.detached, folded: isFolded(item.session.id), ontoggle: () => toggleFamily(item.session.id) }}
      onrowclick={handleRowClick}
      onreopen={(sess, beside) => void reopenArchived(sess, beside)}
      onmenu={showThreadMenu}
      {onselect}
    />
  {/each}
{/snippet}

<aside class="th-sidebar" aria-label="Threads" data-testid="threads-sidebar">
  <div class="th-top">
    <div class="th-search">
      <Icon name="search" size={14} />
      <input
        bind:this={searchInputEl}
        type="search"
        dir="auto"
        bind:value={search}
        placeholder="Search"
        aria-label="Search threads, branches, projects"
        data-testid="threads-search"
      />
      {#if search}
        <button
          type="button"
          class="icon-btn th-search-clear"
          aria-label="Clear search"
          title="Clear search"
          onclick={clearSearch}
        ><Icon name="x" size={12} /></button>
      {/if}
    </div>
    <button
      type="button"
      class="icon-btn"
      class:th-on={threads.projectFilters.length > 0}
      aria-haspopup="menu"
      aria-label="Filter by project: {activeFilterLabel}"
      title="Filter by project ({activeFilterLabel})"
      data-testid="threads-project-filter"
      aria-expanded={!!filterAnchor}
      onpointerenter={() => FilterPopover.prefetch()}
      onclick={(e) => (filterAnchor = filterAnchor ? null : e.currentTarget)}
    >
      <Icon name="filter" size={14} />
    </button>
    <button
      type="button"
      class="icon-btn"
      aria-label="Add project"
      title="Add project"
      data-testid="threads-add-project"
      onclick={() => (addingProject = true)}
    >
      <Icon name="folder" size={14} />
    </button>
    <button
      type="button"
      class="icon-btn"
      aria-label="New thread"
      title="New thread"
      data-testid="threads-new-btn"
      onclick={() => newThreadIn(null)}
    >
      <Icon name="edit" size={14} />
    </button>
  </div>

  {#if threads.projectFilters.length > 0}
    <div class="th-filter-note">
      <span class="ellipsis">Showing {activeFilterLabel}</span>
      <button type="button" class="btn small ghost" data-testid="threads-filter-clear" onclick={() => (threads.projectFilters = [])}>
        Clear
      </button>
    </div>
  {/if}

  <div class="th-scroll" use:listKeys>
    {#if pinnedThreads.length > 0}
      <section class="th-group" aria-label="Pinned threads">
        <h2 class="th-label">Pinned</h2>
        <div class="th-list" data-testid="threads-pinned-list">
          {#each pinnedThreads as s (s.id)}
            <ThreadSidebarRow
              session={s}
              variant="pinned"
              onrowclick={handleRowClick}
              onreopen={(sess, beside) => void reopenArchived(sess, beside)}
              onmenu={showThreadMenu}
              {onselect}
            />
          {/each}
        </div>
      </section>
    {/if}

    {#if grouped ? ideProjectGroups.length === 0 && projectGroups.length === 0 : sections.main.length === 0}
      {#if search.trim()}
        <div class="th-empty">
          <p>No threads match</p>
          <button type="button" class="btn small ghost" onclick={clearSearch}>Clear search</button>
        </div>
      {:else if sections.working.length === 0 && pinnedThreads.length === 0}
        <div class="th-empty">
          <p>No threads yet</p>
          <button type="button" class="btn small ghost" data-testid="threads-empty-new" onclick={() => newThreadIn(null)}>New thread</button>
        </div>
      {/if}
    {:else if !grouped}
      <div class="th-list" data-testid="threads-active-list">
        {@render familyRows(sections.main)}
      </div>
    {:else}
      <ThreadProjectGroups
        ideGroups={ideProjectGroups}
        groups={projectGroups}
        {foldedFamilies}
        rows={familyRows}
        ontogglefamily={toggleFamily}
        onnewthread={newThreadIn}
        onprojectmenu={showProjectMenu}
        onrowclick={handleRowClick}
        onreopen={(sess, beside) => void reopenArchived(sess, beside)}
        onmenu={showThreadMenu}
        {onselect}
      />
    {/if}

    {#if sideGroups.length > 0}
      <LazyMount
        lazy={OriginGroupsView}
        what="other thread groups"
        variant="compact"
        props={{
          groups: sideGroups,
          onrowclick: handleRowClick,
          onreopen: (sess: Session, beside: boolean) => void reopenArchived(sess, beside),
          onmenu: showThreadMenu,
          onselect,
        }}
      />
    {/if}

    {#if sections.working.length > 0}
      <section class="th-group" aria-label="Working threads">
        <button
          type="button"
          class="th-fold"
          aria-expanded={workingExpanded}
          data-testid="threads-working-toggle"
          onclick={() => (workingExpanded = !workingExpanded)}
        >
          <span>Working</span>
          <span class="th-count">{sections.working.filter((r) => !r.child).length}</span>
          <Icon name={workingExpanded ? 'chevronDown' : 'chevronRight'} size={12} />
        </button>
        <div class="th-list" data-testid="threads-working-list">
          {@render familyRows(workingExpanded ? sections.working : sections.working.filter((r) => r.session.id === threads.focusedSessionId))}
        </div>
      </section>
    {/if}

    <section class="th-group" aria-label="History and archived threads">
      <button
        type="button"
        class="th-fold"
        aria-expanded={historyExpanded}
        data-testid="threads-history-toggle"
        onclick={() => (historyExpanded = !historyExpanded)}
      >
        <span>History</span>
        {#if historyItems.length > 0}<span class="th-count">{historyItems.length}</span>{/if}
        <Icon name={historyExpanded ? 'chevronDown' : 'chevronRight'} size={12} />
      </button>
      {#if historyExpanded && historyItems.length === 0 && !ws.archivedLoading}
        <p class="th-empty-row">Threads you move to History or close appear here.</p>
      {:else}
        <div class="th-list" data-testid="threads-history-list">
          {#each historyExpanded ? historyItems : historyItems.filter((h) => h.session.id === threads.focusedSessionId) as h (h.session.id)}
            <ThreadSidebarRow
              session={h.session}
              variant="history"
              onrowclick={handleRowClick}
              onreopen={(sess, beside) => void reopenArchived(sess, beside)}
              onmenu={showThreadMenu}
              {onselect}
            />
          {/each}
          {#if historyExpanded && ws.archivedHasMore}
            <button
              type="button"
              class="btn small ghost th-more"
              disabled={ws.archivedLoading}
              onclick={() => void ws.loadArchived(true)}
            >
              {ws.archivedLoading ? 'Loading more…' : 'Load older threads'}
            </button>
          {/if}
        </div>
      {/if}
    </section>
  </div>

  <ThreadSidebarFooter {onselect} />
</aside>

{#if filterAnchor && FilterPopover.component}
  <FilterPopover.component
    anchor={filterAnchor}
    projects={projectOptions}
    {homePath}
    onclose={() => (filterAnchor = null)}
    onsettings={(e: MouseEvent, p: { key: string; label: string; cwd: string }) => showProjectMenu(e, p)}
  />
{/if}

{#if addingProject && FolderPickerModal.component}
  <FolderPickerModal.component
    title="Add project"
    start={ws.current?.root_path ?? homePath ?? ''}
    onpick={(picked) => {
      addingProject = false;
      threads.addProject(picked);
      newThreadIn(picked.replace(/\/+$/, ''));
    }}
    onclose={() => (addingProject = false)}
  />
{/if}

<style>
  .th-sidebar {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
    background: var(--bg-sidebar);
    border-inline-end: 1px solid var(--border);
  }
  .th-top {
    display: flex;
    align-items: center;
    gap: 2px;
    padding-block: var(--sp-5) var(--sp-3);
    padding-inline: var(--sp-6) var(--sp-4);
    flex-shrink: 0;
  }
  .th-search {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-dim);
  }
  .th-search input {
    flex: 1;
    min-width: 0;
    block-size: 28px;
    border: 0;
    border-radius: var(--radius-s);
    padding: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-l);
    outline: none;
  }
  .th-search:focus-within {
    color: var(--text);
  }
  .th-search input:focus-visible {
    box-shadow: 0 2px 0 -1px var(--accent-text);
  }
  .th-on {
    color: var(--accent-text);
  }
  .th-filter-note {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding-inline: 14px 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .th-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: var(--sp-2) var(--sp-4) var(--sp-4);
    display: flex;
    flex-direction: column;
    gap: var(--sp-5);
  }
  .th-group {
    display: flex;
    flex-direction: column;
  }
  .th-label {
    margin: 0;
    padding: var(--sp-2) var(--sp-5);
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text-dim);
  }
  /* "Working N" / "History N": one calm line, the count and chevron at the end. */
  .th-fold {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    block-size: 32px;
    padding-inline: var(--sp-5);
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
  }
  .th-fold:hover {
    background: var(--hover);
    color: var(--text);
  }
  .th-fold :global(svg) {
    flex-shrink: 0;
  }
  .th-list {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .th-count {
    margin-inline-start: auto;
    font-size: var(--fs-s);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .th-empty { margin: 0; padding: 20px 8px; text-align: center; font-size: var(--fs-s); color: var(--text-dim); display: flex; flex-direction: column; align-items: center; gap: 6px; }
  .th-empty p { margin: 0; }
  .th-search-clear { flex-shrink: 0; }
  .th-empty-row {
    margin: 0;
    padding-block: 4px;
    padding-inline: 28px 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .th-more {
    margin: 6px 8px;
  }
</style>
