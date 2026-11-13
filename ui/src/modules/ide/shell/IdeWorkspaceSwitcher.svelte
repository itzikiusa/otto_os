<script lang="ts">
  // Spacious, viewport-clamped macOS-style Workspace & Multi-Folder Project Switcher
  // for the IDE header. Shows full project names + paths on wide screens and opens
  // a searchable switcher with 1-click folder picker & "Add Folder to Project…".
  import { tick } from 'svelte';
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LazyMount from '../../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../../lib/stores/workspace.svelte';
  import { ide, ideBasename } from '../ideStore.svelte';

  const FolderPickerLazy = lazyComponent(() => import('../../../lib/components/FolderPicker.svelte'));

  let menuOpen = $state(false);
  let query = $state('');
  let folderPickerMode = $state<'open' | 'attach' | null>(null);
  let triggerEl = $state<HTMLButtonElement | undefined>(undefined);
  let popEl = $state<HTMLDivElement | undefined>(undefined);
  let searchEl = $state<HTMLInputElement | undefined>(undefined);
  let pos = $state({ left: 12, top: 48, width: 480, maxH: 440 });

  function shortenHomePath(p: string): string {
    if (!p) return '';
    const m = p.match(/^\/Users\/[^/]+(\/.*)?$/);
    if (m) return `~${m[1] || ''}`;
    const mLinux = p.match(/^\/home\/[^/]+(\/.*)?$/);
    if (mLinux) return `~${mLinux[1] || ''}`;
    return p;
  }

  const primaryLabel = $derived.by((): string => {
    if (!ide.hasTarget) return 'Select project';
    if (ide.target.kind === 'ssh') {
      const host = ide.target.connection_name || 'SSH';
      return `${host}:${ideBasename(ide.target.root)}`;
    }
    const matchedWs = ws.workspaces.find(
      (w) => w.id !== SCRATCH_WORKSPACE_ID && w.root_path === ide.target.root,
    );
    return matchedWs?.name || ideBasename(ide.target.root) || ide.target.root;
  });

  const subtitlePath = $derived(
    ide.hasTarget ? shortenHomePath(ide.target.root) : '',
  );

  const extraRootCount = $derived(Math.max(0, ide.projectRoots.length - 1));

  const filteredWorkspaces = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const list = ws.workspaces.filter((w) => w.id !== SCRATCH_WORKSPACE_ID && w.root_path);
    if (!q) return list;
    return list.filter(
      (w) =>
        (w.name || '').toLowerCase().includes(q) ||
        w.root_path.toLowerCase().includes(q),
    );
  });

  function clampPosition(): void {
    if (typeof window === 'undefined' || !triggerEl) return;
    const rect = triggerEl.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const width = Math.max(260, Math.min(540, vw - 16));
    const isRtl = document.documentElement.dir === 'rtl';
    const rawLeft = isRtl ? rect.right - width : rect.left;
    const left = Math.max(8, Math.min(rawLeft, vw - width - 8));
    const maxH = Math.max(200, Math.min(520, vh - 16));
    const measuredH = popEl ? Math.min(popEl.offsetHeight || 320, maxH) : 320;
    const desiredTop = rect.bottom + 6;
    const top = Math.max(8, Math.min(desiredTop, vh - measuredH - 8));
    pos = { left, top, width, maxH };
  }

  async function toggleMenu(): Promise<void> {
    menuOpen = !menuOpen;
    if (menuOpen) {
      query = '';
      clampPosition();
      await tick();
      clampPosition();
      searchEl?.focus();
    }
  }

  function closeMenu(): void {
    menuOpen = false;
  }

  $effect(() => {
    if (!menuOpen) return;
    void filteredWorkspaces.length;
    requestAnimationFrame(() => clampPosition());
  });

  $effect(() => {
    if (!menuOpen) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        closeMenu();
        triggerEl?.focus();
      }
    };
    const onResize = (): void => clampPosition();
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('resize', onResize);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('resize', onResize);
    };
  });

  async function pickWorkspace(rootPath: string, id: string): Promise<void> {
    closeMenu();
    await ide.setLocalTarget(rootPath, id);
  }

  function startFolderPick(mode: 'open' | 'attach'): void {
    closeMenu();
    folderPickerMode = mode;
  }

  async function handlePickedFolder(picked: string): Promise<void> {
    const mode = folderPickerMode;
    folderPickerMode = null;
    if (!picked) return;
    if (mode === 'attach' && ide.hasTarget) {
      await ide.addProjectFolder(picked);
    } else {
      await ide.setLocalTarget(picked);
    }
  }
