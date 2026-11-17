<script lang="ts">
  // LSP Find-Usages bottom dock panel with resizable file-summary sidebar and
  // reference preview list.
  import Badge from '../../../lib/components/Badge.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import PaneDivider from '../../../lib/components/PaneDivider.svelte';
  import { copyWithToast } from '../../../lib/clipboard';
  import { loadPaneWidth } from '../../../lib/paneResizer';
  import { plural } from '../../../lib/plural';
  import { viewport } from '../../../lib/stores/viewport.svelte';
  import { ide, ideBasename, type IdeReferenceLocation } from '../ideStore.svelte';

  interface UsageGroup {
    path: string;
    relPath: string;
    items: IdeReferenceLocation[];
  }

  const LS_USG_W = 'otto.ide.usagesFilesW';
  let filesW = $state(loadPaneWidth(LS_USG_W, 240, 160, 480));
  let usagesQuery = $state('');
  let selectedFile = $state<string | null>(null);

  const filteredUsages = $derived.by((): IdeReferenceLocation[] => {
    const q = usagesQuery.trim().toLowerCase();
    if (!q) return ide.usages;
    return ide.usages.filter(
      (u) => u.path.toLowerCase().includes(q) || u.preview.toLowerCase().includes(q),
    );
  });

  const usageGroups = $derived.by((): UsageGroup[] => {
    const map = new Map<string, IdeReferenceLocation[]>();
    for (const u of filteredUsages) {
      const arr = map.get(u.path);
      if (arr) arr.push(u);
      else map.set(u.path, [u]);
    }
    const groups: UsageGroup[] = [];
    for (const [path, items] of map.entries()) {
      groups.push({ path, relPath: ide.relativePath(path) || ideBasename(path), items });
    }
    return groups;
  });

  const visibleGroups = $derived(
    selectedFile ? usageGroups.filter((g) => g.path === selectedFile) : usageGroups,
  );

  function findAtCursor(): void {
    if (!ide.activePath) return;
    void ide.findReferences(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col);
  }
</script>

