<script lang="ts">
  // Multi-repo folder picker for creating Named Multi-Repo IDE Projects (e.g. 30+ Go or
  // Java microservices under `~`) or bulk-attaching sibling repositories in one step.
  import { onDestroy, untrack } from 'svelte';
  import { api } from '../../../lib/api/client';
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LazyMount from '../../../lib/components/LazyMount.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import Switch from '../../../lib/components/Switch.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { loadErrorText } from '../../../lib/loadError';
  import { plural } from '../../../lib/plural';
  import { toasts } from '../../../lib/toast.svelte';
  import { ide, ideBasename, type IdeFsEntry, type IdeFsTreeResp } from '../ideStore.svelte';

  const FolderPickerLazy = lazyComponent(() => import('../../../lib/components/FolderPicker.svelte'));

  interface Props {
    mode: 'create' | 'attach';
    onclose: () => void;
  }

  let { mode, onclose }: Props = $props();

  type RepoTech = 'go' | 'java' | 'rust' | 'node' | 'python';

  function initialParentDir(): string {
    if (ide.target.kind === 'local' && ide.target.root) {
      const clean = ide.target.root.replace(/\/+$/, '');
      const slash = clean.lastIndexOf('/');
      if (slash > 0) return clean.slice(0, slash);
    }
    return '~';
  }

  let projectName = $state('');
  let pinned = $state(true);
  let parentInput = $state(initialParentDir());
  let resolvedParent = $state('');
  let parentOfParent = $state<string | null>(null);
  let childDirs = $state<IdeFsEntry[]>([]);
  let techByPath = $state<Record<string, RepoTech[]>>({});
  let selectedMap = $state<Record<string, boolean>>({});
  let primaryRoot = $state('');
  let filter = $state('');
  let loading = $state(false);
  let loadErr = $state<string | null>(null);
  let saving = $state(false);
  let browseOpen = $state(false);
  let scanAbort: AbortController | null = null;

  const selectedPaths = $derived(
    Object.entries(selectedMap)
      .filter(([, on]) => on)
      .map(([p]) => p),
  );

  const filteredDirs = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return childDirs;
    return childDirs.filter((d) => {
      const techs = (techByPath[d.path] ?? []).join(' ');
      return d.name.toLowerCase().includes(q) || techs.includes(q);
    });
  });

  const goCount = $derived(childDirs.filter((d) => (techByPath[d.path] ?? []).includes('go')).length);
  const javaCount = $derived(childDirs.filter((d) => (techByPath[d.path] ?? []).includes('java')).length);

  $effect(() => {
    untrack(() => {
      if (mode === 'attach' && ide.hasTarget) {
        const pre: Record<string, boolean> = {};
        for (let i = 0; i < ide.projectRoots.length; i++) pre[ide.projectRoots[i].path] = true;
        selectedMap = pre;
      }
      void loadDirectory(parentInput);
    });
  });

  onDestroy(() => {
    scanAbort?.abort();
  });

  async function loadDirectory(rawDir: string): Promise<void> {
    const dir = rawDir.trim() || '~';
    scanAbort?.abort();
    const ctl = new AbortController();
    scanAbort = ctl;
    loading = true;
    loadErr = null;
    try {
      const q = new URLSearchParams({ kind: 'local', root: dir, show_hidden: 'false' }).toString();
      const resp = await api.get<IdeFsTreeResp>(`/ide/fs/tree?${q}`);
      if (ctl.signal.aborted) return;
      resolvedParent = resp.root || dir;
      parentInput = resolvedParent;
      parentOfParent = resp.parent ?? null;
      const dirs = (resp.entries ?? [])
        .filter((e) => e.kind === 'dir' && !e.name.startsWith('.'))
        .sort((a, b) => a.name.localeCompare(b.name));
      childDirs = dirs;
      void detectChildRepoTechs(dirs.slice(0, 80), ctl.signal);
    } catch (e) {
      if (!ctl.signal.aborted) loadErr = loadErrorText(e);
    } finally {
      if (!ctl.signal.aborted) loading = false;
    }
  }

  async function detectChildRepoTechs(dirs: IdeFsEntry[], signal: AbortSignal): Promise<void> {
    const queue = [...dirs];
    const concurrency = Math.min(6, queue.length);
    const workers = Array.from({ length: concurrency }, async () => {
      while (queue.length > 0 && !signal.aborted) {
        const entry = queue.shift();
        if (!entry) break;
        try {
          const q = new URLSearchParams({ kind: 'local', root: entry.path, show_hidden: 'false' }).toString();
          const sub = await api.get<IdeFsTreeResp>(`/ide/fs/tree?${q}`);
          if (signal.aborted) return;
          const names = new Set((sub.entries ?? []).map((x) => x.name));
          const detected: RepoTech[] = [];
          if (names.has('go.mod') || names.has('go.work')) detected.push('go');
          if (names.has('pom.xml') || names.has('build.gradle') || names.has('build.gradle.kts')) detected.push('java');
          if (names.has('Cargo.toml')) detected.push('rust');
          if (names.has('package.json')) detected.push('node');
          if (names.has('pyproject.toml') || names.has('requirements.txt')) detected.push('python');
          if (detected.length > 0) {
            techByPath = { ...techByPath, [entry.path]: detected };
          }
        } catch {
          // Unreadable child folder — skip silently.
        }
      }
    });
    await Promise.all(workers);
  }

  function toggleRepo(path: string, checked: boolean): void {
    const next = { ...selectedMap };
    if (checked) next[path] = true;
    else delete next[path];
    selectedMap = next;
    if (checked && !primaryRoot) primaryRoot = path;
    else if (!checked && primaryRoot === path) {
      primaryRoot = Object.keys(next)[0] ?? '';
    }
  }

  function selectByTech(tech: RepoTech): void {
    const next = { ...selectedMap };
    let added = 0;
    for (let i = 0; i < childDirs.length; i++) {
      const p = childDirs[i].path;
      if ((techByPath[p] ?? []).includes(tech)) {
        if (!next[p]) added += 1;
        next[p] = true;
      }
    }
    selectedMap = next;
    if (!primaryRoot) primaryRoot = Object.keys(next)[0] ?? '';
    if (mode === 'create' && !projectName.trim()) {
      projectName = tech === 'go' ? 'Go Microservices' : 'Java Services';
    }
    if (added === 0) toasts.info(`No ${tech === 'go' ? 'Go' : 'Maven / Gradle'} repos detected in this folder`);
  }

  function selectVisible(): void {
    const next = { ...selectedMap };
    for (let i = 0; i < filteredDirs.length; i++) next[filteredDirs[i].path] = true;
    selectedMap = next;
    if (!primaryRoot) primaryRoot = Object.keys(next)[0] ?? '';
  }

  function clearSelection(): void {
    selectedMap = {};
    primaryRoot = '';
  }

  async function handleConfirm(): Promise<void> {
    if (selectedPaths.length === 0) {
      toasts.info('Select at least one repository folder');
      return;
    }
    saving = true;
    try {
      if (mode === 'create') {
        const primary = selectedMap[primaryRoot] ? primaryRoot : selectedPaths[0];
        const ordered = [primary, ...selectedPaths.filter((p) => p !== primary)];
        const fallbackName =
          ordered.length > 1
            ? `${ideBasename(primary)} + ${ordered.length - 1}`
            : ideBasename(primary) || 'IDE Project';
        const saved = await ide.saveNamedProject({
          name: projectName.trim() || fallbackName,
          primary_root: primary,
          folders: ordered,
          pinned,
        });
        if (saved) {
          await ide.activateProject(saved.id);
          toasts.success(`Created project "${saved.name}"`, plural(ordered.length, 'repository'));
          onclose();
        }
      } else {
        const added = await ide.attachProjectFolders(selectedPaths);
        toasts.success(added > 0 ? `Attached ${plural(added, 'repository')}` : 'Repositories already attached');
        onclose();
      }
    } finally {
      saving = false;
    }
  }
