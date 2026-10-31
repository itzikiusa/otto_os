<script lang="ts">
  import Icon from '../../lib/components/Icon.svelte';
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { events } from '../../lib/events.svelte';
  import { rowMenu } from '../../lib/rowMenu';
  import { sessionState } from '../../lib/status';
  import { git } from '../../lib/stores/git.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toastError } from '../../lib/toastError';
  import type { Session } from '../../lib/api/types';
  import ThreadSidebarRow from './ThreadSidebarRow.svelte';
  import {
    compactPath,
    delegatedParentId,
    isThreadSession,
    projectNameFromCwd,
    sessionBranch,
  } from './threadsHelpers';
  import { threads } from './threadsStore.svelte';

  interface Props {
    onselect?: () => void;
    onstartroom: (sessionId: string) => void;
  }
  let { onselect, onstartroom }: Props = $props();

  let search = $state('');
  let pinnedExpanded = $state(true);
  let activeExpanded = $state(true);
  let projectsExpanded = $state(true);
  let historyExpanded = $state(false);
  let closedFolders = $state<Record<string, boolean>>({});

  // Lazy-load archived sessions when History is expanded or when searching.
  $effect(() => {
    if ((historyExpanded || search.trim() !== '') && !ws.archivedLoaded && !ws.archivedLoading) {
      void ws.loadArchived();
    }
  });

  const homePath = $derived(ws.scratch?.root_path ?? null);
  const allLiveThreads = $derived(ws.sessions.filter((s) => !s.archived && isThreadSession(s)));

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
    return [...map.values()];
  });

  const activeFilterLabel = $derived.by(() => {
    if (threads.projectFilter === 'all') return 'All projects';
    return projectOptions.find((p) => p.key === threads.projectFilter)?.label ?? projectNameFromCwd(threads.projectFilter);
  });

  function matchesFilters(s: Session): boolean {
    if (threads.projectFilter !== 'all') {
      const cwd = (s.cwd ?? '').replace(/\/+$/, '');
      if (cwd !== threads.projectFilter && !cwd.startsWith(`${threads.projectFilter}/`)) return false;
    }
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
    for (const s of ws.archivedSessions) if (isThreadSession(s)) byId.set(s.id, s);
    return threads.pinnedIds
      .map((id) => byId.get(id))
      .filter((s): s is Session => !!s && matchesFilters(s));
  });

  // Active inbox: unsettled live threads, ordered with Needs-You first, then working,
  // with delegated subagent workers nested directly underneath their parent thread.
  const activeOrdered = $derived.by(() => {
    const unsettled = filteredLive.filter((s) => !threads.isSettled(s.id));
    const score = (s: Session): number => {
      if (ws.needsYou[s.id]) return 0;
      const st = ws.statusMap[s.id] ?? s.status;
      if (st === 'working' || st === 'running') return 1;
      return 2;
    };
    const sorted = [...unsettled].sort((a, b) => {
      const d = score(a) - score(b);
      if (d !== 0) return d;
      return b.last_active_at.localeCompare(a.last_active_at);
    });
    const presentIds = new Set(sorted.map((s) => s.id));
    const childrenOf = new Map<string, Session[]>();
    const roots: Session[] = [];
    for (const s of sorted) {
      const parentId = delegatedParentId(s);
      if (parentId && parentId !== s.id && presentIds.has(parentId)) {
        const list = childrenOf.get(parentId) ?? [];
        list.push(s);
        childrenOf.set(parentId, list);
      } else {
        roots.push(s);
      }
    }
    const out: { session: Session; child: boolean }[] = [];
    const visit = (s: Session, child: boolean): void => {
      out.push({ session: s, child });
      for (const kid of childrenOf.get(s.id) ?? []) visit(kid, true);
    };
    for (const r of roots) visit(r, false);
    return out;
  });

  // Grouped by project directory/repo.
  const projectGroups = $derived.by(() => {
    const groups = new Map<string, { key: string; label: string; cwd: string; items: Session[] }>();
    for (const s of filteredLive) {
      const cwd = (s.cwd ?? ws.current?.root_path ?? '~').replace(/\/+$/, '') || '~';
      let g = groups.get(cwd);
      if (!g) {
        g = { key: cwd, label: projectNameFromCwd(cwd, ws.current?.name ?? 'Workspace'), cwd, items: [] };
        groups.set(cwd, g);
      }
      g.items.push(s);
    }
    if (groups.size === 0 && ws.current?.root_path) {
      const root = ws.current.root_path.replace(/\/+$/, '');
      groups.set(root, { key: root, label: ws.current.name || projectNameFromCwd(root), cwd: root, items: [] });
    }
    return [...groups.values()];
  });

  // History combines settled threads + archived daemon threads.
  const historyItems = $derived.by(() => {
    const settled = filteredLive.filter((s) => threads.isSettled(s.id));
    const archived = ws.archivedSessions.filter((s) => isThreadSession(s) && matchesFilters(s));
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

  function openProjectFilterMenu(e: MouseEvent): void {
    const items: MenuItem[] = [
      {
        label: 'All projects',
        checked: threads.projectFilter === 'all',
        pinned: true,
        action: () => (threads.projectFilter = 'all'),
      },
      ...(projectOptions.length > 0 ? [{ separator: true, pinned: true }] : []),
      ...projectOptions.map(
        (p): MenuItem => ({
          label: p.label,
          hint: compactPath(p.cwd, homePath),
          checked: threads.projectFilter === p.key,
          action: () => (threads.projectFilter = p.key),
        }),
      ),
    ];
    ctxMenu.show(e, items, {
      filter: projectOptions.length > 6,
      filterPlaceholder: 'Filter projects…',
    });
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

<aside class="th-sidebar" aria-label="Threads inbox and history" data-testid="threads-sidebar">
  <div class="th-filter-bar">
    <div class="input-group th-search-wrap">
      <Icon name="search" size={13} />
      <input
        type="search"
        dir="auto"
        bind:value={search}
        placeholder="Search threads, branches, projects…"
        aria-label="Search threads, branches, projects"
        data-testid="threads-search"
      />
      {#if search}
        <button
          type="button"
          class="icon-btn"
          aria-label="Clear search"
          title="Clear search"
          onclick={() => (search = '')}
        >
          <Icon name="x" size={12} />
        </button>
      {/if}
    </div>
    <button
      type="button"
      class="btn small th-project-btn"
      onclick={openProjectFilterMenu}
      aria-haspopup="menu"
      title="Filter threads by project"
      data-testid="threads-project-filter"
    >
      <Icon name="folder" size={12} />
      <span class="ellipsis">{activeFilterLabel}</span>
      <Icon name="chevronDown" size={12} />
    </button>
  </div>

  <div class="th-sections">
    {#if pinnedThreads.length > 0}
      <section class="th-section" aria-label="Pinned threads">
        <button
          type="button"
          class="th-sec-head"
          aria-expanded={pinnedExpanded}
          onclick={() => (pinnedExpanded = !pinnedExpanded)}
        >
          <Icon name={pinnedExpanded ? 'chevronDown' : 'chevronRight'} size={12} />
          <Icon name="star" size={12} />
          <span class="th-sec-title">Pinned</span>
          <span class="th-sec-count">{pinnedThreads.length}</span>
        </button>
        {#if pinnedExpanded}
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
        {/if}
      </section>
    {/if}

    <section class="th-section" aria-label="Active threads">
      <button
        type="button"
        class="th-sec-head"
        aria-expanded={activeExpanded}
        onclick={() => (activeExpanded = !activeExpanded)}
      >
        <Icon name={activeExpanded ? 'chevronDown' : 'chevronRight'} size={12} />
        <span class="th-sec-title">Active</span>
        <span class="th-sec-count">{activeOrdered.length}</span>
      </button>
      {#if activeExpanded}
        {#if activeOrdered.length === 0}
          <p class="th-empty-hint">
            {search.trim() ? 'No active threads match your search.' : 'Inbox clear — start a thread or reopen one from History.'}
          </p>
        {:else}
          <div class="th-list" data-testid="threads-active-list">
            {#each activeOrdered as item (item.session.id)}
              <ThreadSidebarRow
                session={item.session}
                variant="active"
                child={item.child}
                onrowclick={handleRowClick}
                onreopen={(sess, beside) => void reopenArchived(sess, beside)}
                onmenu={showThreadMenu}
                {onselect}
              />
            {/each}
          </div>
        {/if}
      {/if}
    </section>

    <section class="th-section" aria-label="Threads by project">
      <button
        type="button"
        class="th-sec-head"
        aria-expanded={projectsExpanded}
        onclick={() => (projectsExpanded = !projectsExpanded)}
      >
        <Icon name={projectsExpanded ? 'chevronDown' : 'chevronRight'} size={12} />
        <span class="th-sec-title">Projects</span>
        <span class="th-sec-count">{projectGroups.length}</span>
      </button>
      {#if projectsExpanded}
        <div class="th-projects">
          {#each projectGroups as grp (grp.key)}
            {@const folderOpen = !closedFolders[grp.key]}
            <div class="th-proj-group">
              <div class="th-proj-head reveal-host">
                <button
                  type="button"
                  class="th-proj-toggle"
                  aria-expanded={folderOpen}
                  onclick={() => (closedFolders = { ...closedFolders, [grp.key]: folderOpen })}
                >
                  <Icon name={folderOpen ? 'chevronDown' : 'chevronRight'} size={12} />
                  <Icon name="folder" size={13} />
                  <span class="ellipsis" dir="auto">{grp.label}</span>
                  <span class="th-sec-count">{grp.items.length}</span>
                </button>
                <button
                  type="button"
                  class="icon-btn reveal-on-hover"
                  aria-label="New thread in {grp.label}"
                  title="New thread in {grp.label}"
                  onclick={() => {
                    threads.startNewThread({ workspaceId: ws.currentId, cwd: grp.cwd });
                    onselect?.();
                  }}
                >
                  <Icon name="plus" size={12} />
                </button>
              </div>
              {#if folderOpen && grp.items.length > 0}
                <div class="th-proj-items">
                  {#each grp.items as s (s.id)}
                    {@const st = sessionState(s, ws.statusMap[s.id] ?? s.status, ws.needsYou[s.id] === true, { stale: events.state !== 'connected' })}
                    <button
                      type="button"
                      class="th-proj-item"
                      class:selected={threads.focusedSessionId === s.id && !threads.newDraftMode}
                      onclick={(e) => handleRowClick(e, s)}
                      oncontextmenu={(e) => showThreadMenu(e, s)}
                      use:rowMenu
                    >
                      <StatusDot state={st} />
                      <span class="ellipsis" dir="auto">{s.title}</span>
                    </button>
                  {/each}
                </div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </section>

    <section class="th-section" aria-label="History and archived threads">
      <button
        type="button"
        class="th-sec-head"
        aria-expanded={historyExpanded}
        onclick={() => (historyExpanded = !historyExpanded)}
      >
        <Icon name={historyExpanded ? 'chevronDown' : 'chevronRight'} size={12} />
        <Icon name="clock" size={12} />
        <span class="th-sec-title">History</span>
        {#if historyItems.length > 0}
          <span class="th-sec-count">{historyItems.length}</span>
        {/if}
      </button>
      {#if historyExpanded}
        {#if historyItems.length === 0 && !ws.archivedLoading}
          <p class="th-empty-hint">Settled and archived threads appear here.</p>
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
                class="btn small ghost th-more-btn"
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
</aside>

<style>
  .th-sidebar {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
    background: var(--surface);
    border-inline-end: 1px solid var(--border);
  }
  .th-filter-bar {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px;
    border-block-end: 1px solid var(--border);
    flex-shrink: 0;
  }
  .th-search-wrap {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-inline: 8px;
    min-block-size: 30px;
  }
  .th-search-wrap:focus-within {
    border-color: var(--accent-line-strong);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }
  .th-search-wrap input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    outline: none;
  }
  .th-project-btn {
    justify-content: space-between;
    width: 100%;
  }
  .th-sections {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-block: 6px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .th-section {
    display: flex;
    flex-direction: column;
  }
  .th-sec-head {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 6px 10px;
    border: 0;
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    text-align: start;
    cursor: pointer;
  }
  .th-sec-head:hover {
    color: var(--text);
  }
  .th-sec-title {
    flex: 1;
    min-width: 0;
  }
  .th-sec-count {
    font-size: var(--fs-xs);
    font-weight: 500;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .th-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-inline: 6px;
  }
  .th-projects {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-inline: 6px;
  }
  .th-proj-head {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .th-proj-toggle {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    font-weight: 500;
    text-align: start;
    cursor: pointer;
  }
  .th-proj-toggle:hover {
    background: var(--hover);
  }
  .th-proj-items {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-inline-start: 20px;
  }
  .th-proj-item {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 4px 8px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .th-proj-item:hover {
    background: var(--hover);
    color: var(--text);
  }
  .th-proj-item.selected {
    background: var(--accent-soft);
    color: var(--text);
    font-weight: 500;
  }
  .th-empty-hint {
    margin: 0;
    padding: 8px 12px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    line-height: 1.5;
  }
  .th-more-btn {
    margin: 6px 8px;
  }
</style>
