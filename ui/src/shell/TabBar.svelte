<script lang="ts">
  // Session tabs: click activates, middle-click closes, ⌘W closes active,
  // ⌃Tab cycles (handled in keys.ts → workspace store).
  import Icon from '../lib/components/Icon.svelte';
  import StatusDot from '../lib/components/StatusDot.svelte';
  import { ws, DB_PANE_ID } from '../lib/stores/workspace.svelte';
  import { ui, isTauri } from '../lib/stores/ui.svelte';
  import { startWindowDrag } from '../lib/windowDrag';
  import { router } from '../lib/router.svelte';
  import { ctxMenu } from '../lib/contextmenu.svelte';
  import ShareModal from '../modules/agents/ShareModal.svelte';

  // `bellGutter` reserves space on the right so the shell's floating
  // notification bell never overlaps the tab-bar controls.
  let { bellGutter = false }: { bellGutter?: boolean } = $props();

  // Share modal: tracks the session id we're sharing; null = closed.
  let shareSessionId = $state<string | null>(null);

  let renamingId: string | null = $state(null);
  let draft = $state('');

  function activate(id: string): void {
    // Navigate via the route so Back/Forward walk session history.
    ws.navigateToSession(id);
  }

  function title(id: string): string {
    if (id === DB_PANE_ID) return 'Database';
    return ws.sessions.find((s) => s.id === id)?.title ?? '…';
  }

  // A tab is "suspended / resumable" — parked to save memory, but auto-resumes
  // on open (`--resume`): status `reconnectable`, or an exited agent session
  // that still has a provider_session_id. A plain exited shell is "ended".
  function isResumable(id: string): boolean {
    const status = ws.statusMap[id] ?? 'idle';
    if (status === 'reconnectable') return true;
    if (status !== 'exited') return false;
    const s = ws.sessions.find((x) => x.id === id);
    return s?.kind === 'agent' && s.provider_session_id != null;
  }
  const SUSPENDED_TIP = 'Suspended to save memory — opens instantly';

  // A tab "needs you" when its session is blocked on operator input — distinct
  // from idle. Cleared by the store when the session is opened / fed input.
  function needsYou(id: string): boolean {
    return id !== DB_PANE_ID && ws.needsYou[id] === true;
  }

  function startRename(id: string): void {
    if (ws.myRole === 'viewer' || id === DB_PANE_ID) return;
    renamingId = id;
    draft = title(id);
  }

  async function commitRename(): Promise<void> {
    const id = renamingId;
    renamingId = null;
    if (!id) return;
    const next = draft.trim();
    if (next && next !== title(id)) await ws.renameSession(id, next);
  }

  // ── Drag-to-reorder tabs ─────────────────────────────────────────────────
  // HTML5 drag: tracks which tab is being dragged (dragId) and which position
  // the tab is being dragged over (dragOverId). On drop we reorder and persist.
  let dragId: string | null = $state(null);
  let dragOverId: string | null = $state(null);

  function onDragStart(e: DragEvent, id: string): void {
    dragId = id;
    e.dataTransfer?.setData('text/plain', id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }

  function onDragOver(e: DragEvent, id: string): void {
    // Only accept drags from our own tabs.
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
    if (!dragId || id === dragId) return;
    const targetIdx = ws.openTabs.indexOf(id);
    ws.reorderTab(dragId, targetIdx);
    dragId = null;
    dragOverId = null;
  }

  function onDragEnd(): void {
    dragId = null;
    dragOverId = null;
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="tabbar"
  class:tauri-pad={isTauri && !ui.railExpanded}
  class:bell-gutter={bellGutter}
  data-tauri-drag-region
  onmousedown={startWindowDrag}
>
  <div class="tabs">
    {#each ws.openTabs as id (id)}
      <div
        class="tab"
        class:active={ws.activeSessionId === id}
        class:resumable={isResumable(id)}
        class:needs-you={needsYou(id)}
        class:drag-over={dragOverId === id}
        draggable={id !== DB_PANE_ID}
        role="tab"
        tabindex="0"
        aria-selected={ws.activeSessionId === id}
        onclick={() => activate(id)}
        onkeydown={(e) => e.key === 'Enter' && activate(id)}
        ondblclick={() => startRename(id)}
        ondragstart={(e) => onDragStart(e, id)}
        ondragover={(e) => onDragOver(e, id)}
        ondragleave={() => onDragLeave(id)}
        ondrop={(e) => onDrop(e, id)}
        ondragend={onDragEnd}
        onauxclick={(e) => {
          if (e.button === 1) {
            e.preventDefault();
            ws.closeTab(id);
          }
        }}
        oncontextmenu={(e) => ctxMenu.show(e, [
          { label: 'Rename', icon: 'edit', action: () => startRename(id) },
          { label: 'Close tab', icon: 'x', action: () => ws.closeTab(id) },
          { separator: true },
          { label: 'New session…', icon: 'plus', action: () => (ui.newSessionOpen = true) },
          ...(id !== DB_PANE_ID
            ? [{ label: 'Share…', icon: 'share', action: () => (shareSessionId = id) }]
            : []),
          { separator: true },
          {
            label: ws.viewMode === 'tiled' ? 'Switch to tabbed view' : 'Switch to tiled view',
            icon: ws.viewMode === 'tiled' ? 'square' : 'grid',
            action: () => ws.setViewMode(ws.viewMode === 'tiled' ? 'tabs' : 'tiled'),
          },
          {
            label: ws.viewMode === 'mission' ? 'Exit Mission Control' : 'Mission Control',
            icon: 'gauge',
            action: () => ws.setViewMode(ws.viewMode === 'mission' ? 'tabs' : 'mission'),
          },
        ])}
      >
        {#if id === DB_PANE_ID}
          <Icon name="db" size={11} />
        {:else if isResumable(id)}
          <span class="susp-dot" title={SUSPENDED_TIP} aria-hidden="true">
            <Icon name="refresh" size={8} />
          </span>
        {:else}
          <StatusDot status={ws.statusMap[id] ?? 'idle'} size={6} />
        {/if}
        {#if renamingId === id}
          <!-- svelte-ignore a11y_autofocus -->
          <input
            class="tab-rename"
            bind:value={draft}
            autofocus
            onclick={(e) => e.stopPropagation()}
            onblur={commitRename}
            onkeydown={(e) => {
              if (e.key === 'Enter') commitRename();
              else if (e.key === 'Escape') renamingId = null;
            }}
          />
        {:else}
          <span
            class="tab-title"
            title={isResumable(id) ? SUSPENDED_TIP : 'Double-click to rename'}
          >{title(id)}</span>
          {#if needsYou(id)}
            <span class="tab-needs-you" title="Waiting on you" aria-label="Needs you">
              <Icon name="bell" size={9} />
            </span>
          {/if}
        {/if}
        <button
          class="tab-close"
          onclick={(e) => {
            e.stopPropagation();
            ws.closeTab(id);
          }}
          aria-label="Close tab"
          title="Close (⌘W)"
        >
          <Icon name="x" size={9} />
        </button>
      </div>
    {/each}
  </div>
  <div class="view-toggle" role="group" aria-label="View mode">
    <button
      class:active={ws.viewMode === 'tabs'}
      onclick={() => ws.setViewMode('tabs')}
      title="Tabbed view"
      aria-label="Tabbed view"
    >
      <Icon name="square" size={12} />
    </button>
    <button
      class:active={ws.viewMode === 'tiled'}
      onclick={() => ws.setViewMode('tiled')}
      title="Tiled view — see all sessions at once"
      aria-label="Tiled view"
    >
      <Icon name="grid" size={12} />
    </button>
    <button
      class:active={ws.viewMode === 'mission'}
      onclick={() => ws.setViewMode('mission')}
      title="Mission Control — work queue"
      aria-label="Mission Control"
    >
      <Icon name="gauge" size={12} />
    </button>
  </div>
  <button
    class="icon-btn new-tab"
    onclick={() => (ui.newSessionOpen = true)}
    title="New session (⌘T)"
    aria-label="New session"
  >
    <Icon name="plus" size={13} />
  </button>
</div>

<style>
  .tabbar {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 38px;
    padding: 0 8px;
    border-bottom: 1px solid var(--border);
    background: var(--bg);
    flex-shrink: 0;
  }
  .tabbar.tauri-pad {
    padding-inline-start: 78px;
  }
  .tabbar.bell-gutter {
    padding-inline-end: 42px;
  }
  .tabs {
    display: flex;
    align-items: center;
    gap: 2px;
    overflow-x: auto;
    scrollbar-width: none;
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 5px 0;
  }
  .tabs::-webkit-scrollbar {
    display: none;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 28px;
    padding: 0 6px 0 10px;
    border-radius: var(--radius-s);
    border: 1px solid transparent;
    color: var(--text-dim);
    font-size: 12.5px;
    cursor: pointer;
    white-space: nowrap;
    transition: background 120ms ease-out, color 120ms ease-out;
    max-width: 200px;
  }
  .tab:hover {
    background: var(--surface-2);
  }
  .tab.active {
    background: var(--surface);
    border-color: var(--border);
    color: var(--text);
  }
  /* Suspended / resumable tab: parked to save memory, reopens instantly.
     Slightly dimmed (calm, not an error); full opacity on hover/active. */
  .tab.resumable:not(.active) {
    opacity: 0.74;
  }
  .tab.resumable:not(.active):hover {
    opacity: 1;
  }
  /* Drop target while dragging a tab over it — faint left border beacon. */
  .tab.drag-over {
    border-inline-start: 2px solid var(--accent);
  }
  /* "Needs you" — blocked on operator input. Amber accents stand out from the
     calmer active/idle styling without being alarming. */
  .tab.needs-you:not(.active) {
    border-color: color-mix(in srgb, #febc2e 45%, transparent);
    color: var(--text);
  }
  .tab-needs-you {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 14px;
    height: 14px;
    border-radius: 99px;
    color: #febc2e;
    background: color-mix(in srgb, #febc2e 18%, transparent);
  }
  .susp-dot {
    display: grid;
    place-items: center;
    width: 8px;
    height: 8px;
    flex-shrink: 0;
    color: #febc2e;
  }
  .tab-title {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tab-close {
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    border: none;
    border-radius: 3px;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    opacity: 0;
    transition: opacity 120ms ease-out, background 120ms ease-out;
  }
  .tab:hover .tab-close,
  .tab.active .tab-close {
    opacity: 1;
  }
  .tab-close:hover {
    background: color-mix(in srgb, var(--text-dim) 22%, transparent);
    color: var(--text);
  }
  .new-tab {
    flex-shrink: 0;
  }
  .tab-rename {
    font-size: 12.5px;
    background: var(--surface-2);
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    color: var(--text);
    padding: 0 5px;
    max-width: 150px;
    outline: none;
  }
  .view-toggle {
    display: flex;
    flex-shrink: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .view-toggle button {
    display: grid;
    place-items: center;
    width: 26px;
    height: 22px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .view-toggle button.active {
    background: var(--surface);
    color: var(--accent);
  }
</style>

{#if shareSessionId}
  <ShareModal sessionId={shareSessionId} onclose={() => (shareSessionId = null)} />
{/if}