<div class="usg-root" data-testid="ide-usages-panel">
  {#if ide.usages.length > 0 || ide.usagesSymbol}
    <header class="usg-bar">
      {#if ide.usagesSymbol}<Badge tone="accent" label={ide.usagesSymbol} />{/if}
      <span class="usg-summary dim">
        {plural(ide.usages.length, 'reference')} across {plural(usageGroups.length, 'file')}{#if ide.usagesSource === 'text'}<span title="No language server answered, so these are whole-word text matches" data-testid="ide-usages-text-match"> · text match</span>{/if}
      </span>
      <span class="grow"></span>
      {#if ide.usages.length > 0}
        <div class="input-group usg-search">
          <Icon name="search" size={12} />
          <input type="search" dir="ltr" aria-label="Filter usages" placeholder="Filter usages…" bind:value={usagesQuery} />
        </div>
      {/if}
      {#if ide.activePath}
        <button type="button" class="btn small ghost" onclick={findAtCursor}>
          <Icon name="refresh" size={12} /><span>Find at cursor</span>
        </button>
      {/if}
    </header>
  {/if}

  <div class="usg-body">
    <LoadState what="symbol usages" loading={ide.usagesLoading} empty={ide.usages.length === 0} variant="panel" rows={3}>
      {#snippet emptyView()}
        <EmptyState
          icon="search"
          title="No symbol usages yet"
          body="Place the caret on any symbol in the editor and run Find usages to list every reference across the project."
          actionLabel={ide.activePath ? 'Find usages at cursor' : undefined}
          actionIcon="search"
          actionKind="secondary"
          onaction={ide.activePath ? findAtCursor : undefined}
          headingLevel={2}
        />
      {/snippet}

      {#if usageGroups.length === 0}
        <EmptyState icon="filter" title="No matching usages" body="No reference locations match the active filter." actionLabel="Clear filter" actionKind="secondary" onaction={() => (usagesQuery = '')} headingLevel={2} />
      {:else}
        <div class="usg-split">
          {#if usageGroups.length > 1 && !viewport.isPhone}
            <aside class="usg-files" style:inline-size={`${filesW}px`} aria-label="Files with usages">
              <button type="button" class="usg-file-btn" class:active={selectedFile === null} onclick={() => (selectedFile = null)}>
                <span>All files</span><Badge tone="neutral" variant="outline" label={String(filteredUsages.length)} />
              </button>
              {#each usageGroups as g (g.path)}
                <button type="button" class="usg-file-btn mono" class:active={selectedFile === g.path} dir="ltr" onclick={() => (selectedFile = g.path)} title={g.path}>
                  <span class="ellipsis">{g.relPath}</span>
                  <Badge tone="neutral" variant="outline" label={String(g.items.length)} />
                </button>
              {/each}
            </aside>
            <PaneDivider bind:width={filesW} storageKey={LS_USG_W} min={160} max={480} defaultWidth={240} label="Resize usages file list" />
          {/if}

          <div class="usg-list">
            {#each visibleGroups as group (group.path)}
              <section class="usg-group" aria-label={`References in ${group.relPath}`}>
                <div class="usg-file-head">
                  <Icon name="file" size={12} />
                  <span class="usg-file-path mono ellipsis" dir="ltr" title={group.path}>{group.relPath}</span>
                  <Badge tone="neutral" variant="outline" label={String(group.items.length)} />
                </div>
                {#each group.items as u, idx (`${u.path}:${u.line}:${u.column}:${idx}`)}
                  <div class="usg-row-wrap reveal-host">
                    <button type="button" class="usg-row" onclick={() => void ide.openFile(u.path, { line: u.line, col: u.column })}>
                      <span class="usg-loc mono dim" dir="ltr">:{u.line}:{u.column}</span>
                      <span class="usg-preview mono ellipsis" dir="ltr" title={u.preview.trim()}>{u.preview.trim() || `${ideBasename(u.path)}:${u.line}`}</span>
                    </button>
                    <button type="button" class="icon-btn reveal-on-hover" aria-label={`Copy ${group.relPath}:${u.line}`} title={`Copy ${group.relPath}:${u.line}`} onclick={() => void copyWithToast(`${group.relPath}:${u.line}:${u.column}`, 'location')}>
                      <Icon name="copy" size={12} />
                    </button>
                  </div>
                {/each}
              </section>
            {/each}
          </div>
        </div>
      {/if}
    </LoadState>
  </div>
</div>

<style>
  .usg-root { display: flex; flex-direction: column; block-size: 100%; min-block-size: 0; min-inline-size: 0; background: var(--surface); }
  .usg-bar { display: flex; align-items: center; gap: 8px; block-size: 32px; min-block-size: 32px; padding-inline: 10px; border-block-end: 1px solid var(--border); background: var(--surface); flex-shrink: 0; overflow: hidden; }
  .usg-summary { font-size: var(--fs-xs); font-variant-numeric: tabular-nums; }
  .usg-search { block-size: 22px; inline-size: 180px; font-size: var(--fs-xs); }
  .usg-body { flex: 1; min-block-size: 0; display: flex; flex-direction: column; overflow: hidden; }
  .usg-split { flex: 1; min-block-size: 0; display: flex; overflow: hidden; }
  .usg-files { flex-shrink: 0; overflow-y: auto; border-inline-end: 1px solid var(--border); padding-block: 4px; }
  .usg-file-btn { inline-size: 100%; display: flex; align-items: center; justify-content: space-between; gap: 6px; padding: 4px 10px; border: 0; background: transparent; color: var(--text); font-size: var(--fs-xs); text-align: start; cursor: pointer; }
  .usg-file-btn:hover { background: var(--hover); }
  .usg-file-btn.active { background: var(--accent-soft); color: var(--accent-text); }
  .usg-list { flex: 1; min-inline-size: 0; overflow-y: auto; }
  .usg-group { border-block-end: 1px solid var(--separator); }
  .usg-file-head { display: flex; align-items: center; gap: 6px; padding: 6px 10px; background: var(--surface-2); font-size: var(--fs-xs); font-weight: 600; }
  .usg-file-path { flex: 1; font-size: var(--fs-xs); }
  .usg-row-wrap { display: flex; align-items: center; gap: 6px; padding-inline-end: 8px; border-block-start: 1px solid var(--separator); }
  .usg-row-wrap:hover { background: var(--hover); }
  .usg-row { flex: 1; min-inline-size: 0; display: flex; align-items: center; gap: 10px; padding-block: 4px; padding-inline: 20px 8px; border: 0; background: transparent; color: var(--text); font-size: var(--fs-xs); text-align: start; cursor: pointer; }
  .usg-loc { flex-shrink: 0; font-size: var(--fs-xs); font-variant-numeric: tabular-nums; }
  .usg-preview { flex: 1; font-size: var(--fs-xs); }
</style>
