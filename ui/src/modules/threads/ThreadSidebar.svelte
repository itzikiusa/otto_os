<script lang="ts">
  // The T3-style left rail: one search row (filter · add project · new thread),
  // pinned threads, then projects → their threads (needs-you first, workers
  // nested under their lead), History folded at the bottom, and a quiet footer
  // (Settings · Git · Usage · Sync).
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
  import { toastError } from '../../lib/toastError';
  import type { Session } from '../../lib/api/types';
  import ThreadSidebarRow from './ThreadSidebarRow.svelte';
  import {
    delegatedParentId,
    isThreadSession,
    projectNameFromCwd,
    sessionBranch,
  } from './threadsHelpers';
  import { threadRepos } from './threadRepos.svelte';
  import { FilterPopover, PullRequestsView, SettingsView, UsageView } from './threadLazy.svelte';
  import { threadsChrome } from './threadsChrome.svelte';
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
  let closedFolders = $state<Record<string, boolean>>({});
  let addingProject = $state(false);
  let syncing = $state(false);

  // Lazy-load archived sessions when History is expanded or when searching.
  $effect(() => {
    if ((historyExpanded || search.trim() !== '') && !ws.archivedLoaded && !ws.archivedLoading) {
      void ws.loadArchived();
    }
  });

  const homePath = $derived(ws.scratch?.root_path ?? null);
  // Id index over live + archived sessions: a delegated worker's parent
  // lookup is O(1) instead of a scan of every session per row.
  const allPool = $derived(new Map([...ws.archivedSessions, ...ws.sessions].map((s) => [s.id, s] as const)));
  const allLiveThreads = $derived([
    ...ws.sessions.filter((s) => !s.archived && isThreadSession(s, allPool)),
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
      if (c && !map.has(c)) map.set(c, { key: c, label: projectNameFromCwd(c), cwd: c });
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
    return projectOptions.find((p) => p.key === keys[0])?.label ?? projectNameFromCwd(keys[0]);
  });

  function matchesFilters(s: Session): boolean {
    if (!threads.inProjectFilter(s.cwd)) return false;
    const q = search.trim().toLowerCase();
    if (!q) return true;
    const br = sessionBranch(s, git.repos, git.statusById) ?? '';
    const proj = projectNameFromCwd(s.cwd);
    return (
      s.title.toLowerCase().includes(q) ||
      s.provider.toLowerCase().includes(q) ||
      (s.cwd ?? '').toLowerCase().includes(q) ||
      proj.toLowerCase().includes(q) ||
      br.toLowerCase().includes(q)
    );
  }

  const filteredLive = $derived(allLiveThreads.filter(matchesFilters));

  const pinnedThreads = $derived.by(() => {
    const byId = new Map<string, Session>();
    for (const s of allLiveThreads) byId.set(s.id, s);
    for (const s of ws.archivedSessions) if (isThreadSession(s, allPool)) byId.set(s.id, s);
    return threads.pinnedIds
      .map((id) => byId.get(id))
      .filter((s): s is Session => !!s && matchesFilters(s));
  });

  // Inbox order inside each project: needs-you first, then working, then most
  // recent — delegated workers nested directly under their lead thread.
  function inboxOrder(list: Session[]): { session: Session; child: boolean }[] {
    const score = (s: Session): number => {
      if (ws.needsYou[s.id]) return 0;
      const st = ws.statusMap[s.id] ?? s.status;
      return st === 'working' || st === 'running' ? 1 : 2;
    };
    const sorted = [...list].sort((a, b) => score(a) - score(b) || b.last_active_at.localeCompare(a.last_active_at));
    const present = new Set(sorted.map((s) => s.id));
    const childrenOf = new Map<string, Session[]>();
    const roots: Session[] = [];
    for (const s of sorted) {
      const parentId = delegatedParentId(s);
      if (parentId && parentId !== s.id && present.has(parentId)) {
        childrenOf.set(parentId, [...(childrenOf.get(parentId) ?? []), s]);
      } else roots.push(s);
    }
    const out: { session: Session; child: boolean }[] = [];
    const visit = (s: Session, child: boolean): void => {
      out.push({ session: s, child });
      for (const kid of childrenOf.get(s.id) ?? []) visit(kid, true);
    };
    for (const r of roots) visit(r, false);
    return out;
  }

  // Projects → their unsettled threads. Projects added from "Add project"
  // show even before their first thread.
  const projectGroups = $derived.by(() => {
    const groups = new Map<string, { key: string; label: string; cwd: string; items: Session[] }>();
    const ensure = (cwd: string) => {
      let g = groups.get(cwd);
      if (!g) {
        g = { key: cwd, label: projectNameFromCwd(cwd, ws.current?.name ?? 'Workspace'), cwd, items: [] };
        groups.set(cwd, g);
      }
      return g;
    };
    for (const s of filteredLive) {
      if (threads.isSettled(s.id)) continue;
      ensure((s.cwd ?? ws.current?.root_path ?? '~').replace(/\/+$/, '') || '~').items.push(s);
    }
    if (!search.trim()) {
      for (const p of threads.extraProjects) {
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
      .map((g) => ({ ...g, ordered: inboxOrder(g.items) }));
  });

  // History combines settled threads + archived daemon threads.
  const historyItems = $derived.by(() => {
    const settled = filteredLive.filter((s) => threads.isSettled(s.id));
    const archived = ws.archivedSessions.filter((s) => isThreadSession(s, allPool) && matchesFilters(s));
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

  // The filter popover (lazy; multi-select, pin, per-project options).
  let filterAnchor = $state<HTMLElement | null>(null);

  async function sync(): Promise<void> {
    if (syncing) return;
    syncing = true;
    try {
      threadRepos.reset();
      await ws.refreshSessions();
      if (ws.currentId && ws.currentId !== SCRATCH_WORKSPACE_ID) await runtimeThreads.load(ws.currentId);
      if (ws.currentId) await git.loadRepos(ws.currentId, true);
      for (const id of Object.keys(git.statusById)) void git.refreshStatus(id);
    } catch (e) {
      toastError('Couldn’t refresh threads', e);
    } finally {
      syncing = false;
    }
  }

  function showProjectMenu(e: MouseEvent, grp: { key: string; label: string; cwd: string }): void {
    e.preventDefault();
    const added = threads.extraProjects.includes(grp.key);
    ctxMenu.show(e, [
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

  async function renameThread(s: Session): Promise<void> {
    const next = await confirmer.promptText('Enter a title for this thread.', {
      title: 'Rename thread',
      confirmLabel: 'Rename',
      initial: s.title,
      placeholder: 'Thread title',
    });
    if (!next || next === s.title) return;
    try {
      await ws.renameSession(s.id, next);
    } catch (e) {
      toastError('Couldn’t rename thread', e);
    }
  }

  async function reopenArchived(s: Session, beside = false): Promise<void> {
    threads.unsettle(s.id);
    if (s.archived) {
      try {
        await ws.unarchiveSession(s.id);
      } catch (e) {
        toastError('Couldn’t reopen thread', e);
        return;
      }
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
    const pinned = threads.isPinned(s.id);
    const settled = threads.isSettled(s.id);
    const canSendFromFocused =
      !!threads.focusedSessionId && threads.focusedSessionId !== s.id && !s.archived;
    const items: MenuItem[] = [
      {
        label: pinned ? 'Unpin thread' : 'Pin thread',
        icon: 'star',
        action: () => threads.togglePin(s.id),
      },
      {
        label: 'Open beside in Split',
        icon: 'split',
        action: () => {
          if (s.archived) void reopenArchived(s, true);
          else {
            threads.openBeside(s.id);
            onselect?.();
          }
        },
      },
      ...(canSendFromFocused
        ? [
            {
              label: 'Send message from active thread…',
              icon: 'send',
              action: () => threads.openPeerComposerTo(s.id),
            },
          ]
        : []),
      ...(!s.archived
        ? [
            {
              label: 'Open Room…',
              icon: 'people',
              action: () => onstartroom(s.id),
            },
            {
              label: settled ? 'Move back to Active' : 'Settle to History',
              icon: 'check',
              action: () => threads.toggleSettle(s.id),
            },
          ]
        : [
            {
              label: 'Reopen thread',
              icon: 'refresh',
              action: () => void reopenArchived(s, false),
            },
          ]),
      { separator: true },
      {
        label: 'Rename…',
        icon: 'edit',
        action: () => void renameThread(s),
      },
      ...(!s.archived
        ? [
            {
              label: 'Archive',
              icon: 'archive',
              action: () => {
                void ws.requestArchive(s.id).then((ok) => {
                  if (ok && threads.paneIds.includes(s.id)) threads.closePane(s.id);
                });
              },
            },
          ]
        : []),
      {
        label: 'Delete…',
        icon: 'trash',
        danger: true,
        action: () => {
          void ws.requestDeleteSession(s.id).then(() => {
            if (!ws.getSession(s.id) && threads.paneIds.includes(s.id)) {
              threads.closePane(s.id);
            }
          });
        },
      },
    ];
    ctxMenu.show(e, items);
  }

  function handleRowClick(e: MouseEvent, s: Session): void {
    const split = e.metaKey || e.ctrlKey;
    if (s.archived) {
      void reopenArchived(s, split);
      return;
    }
    threads.unsettle(s.id);
    threads.selectThread(s.id, { split });
    onselect?.();
  }
</script>

<aside class="th-sidebar" aria-label="Threads" data-testid="threads-sidebar">
  <div class="th-top">
    <div class="th-search">
      <Icon name="search" size={14} />
      <input
        type="search"
        dir="auto"
        bind:value={search}
        placeholder="Search"
        aria-label="Search threads, branches, projects"
        data-testid="threads-search"
      />
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

  <div class="th-scroll">
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

    {#if projectGroups.length === 0}
      <p class="th-empty">{search.trim() ? 'No threads match' : 'No threads yet'}</p>
    {:else}
      <div data-testid="threads-active-list">
        {#each projectGroups as grp (grp.key)}
          {@const open = !closedFolders[grp.key]}
          <section class="th-group" aria-label="Project {grp.label}">
            <div class="th-proj reveal-host">
              <button
                type="button"
                class="th-proj-toggle"
                aria-expanded={open}
                title={grp.cwd}
                oncontextmenu={(e) => showProjectMenu(e, grp)}
                onclick={() => (closedFolders = { ...closedFolders, [grp.key]: open })}
              >
                <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
                <span class="ellipsis" dir="auto">{grp.label}</span>
                {#if threads.isProjectPinned(grp.key)}
                  <span class="th-pinned-mark" aria-label="Pinned" title="Pinned project"><Icon name="star" size={12} /></span>
                {/if}
              </button>
              <button
                type="button"
                class="icon-btn reveal-on-hover"
                aria-label="New thread in {grp.label}"
                title="New thread in {grp.label}"
                onclick={() => newThreadIn(grp.cwd)}
              >
                <Icon name="plus" size={12} />
              </button>
            </div>
            {#if open}
              <div class="th-list">
                {#each grp.ordered as item (item.session.id)}
                  <ThreadSidebarRow
                    session={item.session}
                    variant="active"
                    child={item.child}
                    onrowclick={handleRowClick}
                    onreopen={(sess, beside) => void reopenArchived(sess, beside)}
                    onmenu={showThreadMenu}
                    {onselect}
                  />
                {:else}
                  <p class="th-empty-row">No threads yet</p>
                {/each}
              </div>
            {/if}
          </section>
        {/each}
      </div>
    {/if}

    <section class="th-group" aria-label="History and archived threads">
      <button
        type="button"
        class="th-proj-toggle th-history"
        aria-expanded={historyExpanded}
        onclick={() => (historyExpanded = !historyExpanded)}
      >
        <Icon name={historyExpanded ? 'chevronDown' : 'chevronRight'} size={12} />
        <span>History</span>
        {#if historyItems.length > 0}<span class="th-count">{historyItems.length}</span>{/if}
      </button>
      {#if historyExpanded}
        {#if historyItems.length === 0 && !ws.archivedLoading}
          <p class="th-empty-row">Settled and archived threads appear here.</p>
        {:else}
          <div class="th-list" data-testid="threads-history-list">
            {#each historyItems as h (h.session.id)}
              <ThreadSidebarRow
                session={h.session}
                variant="history"
                onrowclick={handleRowClick}
                onreopen={(sess, beside) => void reopenArchived(sess, beside)}
                onmenu={showThreadMenu}
                {onselect}
              />
            {/each}
            {#if ws.archivedHasMore}
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
      {/if}
    </section>
  </div>

  <footer class="th-foot">
    {#if threadsChrome.stageView}
      <button type="button" class="btn small ghost" data-testid="threads-stage-back" onclick={() => (threadsChrome.stageView = null)}>
        <Icon name="chevronLeft" size={12} /> Back
      </button>
    {:else}
      <button
        type="button"
        class="icon-btn"
        aria-label="Settings"
        title="Settings"
        data-testid="threads-open-settings"
        onpointerenter={() => SettingsView.prefetch()}
        onclick={() => (threadsChrome.sheet = 'settings')}
      >
        <Icon name="gear" size={14} />
      </button>
      <button
        type="button"
        class="icon-btn"
        aria-label="Pull requests"
        title="Pull requests"
        data-testid="threads-open-prs"
        onpointerenter={() => PullRequestsView.prefetch()}
        onclick={() => {
          threadsChrome.stageView = 'prs';
          onselect?.();
        }}
      >
        <Icon name="branch" size={14} />
      </button>
      <button
        type="button"
        class="icon-btn"
        aria-label="Usage statistics"
        title="Usage statistics"
        data-testid="threads-open-usage"
        onpointerenter={() => UsageView.prefetch()}
        onclick={() => (threadsChrome.sheet = 'usage')}
      >
        <Icon name="chart" size={14} />
      </button>
      <span class="th-spacer"></span>
      <button
        type="button"
        class="icon-btn"
        aria-label="Refresh threads and git status"
        title="Refresh threads and git status"
        aria-busy={syncing}
        disabled={syncing}
        onclick={() => void sync()}
      >
        <Icon name="refresh" size={14} />
      </button>
    {/if}
  </footer>
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

{#if addingProject}
  <FolderPicker
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
    background: var(--surface);
    border-inline-end: 1px solid var(--border);
  }
  .th-top {
    display: flex;
    align-items: center;
    gap: 2px;
    padding-block: 10px 6px;
    padding-inline: 12px 8px;
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
    font-size: var(--fs-m);
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
    padding: 4px 6px 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .th-group {
    display: flex;
    flex-direction: column;
  }
  .th-label {
    margin: 0;
    padding: 6px 8px 2px;
    font-size: var(--fs-xs);
    font-weight: 500;
    color: var(--text-dim);
  }
  .th-list {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .th-proj {
    display: flex;
    align-items: center;
  }
  .th-proj-toggle {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    block-size: 28px;
    padding-inline: 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    font-weight: 500;
    text-align: start;
    cursor: pointer;
  }
  .th-proj-toggle :global(svg) {
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .th-proj-toggle:hover {
    background: var(--hover);
  }
  .th-pinned-mark {
    display: inline-flex;
    color: var(--text-dim);
  }
  .th-history {
    color: var(--text-dim);
  }
  .th-count {
    margin-inline-start: auto;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .th-empty {
    margin: 0;
    padding: 20px 8px;
    text-align: center;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
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
  .th-foot {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 6px 8px;
    flex-shrink: 0;
  }
  .th-spacer {
    flex: 1;
  }
</style>
