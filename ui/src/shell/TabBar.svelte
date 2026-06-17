<script lang="ts">
  // Session tabs: click activates, middle-click closes, ⌘W closes active,
  // ⌃Tab cycles (handled in keys.ts → workspace store).
  import Icon from '../lib/components/Icon.svelte';
  import StatusDot from '../lib/components/StatusDot.svelte';
  import { ws, DB_PANE_ID } from '../lib/stores/workspace.svelte';
  import { ui, isTauri } from '../lib/stores/ui.svelte';
  import { router } from '../lib/router.svelte';
  import { ctxMenu } from '../lib/contextmenu.svelte';

  // `bellGutter` reserves space on the right so the shell's floating
  // notification bell never overlaps the tab-bar controls.
  let { bellGutter = false }: { bellGutter?: boolean } = $props();

  let renamingId: string | null = $state(null);
  let draft = $state('');

  function activate(id: string): void {
    ws.openSession(id);
    if (router.module !== 'agents') router.go('agents');
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
</script>

<div
  class="tabbar"
  class:tauri-pad={isTauri && !ui.railExpanded}
  class:bell-gutter={bellGutter}
  data-tauri-drag-region
>
  <div class="tabs">
    {#each ws.openTabs as id (id)}
      <div
        class="tab"
        class:active={ws.activeSessionId === id}
        class:resumable={isResumable(id)}
        role="tab"
        tabindex="0"
        aria-selected={ws.activeSessionId === id}
        onclick={() => activate(id)}
        onkeydown={(e) => e.key === 'Enter' && activate(id)}
        ondblclick={() => startRename(id)}
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
          { separator: true },
          {
            label: ws.viewMode === 'tabs' ? 'Switch to tiled view' : 'Switch to tabbed view',
            icon: ws.viewMode === 'tabs' ? 'grid' : 'square',
            action: () => ws.setViewMode(ws.viewMode === 'tabs' ? 'tiled' : 'tabs'),
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
    padding-left: 78px;
  }
  .tabbar.bell-gutter {
    padding-right: 42px;
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
