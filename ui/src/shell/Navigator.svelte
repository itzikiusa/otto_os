<script lang="ts">
  // Expanded 240px navigator: modules (Agents with nested session list),
  // workspaces section, user/settings at the bottom.
  import Icon from '../lib/components/Icon.svelte';
  import StatusDot from '../lib/components/StatusDot.svelte';
  import { router } from '../lib/router.svelte';
  import { ui } from '../lib/stores/ui.svelte';
  import { ws } from '../lib/stores/workspace.svelte';
  import { auth } from '../lib/stores/auth.svelte';
  import { plugins } from '../lib/stores/plugins.svelte';
  import { activity } from '../lib/stores/activity.svelte';
  import { proof } from '../lib/stores/proof.svelte';
  import ProofStatusChip from '../lib/components/ProofStatusChip.svelte';
  import { ctxMenu } from '../lib/contextmenu.svelte';
  import { availableModules, resolveOrder, visibleOrder, type SidebarModule } from '../lib/sidebar';
  import type { Session, SessionStatus } from '../lib/api/types';

  // Load the per-session task roll-up for the current workspace (sidebar chips);
  // it then stays fresh from the events WS (tasks_updated / trail_appended).
  $effect(() => {
    const w = ws.currentId;
    if (w) void activity.loadSummary(w);
  });

  // Load the per-work-item proof roll-up so each session row can show an inline
  // proof chip; kept fresh from the events WS (proof_pack_updated).
  $effect(() => {
    const w = ws.currentId;
    if (w) void proof.loadSummary(w);
  });

  // A session is "suspended / resumable" — parked to save memory, but its
  // provider session is intact so opening it auto-resumes (`--resume`).
  // True when it's `reconnectable`, or an exited agent session that still
  // carries a provider_session_id. A plain exited shell is genuinely "ended".
  function isResumable(s: Session, status: SessionStatus): boolean {
    if (status === 'reconnectable') return true;
    return status === 'exited' && s.kind === 'agent' && s.provider_session_id != null;
  }
  const SUSPENDED_TIP = 'Suspended to save memory — opens instantly';

  let agentsOpen = $state(true);
  // Channel groups (ticket/chat sessions) start collapsed — at ticketing volume
  // they can number in the dozens, so the header + count is shown by default and
  // the user expands on demand.
  let telegramOpen = $state(false);
  let slackOpen = $state(false);
  let connectionsOpen = $state(true);
  let archivedOpen = $state(false);
  let renamingId: string | null = $state(null);
  let draft = $state('');

  // When a channel group is expanded, show only the most recent N sessions with
  // a "show more" expander, so a busy day's worth of tickets doesn't flood the
  // sidebar. Bypassed while searching (the user is actively filtering).
  const CHANNEL_CAP = 20;
  let telegramShowAll = $state(false);
  let slackShowAll = $state(false);

  // Session search: filters every group by title (case-insensitive), plus an
  // optional "Needs you" filter that narrows to sessions blocked on the operator.
  let sessionQuery = $state('');
  const q = $derived(sessionQuery.trim().toLowerCase());
  const matches = (s: Session): boolean => {
    if (ws.needsYouFilter && ws.needsYou[s.id] !== true) return false;
    return q === '' || s.title.toLowerCase().includes(q);
  };
  const fAgents = $derived(ws.plainAgentSessions.filter(matches));
  const fTelegram = $derived(ws.telegramSessions.filter(matches));
  const fSlack = $derived(ws.slackSessions.filter(matches));
  // Capped views (full list when searching or "show all" toggled).
  const visTelegram = $derived(q || telegramShowAll ? fTelegram : fTelegram.slice(0, CHANNEL_CAP));
  const visSlack = $derived(q || slackShowAll ? fSlack : fSlack.slice(0, CHANNEL_CAP));

  // Drag-to-resize the navigator from its right edge (widens the session area).
  let resizing = $state(false);
  function startResize(e: MouseEvent): void {
    e.preventDefault();
    resizing = true;
    const startX = e.clientX;
    const startW = ui.railWidth;
    const onMove = (ev: MouseEvent) => ui.setRailWidth(startW + (ev.clientX - startX));
    const onUp = () => {
      resizing = false;
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
  }

  function openSession(id: string): void {
    ws.navigateToSession(id);
  }

  function startRename(id: string, current: string): void {
    if (ws.myRole === 'viewer') return;
    renamingId = id;
    draft = current;
  }

  async function commitRename(): Promise<void> {
    const id = renamingId;
    renamingId = null;
    if (!id) return;
    const next = draft.trim();
    if (next) await ws.renameSession(id, next);
  }

  // ── Module list (shared registry → RBAC + plugins → user order/visibility) ──
  // `resolved` is the full ordered list (incl. hidden); `visible` drops hidden
  // modules; `navList` is what we actually render — everything while editing (so
  // hidden rows can be toggled back on), only the visible ones otherwise.
  const pluginEntries = $derived(
    plugins.list
      .filter((p) => auth.canPlugin(p.slug, 'view'))
      .map((p): SidebarModule => ({ id: `plugin/${p.slug}`, icon: p.icon, label: p.name })),
  );
  const resolved = $derived(
    resolveOrder(availableModules((f) => auth.can(f, 'view'), pluginEntries), ui.sidebarOrder),
  );
  const visible = $derived(visibleOrder(resolved, ui.sidebarHidden));
  const navList = $derived(ui.sidebarEditMode ? resolved : visible);

  // Active when the route matches the module id. Plugin entries carry a
  // `plugin/<slug>` id while router.module is just `plugin`, so compare the slug.
  // Agents also owns the default ('') route.
  function isActive(id: string): boolean {
    if (id.startsWith('plugin/')) {
      return router.module === 'plugin' && `plugin/${router.parts[1] ?? ''}` === id;
    }
    if (id === 'agents') return router.module === 'agents' || router.module === '';
    return router.module === id;
  }

  function isHidden(id: string): boolean {
    return ui.sidebarHidden.includes(id);
  }

  // ── Edit mode: drag-to-reorder (HTML5 DnD, like TabBar) + up/down buttons
  // (touch-reliable + keyboard-accessible). Both persist the full module order.
  let dragId = $state<string | null>(null);
  let dragOverId = $state<string | null>(null);

  function onDragStart(e: DragEvent, id: string): void {
    dragId = id;
    e.dataTransfer?.setData('text/plain', id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }
  function onDragOver(e: DragEvent, id: string): void {
    if (!dragId || id === dragId) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dragOverId = id;
  }
  function onDragLeave(id: string): void {
    if (dragOverId === id) dragOverId = null;
  }
  function onDrop(e: DragEvent, id: string): void {
    e.preventDefault();
    if (dragId) ui.reorderSidebar(resolved.map((m) => m.id), dragId, id);
    dragId = null;
    dragOverId = null;
  }
  function onDragEnd(): void {
    dragId = null;
    dragOverId = null;
  }
  function move(id: string, delta: number): void {
    ui.moveSidebar(
      resolved.map((m) => m.id),
      id,
      delta,
    );
  }
</script>

<nav class="navigator sidebar-material" class:resizing aria-label="Navigator" style="width:{ui.railWidth}px">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="rail-resize"
    onmousedown={startResize}
    ondblclick={() => ui.setRailWidth(240)}
    title="Drag to resize · double-click to reset"
  ></div>
  <div class="nav-head" class:tauri-pad={false}>
    <img class="nav-logo" src="/otto-mark-64.png" alt="" width="20" height="20" />
    <span class="nav-title">Otto</span>
    <button
      class="icon-btn nav-back"
      onclick={() => router.back()}
      disabled={!router.canBack}
      title="Back (⌘⇧←)"
      aria-label="Back"
    >
      <Icon name="chevronRight" size={14} />
    </button>
    <button
      class="icon-btn"
      onclick={() => router.forward()}
      disabled={!router.canForward}
      title="Forward (⌘⇧→)"
      aria-label="Forward"
    >
      <Icon name="chevronRight" size={14} />
    </button>
    <button
      class="icon-btn"
      onclick={() => ui.toggleRail()}
      title="Collapse sidebar (⌘1)"
      aria-label="Collapse sidebar"
    >
      <Icon name="sidebar" size={14} />
    </button>
  </div>

  <div class="nav-scroll">
    <!-- Global session search: filters every group below (Agents / Telegram /
         Slack). Hidden while editing the sidebar (no session lists shown then). -->
    {#if !ui.sidebarEditMode}
      <div class="nav-search">
        <Icon name="search" size={12} />
        <input
          class="nav-search-input"
          placeholder="Search all sessions…"
          bind:value={sessionQuery}
        />
        {#if sessionQuery}
          <button class="search-clear" onclick={() => (sessionQuery = '')} aria-label="Clear search">×</button>
        {/if}
      </div>

      <!-- "Needs you" filter: narrows every group to sessions blocked on input.
           Only shown once at least one session is flagged (or while already on). -->
      {#if ws.needsYouCount > 0 || ws.needsYouFilter}
        <button
          class="needs-you-filter"
          class:active={ws.needsYouFilter}
          onclick={() => (ws.needsYouFilter = !ws.needsYouFilter)}
          title="Show only sessions waiting on you"
        >
          <Icon name="bell" size={11} />
          <span class="grow">Needs you</span>
          {#if ws.needsYouCount > 0}
            <span class="needs-you-count">{ws.needsYouCount}</span>
          {/if}
        </button>
      {/if}
    {/if}

    <div class="nav-section" data-testid="sidebar-modules">
      {#if ui.sidebarEditMode}
        <p class="edit-hint">Drag to reorder · tap the eye to hide. Hidden items stay listed here while you edit.</p>
      {/if}

      <!-- Modules render in the user's saved order (shared registry). While
           editing, every resolved module shows as a compact draggable row
           (incl. hidden ones, so they can be toggled back on); otherwise the
           special Agents/Connections blocks (with their nested session lists)
           and plain rows render normally. -->
      {#each navList as m, i (m.id)}
        {#if ui.sidebarEditMode}
          {@render editRow(m, i)}
        {:else if m.id === 'agents'}
          {@render agentsBlock()}
        {:else if m.id === 'connections'}
          {@render connectionsBlock()}
        {:else}
          {@render simpleRow(m)}
        {/if}
      {/each}

      {#if !ui.sidebarEditMode && ws.archivedSessions.length > 0}
        <button class="nav-item subtle" onclick={() => (archivedOpen = !archivedOpen)}>
          <Icon name="archive" size={14} />
          <span class="grow">Archived</span>
          <span class="count-chip">{ws.archivedSessions.length}</span>
          <Icon name={archivedOpen ? 'chevronDown' : 'chevronRight'} size={11} />
        </button>
        {#if archivedOpen}
          <div class="nested">
            {#each ws.archivedSessions as s (s.id)}
              <div class="nested-row">
                <!-- svelte-ignore a11y_no_static_element_interactions -->
                <div
                  class="nav-item nested-item archived"
                  title={s.title}
                  oncontextmenu={(e) => ctxMenu.show(e, [
                    ...(ws.myRole !== 'viewer' ? [
                      { label: 'Unarchive', icon: 'refresh', action: () => ws.unarchiveSession(s.id) },
                      { label: 'Delete', icon: 'trash', danger: true as const, action: () => ws.killSession(s.id) },
                    ] : []),
                    { separator: true },
                    { label: 'New session…', icon: 'plus', action: () => (ui.newSessionOpen = true) },
                  ])}
                >
                  <StatusDot status="exited" />
                  <span class="grow ellipsis">{s.title}</span>
                  <span class="provider">{s.provider}</span>
                </div>
                {#if ws.myRole !== 'viewer'}
                  <button class="row-action" title="Restore" aria-label="Restore session" onclick={() => ws.unarchiveSession(s.id)}>
                    <Icon name="refresh" size={11} />
                  </button>
                  <button class="row-action danger" title="Delete" aria-label="Delete session" onclick={() => ws.killSession(s.id)}>
                    <Icon name="trash" size={11} />
                  </button>
                {/if}
              </div>
            {/each}
          </div>
        {/if}
      {/if}
    </div>

    <div class="nav-section">
      <div class="nav-label-row">
        <span class="nav-label">Workspaces</span>
        <button
          class="icon-btn add-ws"
          onclick={() => (ui.newWorkspaceOpen = true)}
          title="Add workspace"
          aria-label="Add workspace"
        >
          <Icon name="plus" size={14} />
        </button>
      </div>
      {#each ws.workspaces as w (w.id)}
        <button
          class="nav-item"
          class:active-ws={ws.currentId === w.id}
          onclick={() => ws.select(w.id)}
          oncontextmenu={(e) => ctxMenu.show(e, [
            { label: 'Switch to this workspace', icon: 'check', action: () => ws.select(w.id) },
            { separator: true },
            { label: 'Add workspace…', icon: 'plus', action: () => (ui.newWorkspaceOpen = true) },
            { label: 'Workspace settings', icon: 'gear', action: () => router.go('settings/appearance') },
          ])}
          title={w.root_path}
        >
          <Icon name="folder" size={14} />
          <span class="grow ellipsis">{w.name}</span>
          {#if ws.currentId === w.id}<Icon name="check" size={12} />{/if}
        </button>
      {/each}
    </div>
  </div>

  <div class="nav-foot">
    {#if ui.sidebarEditMode}
      <button class="nav-item subtle" onclick={() => ui.resetSidebar()} data-testid="sidebar-reset">
        <Icon name="refresh" size={14} />
        <span class="grow">Reset to default</span>
      </button>
    {/if}
    <button
      class="nav-item subtle"
      class:active={ui.sidebarEditMode}
      onclick={() => ui.toggleSidebarEdit()}
      title="Show, hide and reorder sidebar items"
      data-testid="sidebar-edit-toggle"
    >
      <Icon name={ui.sidebarEditMode ? 'check' : 'edit'} size={14} />
      <span class="grow">{ui.sidebarEditMode ? 'Done' : 'Customize sidebar'}</span>
    </button>
    <button
      class="nav-item"
      class:active={router.module === 'walkthroughs'}
      onclick={() => router.go('walkthroughs')}
    >
      <Icon name="info" size={14} />
      <span class="grow">Walkthroughs</span>
    </button>
    <button
      class="nav-item"
      class:active={router.module === 'settings'}
      onclick={() => router.go('settings/appearance')}
    >
      <Icon name="gear" size={14} />
      <span class="grow">Settings</span>
    </button>
    <div class="nav-user">
      <span class="avatar">{(auth.me?.display_name ?? '?').slice(0, 1).toUpperCase()}</span>
      <div class="grow">
        <div class="user-name">{auth.me?.display_name}</div>
        <div class="user-sub">{auth.isRoot ? 'root' : auth.me?.username}</div>
      </div>
      <button class="icon-btn" onclick={() => auth.logout()} title="Sign out" aria-label="Sign out">⎋</button>
    </div>
  </div>
</nav>

{#snippet simpleRow(m: SidebarModule)}
  <button
    class="nav-item"
    class:active={isActive(m.id)}
    onclick={() => router.go(m.id)}
  >
    <Icon name={m.icon} size={14} />
    <span class="grow">{m.label}</span>
  </button>
{/snippet}

{#snippet editRow(m: SidebarModule, i: number)}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="edit-row"
    class:hidden-row={isHidden(m.id)}
    class:drag-over={dragOverId === m.id}
    draggable={true}
    ondragstart={(e) => onDragStart(e, m.id)}
    ondragover={(e) => onDragOver(e, m.id)}
    ondragleave={() => onDragLeave(m.id)}
    ondrop={(e) => onDrop(e, m.id)}
    ondragend={onDragEnd}
    data-testid={`sidebar-edit-row-${m.id}`}
  >
    <span class="grip" title="Drag to reorder" aria-hidden="true"><Icon name="grip" size={14} /></span>
    <Icon name={m.icon} size={14} />
    <span class="grow ellipsis">{m.label}</span>
    <button
      class="row-action"
      onclick={() => move(m.id, -1)}
      disabled={i === 0}
      title="Move up"
      aria-label={`Move ${m.label} up`}
    >
      <Icon name="arrowUp" size={12} />
    </button>
    <button
      class="row-action"
      onclick={() => move(m.id, 1)}
      disabled={i === navList.length - 1}
      title="Move down"
      aria-label={`Move ${m.label} down`}
    >
      <Icon name="arrowDown" size={12} />
    </button>
    <button
      class="row-action"
      onclick={() => ui.toggleSidebarHidden(m.id)}
      title={isHidden(m.id) ? 'Show' : 'Hide'}
      aria-label={isHidden(m.id) ? `Show ${m.label}` : `Hide ${m.label}`}
      data-testid={`sidebar-hide-${m.id}`}
    >
      <Icon name={isHidden(m.id) ? 'eyeOff' : 'eye'} size={13} />
    </button>
  </div>
{/snippet}

{#snippet agentsBlock()}
  <div class="nav-item-row">
    <button
      class="nav-item"
      class:active={router.module === 'agents' || router.module === ''}
      onclick={() => router.go('agents')}
      oncontextmenu={(e) => ctxMenu.show(e, [
        { label: 'New session…', icon: 'plus', action: () => (ui.newSessionOpen = true) },
        { label: 'Add workspace…', icon: 'folder', action: () => (ui.newWorkspaceOpen = true) },
      ])}
    >
      <Icon name="terminal" size={14} />
      <span class="grow">Agents</span>
      {#if ws.workingCount > 0}
        <span class="count-chip working">{ws.workingCount}</span>
      {/if}
    </button>
    <button
      class="icon-btn twisty"
      onclick={() => (agentsOpen = !agentsOpen)}
      aria-label="Toggle session list"
    >
      <Icon name={agentsOpen ? 'chevronDown' : 'chevronRight'} size={12} />
    </button>
  </div>

  {#if q ? fAgents.length > 0 : agentsOpen}
    <div class="nested">
      {#each fAgents as s (s.id)}
        {@render sessionRow(s)}
      {:else}
        <div class="nested-empty">No sessions — ⌘T to start one</div>
      {/each}
    </div>
  {/if}

  {#if q ? fTelegram.length > 0 : ws.telegramSessions.length > 0}
    <div class="nav-item-row">
      <button class="nav-item" onclick={() => (telegramOpen = !telegramOpen)}>
        <Icon name="send" size={14} />
        <span class="grow">Telegram</span>
        <span class="count-chip">{ws.telegramSessions.length}</span>
      </button>
      <button
        class="icon-btn twisty"
        onclick={() => (telegramOpen = !telegramOpen)}
        aria-label="Toggle Telegram list"
      >
        <Icon name={telegramOpen ? 'chevronDown' : 'chevronRight'} size={12} />
      </button>
    </div>
    {#if telegramOpen || q}
      <div class="nested">
        {#each visTelegram as s (s.id)}
          {@render sessionRow(s)}
        {:else}
          <div class="nested-empty">No matching</div>
        {/each}
        {#if !q && fTelegram.length > CHANNEL_CAP}
          <button class="show-more" onclick={() => (telegramShowAll = !telegramShowAll)}>
            {telegramShowAll ? 'Show less' : `Show ${fTelegram.length - CHANNEL_CAP} more`}
          </button>
        {/if}
      </div>
    {/if}
  {/if}

  {#if q ? fSlack.length > 0 : ws.slackSessions.length > 0}
    <div class="nav-item-row">
      <button class="nav-item" onclick={() => (slackOpen = !slackOpen)}>
        <Icon name="slack" size={14} />
        <span class="grow">Slack</span>
        <span class="count-chip">{ws.slackSessions.length}</span>
      </button>
      <button
        class="icon-btn twisty"
        onclick={() => (slackOpen = !slackOpen)}
        aria-label="Toggle Slack list"
      >
        <Icon name={slackOpen ? 'chevronDown' : 'chevronRight'} size={12} />
      </button>
    </div>
    {#if slackOpen || q}
      <div class="nested">
        {#each visSlack as s (s.id)}
          {@render sessionRow(s)}
        {:else}
          <div class="nested-empty">No matching</div>
        {/each}
        {#if !q && fSlack.length > CHANNEL_CAP}
          <button class="show-more" onclick={() => (slackShowAll = !slackShowAll)}>
            {slackShowAll ? 'Show less' : `Show ${fSlack.length - CHANNEL_CAP} more`}
          </button>
        {/if}
      </div>
    {/if}
  {/if}
{/snippet}

{#snippet connectionsBlock()}
  <div class="nav-item-row">
    <button
      class="nav-item"
      class:active={router.module === 'connections'}
      onclick={() => router.go('connections')}
    >
      <Icon name="plug" size={14} />
      <span class="grow">Connections</span>
      {#if ws.connectionSessions.length > 0}
        <span class="count-chip">{ws.connectionSessions.length}</span>
      {/if}
    </button>
    <button
      class="icon-btn twisty"
      onclick={() => (connectionsOpen = !connectionsOpen)}
      aria-label="Toggle connection list"
    >
      <Icon name={connectionsOpen ? 'chevronDown' : 'chevronRight'} size={12} />
    </button>
  </div>

  {#if connectionsOpen}
    <div class="nested">
      {#each ws.connectionSessions as s (s.id)}
        {@render sessionRow(s)}
      {:else}
        <div class="nested-empty">No open connections — open one from the page</div>
      {/each}
    </div>
  {/if}
{/snippet}

{#snippet sessionRow(s: Session)}
  {@const status = ws.statusMap[s.id] ?? s.status}
  {@const resumable = isResumable(s, status)}
  {@const sum = activity.summary(s.id)}
  {@const proofRow = proof.summaryFor('session', s.id)}
  {@const needsYou = ws.needsYou[s.id] === true}
  <div class="nested-row" class:needs-you={needsYou}>
    {#if renamingId === s.id}
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="nav-rename"
        bind:value={draft}
        autofocus
        onblur={commitRename}
        onkeydown={(e) => {
          if (e.key === 'Enter') commitRename();
          else if (e.key === 'Escape') renamingId = null;
        }}
      />
    {:else}
      <button
        class="nav-item nested-item"
        class:active={router.module === 'agents' && ws.activeSessionId === s.id}
        class:resumable
        onclick={() => openSession(s.id)}
        ondblclick={() => startRename(s.id, s.title)}
        oncontextmenu={(e) => ctxMenu.show(e, [
          { label: 'Rename', icon: 'edit', action: () => startRename(s.id, s.title) },
          { separator: true },
          ...(ws.myRole !== 'viewer' ? [
            { label: 'Archive', icon: 'archive', action: () => ws.archiveSession(s.id) },
            { label: 'Delete', icon: 'trash', danger: true as const, action: () => ws.killSession(s.id) },
          ] : []),
          { separator: true },
          { label: 'New session…', icon: 'plus', action: () => (ui.newSessionOpen = true) },
        ])}
        title={resumable ? `${s.title} — ${SUSPENDED_TIP}` : `${s.title} — double-click to rename`}
      >
        {#if resumable}
          <span class="susp-dot" aria-hidden="true">
            <Icon name="refresh" size={9} />
          </span>
        {:else}
          <StatusDot {status} />
        {/if}
        <span class="grow ellipsis">{s.title}</span>
        {#if needsYou}
          <span class="needs-you-dot" title="Waiting on you" aria-label="Needs you">
            <Icon name="bell" size={9} />
          </span>
        {/if}
        {#if sum && sum.total > 0}
          <span
            class="task-chip"
            class:done={sum.done === sum.total}
            class:active={sum.in_progress != null}
            title={sum.in_progress ? `Now: ${sum.in_progress}` : `${sum.done}/${sum.total} tasks done`}
          >{sum.done}/{sum.total}</span>
        {/if}
        {#if proofRow}
          <ProofStatusChip status={proofRow.status} risk={proofRow.risk_score} compact />
        {/if}
        {#if resumable}
          <span class="susp-pill" title={SUSPENDED_TIP}>resumable</span>
        {/if}
        <span class="provider">{s.provider}</span>
      </button>
      {#if ws.myRole !== 'viewer'}
        <button
          class="row-action"
          title="Archive session"
          aria-label="Archive session"
          onclick={() => ws.archiveSession(s.id)}
        >
          <Icon name="archive" size={12} />
        </button>
      {/if}
    {/if}
  </div>
{/snippet}

<style>
  .navigator {
    /* width is set inline from ui.railWidth (drag-resizable) */
    height: 100%;
    display: flex;
    flex-direction: column;
    border-inline-end: 1px solid var(--border);
    position: relative;
  }
  .rail-resize {
    position: absolute;
    inset-inline-end: -3px;
    top: 0;
    bottom: 0;
    width: 7px;
    cursor: col-resize;
    z-index: 20;
  }
  .rail-resize:hover,
  .navigator.resizing .rail-resize {
    background: linear-gradient(
      to right,
      transparent 0,
      color-mix(in srgb, var(--accent) 40%, transparent) 45%,
      color-mix(in srgb, var(--accent) 40%, transparent) 55%,
      transparent 100%
    );
  }
  .nav-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 10px 6px 14px;
  }
  .nav-logo {
    border-radius: 5px;
    display: block;
    flex-shrink: 0;
  }
  .nav-back :global(svg) {
    transform: scaleX(-1);
  }
  .nav-head .icon-btn:disabled {
    opacity: 0.3;
    cursor: default;
  }
  .nav-title {
    font-size: 13px;
    font-weight: 700;
    letter-spacing: -0.01em;
  }
  .nav-scroll {
    flex: 1;
    overflow-y: auto;
    padding: 4px 8px;
  }
  .nav-section {
    margin-bottom: 14px;
  }
  .nav-label {
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--text-dim);
    padding: 6px 8px 4px;
  }
  .nav-label-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-inline-end: 4px;
  }
  .nav-label-row .nav-label {
    flex: 1;
  }
  .add-ws {
    width: 22px;
    height: 22px;
    color: var(--text-dim);
  }
  .add-ws:hover {
    color: var(--text);
  }
  .nav-item-row {
    display: flex;
    align-items: center;
  }
  .nav-item-row .nav-item {
    flex: 1;
  }
  .twisty {
    width: 20px;
    height: 20px;
  }
  .nav-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 28px;
    padding: 0 8px;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: 12.5px;
    cursor: pointer;
    text-align: start;
    transition: background 120ms ease-out;
  }
  .nav-item:hover {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }
  .nav-item.active {
    /* Explicit high-contrast selection: a light-green fill with black text/
       icons. Independent of --accent (which is a dark blue that read as
       "black text on dark blue" — invisible). Reads clearly on dark AND light. */
    background: #7ee787;
    color: #0a0a0a;
    font-weight: 600;
    box-shadow: inset 3px 0 0 #2ea043;
  }
  .nav-item.active :global(svg) {
    color: #0a0a0a;
  }
  .nav-item.active-ws {
    font-weight: 600;
  }
  /* ── Sidebar edit mode ──────────────────────────────────────────────── */
  .edit-hint {
    margin: 2px 6px 8px;
    padding: 6px 8px;
    font-size: 11px;
    line-height: 1.4;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
    border-radius: var(--radius-s);
  }
  .edit-row {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 4px 0 6px;
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: 12.5px;
    cursor: grab;
    border: 1px solid transparent;
  }
  .edit-row:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .edit-row.drag-over {
    border-inline-start: 2px solid var(--accent);
  }
  /* Hidden modules stay listed while editing, dimmed, so they can be re-shown. */
  .edit-row.hidden-row {
    opacity: 0.45;
  }
  .edit-row .grip {
    display: grid;
    place-items: center;
    color: var(--text-dim);
    cursor: grab;
    flex-shrink: 0;
  }
  /* Edit-row action buttons are always visible (unlike the hover-revealed
     session row-actions). */
  .edit-row .row-action {
    opacity: 1;
  }
  .edit-row .row-action:disabled {
    opacity: 0.25;
    cursor: default;
  }
  .nested {
    margin: 2px 0 6px;
    padding-inline-start: 10px;
  }
  .nested-row {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .nested-row .nested-item {
    flex: 1;
    min-width: 0;
  }
  .nested-item {
    height: 26px;
    font-size: 12px;
  }
  .nested-item.archived {
    opacity: 0.65;
    cursor: default;
  }
  /* Suspended / resumable: parked to save memory, instantly reopenable.
     Dimmed (calm, not an error) but kept fully listed + clickable. */
  .nested-item.resumable:not(.active) {
    opacity: 0.72;
  }
  .nested-item.resumable:not(.active):hover {
    opacity: 1;
  }
  .susp-dot {
    display: grid;
    place-items: center;
    width: 7px;
    height: 7px;
    flex-shrink: 0;
    color: #febc2e;
  }
  .susp-pill {
    flex-shrink: 0;
    padding: 0 5px;
    height: 14px;
    line-height: 14px;
    border-radius: 999px;
    font-size: 9px;
    font-weight: 600;
    letter-spacing: 0.01em;
    color: #febc2e;
    background: color-mix(in srgb, #febc2e 16%, transparent);
  }
  .row-action {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    border-radius: var(--radius-s);
    cursor: pointer;
    opacity: 0;
    transition: opacity 120ms ease-out;
  }
  .nested-row:hover .row-action {
    opacity: 1;
  }
  .row-action:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  .row-action.danger:hover {
    color: var(--status-exited);
  }
  .nav-item.subtle {
    color: var(--text-dim);
  }
  .nav-rename {
    flex: 1;
    height: 24px;
    font-size: 12px;
    background: var(--surface-2);
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    color: var(--text);
    padding: 0 6px;
    margin: 1px 0;
    outline: none;
  }
  .nav-search {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 2px 4px 4px;
    padding: 0 8px;
    height: 26px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    color: var(--text-dim);
  }
  .nav-search-input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 12px;
    outline: none;
  }
  .search-clear {
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 15px;
    line-height: 1;
    padding: 0 2px;
  }
  .search-clear:hover {
    color: var(--text);
  }
  .nested-empty {
    padding: 4px 10px 6px;
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .show-more {
    display: block;
    width: 100%;
    text-align: start;
    padding: 4px 10px 6px;
    font-size: 11.5px;
    color: var(--text-dim);
    background: none;
    border: none;
    cursor: pointer;
  }
  .show-more:hover {
    color: var(--text);
  }
  .provider {
    font-size: 10px;
    color: var(--text-dim);
  }
  /* Per-session task roll-up: "done/total". Accent while a task is in progress,
     green when all complete. */
  .task-chip {
    flex-shrink: 0;
    padding: 0 5px;
    height: 14px;
    line-height: 14px;
    border-radius: 999px;
    font-size: 9px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 16%, transparent);
  }
  .task-chip.active {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .task-chip.done {
    color: var(--status-working, #3fb950);
    background: color-mix(in srgb, var(--status-working, #3fb950) 16%, transparent);
  }
  /* "Needs you" — sticky flag for a session blocked on operator input. Amber to
     stand out from the calmer status colors, without shouting. */
  .needs-you-dot {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 14px;
    height: 14px;
    border-radius: 99px;
    color: #febc2e;
    background: color-mix(in srgb, #febc2e 18%, transparent);
  }
  .nested-row.needs-you .nested-item:not(.active) {
    box-shadow: inset 2px 0 0 #febc2e;
  }
  .needs-you-filter {
    display: flex;
    align-items: center;
    gap: 8px;
    width: calc(100% - 8px);
    margin: 0 4px 6px;
    height: 26px;
    padding: 0 8px;
    border: 1px solid color-mix(in srgb, #febc2e 40%, transparent);
    background: color-mix(in srgb, #febc2e 8%, transparent);
    border-radius: var(--radius-s);
    color: #febc2e;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: background 120ms ease-out;
  }
  .needs-you-filter:hover {
    background: color-mix(in srgb, #febc2e 14%, transparent);
  }
  .needs-you-filter.active {
    background: color-mix(in srgb, #febc2e 22%, transparent);
  }
  .needs-you-count {
    min-width: 16px;
    height: 15px;
    padding: 0 4px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 700;
    display: grid;
    place-items: center;
    color: #1a1407;
    background: #febc2e;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count-chip {
    min-width: 16px;
    height: 15px;
    padding: 0 4px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 700;
    display: grid;
    place-items: center;
  }
  .count-chip.working {
    background: color-mix(in srgb, var(--status-working) 22%, transparent);
    color: var(--status-working);
  }
  .nav-foot {
    border-top: 1px solid var(--border);
    padding: 6px 8px 8px;
  }
  .nav-user {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px 2px;
  }
  .avatar {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--accent) 28%, transparent);
    color: var(--accent);
    font-size: 11px;
    font-weight: 600;
    display: grid;
    place-items: center;
    flex-shrink: 0;
  }
  .user-name {
    font-size: 12px;
    font-weight: 500;
    line-height: 1.2;
  }
  .user-sub {
    font-size: 10.5px;
    color: var(--text-dim);
  }
</style>