</script>

<button
  bind:this={triggerEl}
  type="button"
  class="ws-switcher-btn"
  aria-haspopup="dialog"
  aria-expanded={menuOpen}
  title={ide.hasTarget ? `${ide.target.kind === 'ssh' ? 'SSH' : 'Project'}: ${ide.target.root}` : 'Select workspace or project folder'}
  onclick={() => void toggleMenu()}
  data-testid="ide-target-switcher"
>
  <span class="ws-icon" aria-hidden="true">
    <Icon name={ide.target.kind === 'ssh' ? 'server' : 'folder'} size={13} />
  </span>
  <span class="ws-name" dir="ltr">{primaryLabel}</span>
  {#if extraRootCount > 0}
    <Badge tone="accent" variant="outline" label={`+${extraRootCount}`} />
  {/if}
  {#if subtitlePath}
    <span class="ws-path" dir="ltr">{subtitlePath}</span>
  {/if}
  <Icon name="chevronDown" size={12} />
</button>

{#if menuOpen}
  <div class="ws-backdrop" role="presentation" onclick={closeMenu}></div>
  <div
    bind:this={popEl}
    class="ws-popover"
    role="dialog"
    aria-label="Switch workspace or project folder"
    style:inset-inline-start="{pos.left}px"
    style:top="{pos.top}px"
    style:inline-size="{pos.width}px"
    style:max-block-size="{pos.maxH}px"
    data-testid="ide-workspace-popover"
  >
    <div class="ws-search-bar">
      <Icon name="search" size={13} />
      <input
        bind:this={searchEl}
        class="ws-search-in"
        type="search"
        dir="ltr"
        aria-label="Filter workspaces and folders"
        placeholder="Search workspaces or paths…"
        bind:value={query}
        data-testid="ide-workspace-search"
      />
    </div>

    <div class="ws-scroll">
      {#if ide.hasTarget && ide.projectRoots.length > 1}
        <div class="ws-sec-label">Folders in current project ({ide.projectRoots.length})</div>
        {#each ide.projectRoots as root (root.path)}
          <div class="ws-attached-row">
            <Icon name="folder" size={13} />
            <div class="ws-item-copy">
              <span class="ws-item-title" dir="ltr">
                {root.name}
                {#if root.isPrimary}<Badge tone="neutral" variant="outline" label="Primary" />{/if}
              </span>
              <span class="ws-item-sub" dir="ltr">{shortenHomePath(root.path)}</span>
            </div>
            {#if !root.isPrimary}
              <button
                type="button"
                class="btn small ghost"
                onclick={() => void ide.removeProjectFolder(root.path)}
              >
                Remove
              </button>
            {/if}
          </div>
        {/each}
      {/if}

      <div class="ws-sec-label">Workspaces ({filteredWorkspaces.length})</div>
      {#if filteredWorkspaces.length === 0}
        <div class="ws-empty">
          {query.trim() ? 'No workspaces match this filter.' : 'No registered workspaces yet — open any local folder below.'}
        </div>
      {:else}
        {#each filteredWorkspaces as w (w.id)}
          {@const active = ide.target.kind === 'local' && ide.target.root === w.root_path}
          <button
            type="button"
            class="ws-item"
            class:is-active={active}
            onclick={() => void pickWorkspace(w.root_path, w.id)}
            data-testid="ide-workspace-option"
          >
            <span class="ws-item-icon">
              <Icon name="folder" size={14} />
            </span>
            <span class="ws-item-copy">
              <span class="ws-item-title" dir="ltr">{w.name || ideBasename(w.root_path)}</span>
              <span class="ws-item-sub" dir="ltr">{shortenHomePath(w.root_path)}</span>
            </span>
            {#if active}
              <Badge tone="ok" dot label="Active" />
            {/if}
          </button>
        {/each}
      {/if}
    </div>

    <div class="ws-actions-footer">
      <button
        type="button"
        class="ws-action-btn"
        onclick={() => startFolderPick('open')}
        data-testid="ide-open-local-folder-btn"
      >
        <Icon name="folder" size={13} />
        <span>Open local folder…</span>
      </button>
      {#if ide.hasTarget && ide.target.kind === 'local'}
        <button
          type="button"
          class="ws-action-btn"
          onclick={() => startFolderPick('attach')}
          data-testid="ide-switcher-add-folder"
        >
          <Icon name="plus" size={13} />
          <span>Add folder to project…</span>
        </button>
      {/if}
      <button
        type="button"
        class="ws-action-btn"
        onclick={() => { closeMenu(); ide.sshOpen = true; }}
        data-testid="ide-switcher-ssh"
      >
        <Icon name="server" size={13} />
        <span>Remote SSH…</span>
      </button>
    </div>
  </div>
{/if}

{#if folderPickerMode}
  <LazyMount
    lazy={FolderPickerLazy}
    what="folder picker"
    quiet
    props={{
      title: folderPickerMode === 'attach' ? 'Add folder to project' : 'Open project folder',
      start: ide.target.kind === 'local' ? ide.target.root : '',
      onpick: (p: string) => void handlePickedFolder(p),
      onclose: () => (folderPickerMode = null),
    }}
    onerror={() => (folderPickerMode = null)}
  />
{/if}

<style>
  .ws-switcher-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-inline-size: min(560px, 42vw);
    min-inline-size: 0;
    block-size: 28px;
    padding-inline: 10px;
    background: var(--surface-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    font-size: var(--fs-s);
    cursor: pointer;
    transition: border-color var(--dur-fast) var(--ease-out), background var(--dur-fast) var(--ease-out);
  }
  .ws-switcher-btn:hover {
    border-color: var(--border-strong);
    background: var(--hover);
  }
  .ws-icon {
    display: inline-flex;
    color: var(--accent-text);
    flex-shrink: 0;
  }
  .ws-name {
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-inline-size: 0;
  }
  .ws-path {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-inline-size: 220px;
  }
  .ws-backdrop {
    position: fixed;
    inset: 0;
    z-index: var(--z-popover-backdrop);
  }
  .ws-popover {
    position: fixed;
    z-index: var(--z-popover);
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-m);
    box-shadow: var(--shadow-card);
    overflow: hidden;
  }
  .ws-search-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-block: 8px;
    padding-inline: 12px;
    border-block-end: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .ws-search-bar:focus-within {
    border-color: var(--accent);
    box-shadow: inset 0 -1px 0 var(--accent);
  }
  .ws-search-in {
    flex: 1;
    min-inline-size: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    outline: none;
  }
  .ws-scroll {
    flex: 1;
    min-block-size: 0;
    overflow-y: auto;
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ws-sec-label {
    padding-block: 6px 2px;
    padding-inline: 8px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
  }
  .ws-empty {
    padding: 12px 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .ws-item,
  .ws-attached-row {
    display: flex;
    align-items: center;
    gap: 10px;
    inline-size: 100%;
    padding-block: 6px;
    padding-inline: 8px;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    text-align: start;
  }
  .ws-item {
    cursor: pointer;
  }
  .ws-item:hover {
    background: var(--hover);
  }
  .ws-item.is-active {
    background: var(--accent-soft);
    border-color: var(--accent-line);
  }
  .ws-item-icon {
    display: inline-flex;
    color: var(--accent-text);
    flex-shrink: 0;
  }
  .ws-item-copy {
    flex: 1;
    min-inline-size: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ws-item-title {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
    word-break: break-word;
  }
  .ws-item-sub {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ws-actions-footer {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 8px;
    border-block-start: 1px solid var(--border);
    background: var(--surface-2);
    flex-shrink: 0;
  }
  .ws-action-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding-block: 6px;
    padding-inline: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-xs);
    font-weight: 500;
    cursor: pointer;
  }
  .ws-action-btn:hover {
    background: var(--hover);
    border-color: var(--border-strong);
  }
  @media (max-width: 1024px) {
    .ws-path {
      display: none;
    }
    .ws-switcher-btn {
      max-inline-size: 280px;
    }
  }
  @media (max-width: 640px) {
    .ws-switcher-btn {
      max-inline-size: 145px;
      block-size: 26px;
      padding-inline: 8px;
      font-size: var(--fs-xs);
    }
  }
</style>