</script>

<Modal
  title={mode === 'create' ? 'New Multi-Repo IDE Project' : 'Bulk Attach Repositories to Project'}
  width={680}
  height={580}
  {onclose}
  dismissable={!saving}
>
  <div class="bulk-body" data-testid="ide-bulk-repo-modal">
    {#if mode === 'create'}
      <div class="bulk-meta-grid">
        <div class="bulk-field">
          <label class="bulk-label" for="bulk-proj-name">Project name</label>
          <input
            id="bulk-proj-name"
            class="input"
            type="text"
            dir="auto"
            placeholder="e.g. Go Core, Sinatra Go, BO Java…"
            bind:value={projectName}
            data-autofocus
            data-testid="ide-bulk-project-name"
          />
        </div>
        <div class="bulk-pin-row">
          <Switch checked={pinned} label="Pin project in IDE header bar" onchange={(v) => (pinned = v)} />
          <span class="bulk-pin-copy">Show 1-click pill in IDE header</span>
        </div>
      </div>
    {/if}

    <div class="bulk-parent-bar">
      <label class="bulk-label" for="bulk-parent-dir">Parent directory containing repositories</label>
      <div class="bulk-parent-controls">
        <input
          id="bulk-parent-dir"
          class="input bulk-mono"
          type="text"
          dir="ltr"
          bind:value={parentInput}
          onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); void loadDirectory(parentInput); } }}
          data-testid="ide-bulk-parent-input"
        />
        {#if parentOfParent}
          <button
            type="button"
            class="btn small ghost"
            title="Go to parent folder"
            onclick={() => parentOfParent && void loadDirectory(parentOfParent)}
          >
            <Icon name="arrowUp" size={12} />
            Up
          </button>
        {/if}
        <button type="button" class="btn small ghost" onclick={() => (browseOpen = true)}>
          <Icon name="folder" size={12} />
          Browse…
        </button>
        <button type="button" class="btn small" onclick={() => void loadDirectory(parentInput)}>
          Scan
        </button>
      </div>
    </div>

    <div class="bulk-Quick-bar">
      <div class="bulk-search">
        <Icon name="search" size={13} />
        <input
          type="search"
          class="bulk-search-in"
          dir="ltr"
          aria-label="Filter child folders"
          placeholder="Filter folders (e.g. go_, bo_, auth)…"
          bind:value={filter}
          data-testid="ide-bulk-repo-filter"
        />
      </div>
      <div class="bulk-quick-btns">
        <button
          type="button"
          class="btn small ghost"
          onclick={() => selectByTech('go')}
          data-testid="ide-bulk-select-go"
        >
          Select all Go repos{goCount > 0 ? ` (${goCount})` : ''}
        </button>
        <button
          type="button"
          class="btn small ghost"
          onclick={() => selectByTech('java')}
          data-testid="ide-bulk-select-java"
        >
          Select all Maven/Gradle{javaCount > 0 ? ` (${javaCount})` : ''}
        </button>
        <button type="button" class="btn small ghost" onclick={selectVisible} data-testid="ide-bulk-select-visible">
          Select visible
        </button>
        <button type="button" class="btn small ghost" onclick={clearSelection}>Clear</button>
      </div>
    </div>

    <div class="bulk-list" role="group" aria-label="Child repository folders">
      {#if loading || loadErr || filteredDirs.length === 0}
        <LoadState
          what="repositories"
          variant="compact"
          {loading}
          error={loadErr}
          empty={!loading && !loadErr && filteredDirs.length === 0}
          onretry={() => void loadDirectory(parentInput)}
        >
          {#snippet emptyView()}
            <div class="bulk-summary">No subfolders match.</div>
          {/snippet}
        </LoadState>
      {:else}
        {#each filteredDirs as entry (entry.path)}
          {@const checked = Boolean(selectedMap[entry.path])}
          {@const techs = techByPath[entry.path] ?? []}
          {@const isPrimary = mode === 'create' && checked && (primaryRoot === entry.path || (!selectedMap[primaryRoot] && selectedPaths[0] === entry.path))}
          <div class="bulk-row" class:is-checked={checked} data-testid="ide-bulk-repo-row">
            <label class="bulk-check-lbl">
              <input
                type="checkbox"
                {checked}
                onchange={(e) => toggleRepo(entry.path, (e.currentTarget as HTMLInputElement).checked)}
              />
              <Icon name="folder" size={14} />
              <span class="bulk-repo-name" dir="ltr">{entry.name}</span>
            </label>
            <div class="bulk-badges">
              {#if techs.includes('go')}<Badge tone="info" variant="outline" label="Go" />{/if}
              {#if techs.includes('java')}<Badge tone="warn" variant="outline" label="Maven/Gradle" />{/if}
              {#if techs.includes('rust')}<Badge tone="neutral" variant="outline" label="Rust" />{/if}
              {#if techs.includes('node')}<Badge tone="neutral" variant="outline" label="Node/TS" />{/if}
              {#if techs.includes('python')}<Badge tone="neutral" variant="outline" label="Python" />{/if}
              {#if mode === 'create' && checked}
                <button
                  type="button"
                  class="btn small ghost"
                  aria-pressed={isPrimary}
                  onclick={() => (primaryRoot = entry.path)}
                  title="Use as primary project root"
                >
                  {isPrimary ? 'Primary' : 'Set primary'}
                </button>
              {/if}
            </div>
          </div>
        {/each}
      {/if}
    </div>
  </div>

  {#snippet footer()}
    <span class="bulk-summary">{plural(selectedPaths.length, 'repository', 'repositories')} selected</span>
    <div class="bulk-foot-btns">
      <button type="button" class="btn ghost" onclick={onclose} disabled={saving}>Cancel</button>
      <button
        type="button"
        class="btn primary"
        onclick={() => void handleConfirm()}
        disabled={saving || selectedPaths.length === 0}
        data-testid="ide-bulk-confirm-btn"
      >
        {saving
          ? 'Applying…'
          : mode === 'create'
            ? `Create project (${selectedPaths.length})`
            : `Attach ${plural(selectedPaths.length, 'folder')}`}
      </button>
    </div>
  {/snippet}
</Modal>

{#if browseOpen}
  <LazyMount
    lazy={FolderPickerLazy}
    what="folder picker"
    quiet
    props={{
      title: 'Choose parent directory containing repositories',
      start: resolvedParent || parentInput,
      onpick: (p: string) => { browseOpen = false; if (p) void loadDirectory(p); },
      onclose: () => (browseOpen = false),
    }}
    onerror={() => (browseOpen = false)}
  />
{/if}

<style>
  .bulk-body { display: flex; flex-direction: column; gap: 12px; min-block-size: 0; block-size: 100%; }
  .bulk-meta-grid { display: flex; align-items: flex-end; gap: 16px; flex-wrap: wrap; }
  .bulk-field { flex: 1; min-inline-size: 220px; display: flex; flex-direction: column; gap: 4px; }
  .bulk-label { font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim); }
  .bulk-pin-row { display: inline-flex; align-items: center; gap: 8px; padding-block-end: 6px; }
  .bulk-pin-copy { font-size: var(--fs-xs); color: var(--text); }
  .bulk-parent-bar { display: flex; flex-direction: column; gap: 4px; }
  .bulk-parent-controls { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .bulk-mono { flex: 1; min-inline-size: 200px; font-family: var(--font-mono); font-size: var(--fs-xs); }
  .bulk-Quick-bar { display: flex; flex-direction: column; gap: 6px; }
  .bulk-search {
    display: flex; align-items: center; gap: 8px; padding-block: 6px; padding-inline: 10px;
    border: 1px solid var(--control-border); border-radius: var(--radius-s); background: var(--surface-2); color: var(--text-dim);
  }
  .bulk-search:focus-within { border-color: var(--accent); box-shadow: 0 0 0 2px var(--accent-soft); }
  .bulk-search-in { flex: 1; min-inline-size: 0; border: 0; background: transparent; color: var(--text); font-size: var(--fs-s); outline: none; }
  .bulk-quick-btns { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .bulk-list {
    flex: 1; min-block-size: 180px; overflow-y: auto; border: 1px solid var(--border);
    border-radius: var(--radius-s); background: var(--surface); padding: 4px; display: flex; flex-direction: column; gap: 2px;
  }
  .bulk-row {
    display: flex; align-items: center; justify-content: space-between; gap: 8px;
    padding-block: 6px; padding-inline: 8px; border-radius: var(--radius-s); border: 1px solid transparent;
  }
  .bulk-row:hover { background: var(--hover); }
  .bulk-row.is-checked { background: var(--accent-faint); border-color: var(--accent-line); }
  .bulk-check-lbl { display: flex; align-items: center; gap: 8px; flex: 1; min-inline-size: 0; cursor: pointer; font-size: var(--fs-s); color: var(--text); }
  .bulk-repo-name { font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .bulk-badges { display: flex; align-items: center; gap: 6px; flex-shrink: 0; }
  .bulk-summary { font-size: var(--fs-xs); color: var(--text-dim); margin-inline-end: auto; }
  .bulk-foot-btns { display: flex; align-items: center; gap: 8px; }
</style>
