<script lang="ts">
  // GoLand / IntelliJ Dependency Analyzer View (`[ Text | Dependency Analyzer ]` on manifest tabs):
  // Filterable two-pane package explorer, CVE advisories, managed-in parent jump, and 1-click upgrades.
  import Badge from '../../../lib/components/Badge.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import {
    detectManifestEcosystem,
    executeIdeDepAction,
    inspectIdeDependencies,
    parseManifestDependencies,
    type IdeDepItem,
  } from '../../../lib/components/cm-ide-deps';

  interface Props {
    filePath: string;
    content: string;
    root: string;
    connectionId?: string | null;
    projectFolders?: string[];
    readOnly?: boolean;
    initialDeps?: IdeDepItem[];
    onapplycontent: (nextContent: string) => void;
    onjumplocation: (loc: { path: string; line: number; column: number }) => void;
    onswitchtext: (line: number) => void;
  }

  let {
    filePath, content, root, connectionId = null, projectFolders = [], readOnly = false,
    initialDeps = [], onapplycontent, onjumplocation, onswitchtext,
  }: Props = $props();

  let filterText = $state('');
  let filterMode = $state<'all' | 'upgradable' | 'vulnerable' | 'deprecated'>('all');
  let loading = $state(false);
  let busyAction = $state<string | null>(null);
  let items = $state<IdeDepItem[]>([]);
  let selectedName = $state<string | null>(null);

  const eco = $derived(detectManifestEcosystem(filePath));

  async function refreshDeps(): Promise<void> {
    loading = true;
    try {
      const inspected = await inspectIdeDependencies({ root, connectionId, projectFolders, path: filePath, content });
      items = inspected.length > 0 ? inspected : parseManifestDependencies(filePath, content);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    const _fp = filePath;
    const _c = content;
    void _fp; void _c;
    items = initialDeps.length > 0 ? initialDeps : parseManifestDependencies(filePath, content);
    void refreshDeps();
  });

  const filtered = $derived.by(() => {
    const q = filterText.trim().toLowerCase();
    return items.filter((d) => {
      if (q && !d.name.toLowerCase().includes(q) && !d.current_version.toLowerCase().includes(q)) return false;
      if (filterMode === 'upgradable') return Boolean(d.latest_version && d.latest_version !== d.current_version);
      if (filterMode === 'vulnerable') return Boolean(d.vulnerabilities?.length);
      if (filterMode === 'deprecated') return Boolean(d.deprecated);
      return true;
    });
  });

  const selectedDep = $derived(filtered.find((d) => d.name === selectedName) ?? filtered[0] ?? null);

  async function runAction(action: string, dep?: IdeDepItem | null, version?: string | null): Promise<void> {
    busyAction = action;
    try {
      const res = await executeIdeDepAction({ root, connectionId, projectFolders, path: filePath, content, action, dep, version, deps: items });
      if (res.updated_content && res.updated_content !== content) onapplycontent(res.updated_content);
      await refreshDeps();
    } finally {
      busyAction = null;
    }
  }
</script>

<div class="ide-dep-analyzer" data-testid="ide-dependency-analyzer">
  <div class="ide-da-toolbar">
    <input
      class="ide-da-search"
      type="search"
      dir="ltr"
      aria-label="Filter dependencies"
      placeholder="Search packages…"
      bind:value={filterText}
      data-testid="ide-dep-search-input"
    />
    <div class="segmented" role="group" aria-label="Dependency filter">
      <button type="button" aria-pressed={filterMode === 'all'} onclick={() => (filterMode = 'all')}>All ({items.length})</button>
      <button type="button" aria-pressed={filterMode === 'upgradable'} onclick={() => (filterMode = 'upgradable')}>Upgradable</button>
      <button type="button" aria-pressed={filterMode === 'vulnerable'} onclick={() => (filterMode = 'vulnerable')}>Vulnerable</button>
      <button type="button" aria-pressed={filterMode === 'deprecated'} onclick={() => (filterMode = 'deprecated')}>Deprecated</button>
    </div>
    <div class="ide-da-actions">
      <button type="button" class="btn small ghost" disabled={loading || Boolean(busyAction)} onclick={() => void refreshDeps()}>
        <Icon name="refresh" size={12} />Refresh
      </button>
      <button type="button" class="btn small ghost" disabled={readOnly || Boolean(busyAction)} onclick={() => void runAction('update_all_latest')} data-testid="ide-dep-update-all-btn">
        Upgrade all
      </button>
      {#if eco === 'go'}
        <button type="button" class="btn small ghost" disabled={readOnly || Boolean(busyAction)} onclick={() => void runAction('tidy')}>Go Mod Tidy</button>
      {:else}
        <button type="button" class="btn small ghost" disabled={readOnly || Boolean(busyAction)} onclick={() => void runAction(eco === 'maven' ? 'maven_sync' : eco === 'gradle' ? 'gradle_sync' : 'sync')}>Sync</button>
      {/if}
    </div>
  </div>

  {#if filtered.length === 0}
    <div class="ide-da-empty">
      <EmptyState icon="box" title="No matching dependencies" body="Try clearing the search filter or switching back to All dependencies." />
    </div>
  {:else}
    <div class="ide-da-panes">
      <div class="ide-da-list" role="listbox" aria-label="Dependencies">
        {#each filtered as dep (`${dep.name}:${dep.line}`)}
          {@const hasUpdate = Boolean(dep.latest_version && dep.latest_version !== dep.current_version)}
          <button
            type="button"
            role="option"
            aria-selected={selectedDep?.name === dep.name}
            class="ide-da-row"
            class:active={selectedDep?.name === dep.name}
            onclick={() => (selectedName = dep.name)}
            data-testid="ide-dep-analyzer-row"
          >
            <span class="ide-da-pkg" class:deprecated={dep.deprecated} dir="ltr">{dep.name}</span>
            <span class="ide-da-ver" dir="ltr">
              {dep.current_version || 'managed'}{#if hasUpdate} → {dep.latest_version}{/if}
            </span>
            {#if dep.vulnerabilities?.length}<Badge tone="bad" label="{dep.vulnerabilities.length} CVE" />{/if}
            {#if dep.deprecated}<Badge tone="warn" variant="outline" label="Deprecated" />{/if}
            {#if dep.indirect}<Badge tone="neutral" variant="outline" label="indirect" />{/if}
          </button>
        {/each}
      </div>

      {#if selectedDep}
        <div class="ide-da-detail" data-testid="ide-dep-analyzer-detail">
          <div class="ide-da-detail-head">
            <h3 class="ide-da-title" dir="ltr">{selectedDep.name}</h3>
            <button type="button" class="btn small ghost" onclick={() => onswitchtext(selectedDep.line)}>Jump to line {selectedDep.line}</button>
          </div>
          <div class="ide-da-meta" dir="ltr">
            <span>Current: <strong>{selectedDep.current_version || 'managed'}</strong></span>
            {#if selectedDep.latest_patch}<span>Patch: <strong>{selectedDep.latest_patch}</strong></span>{/if}
            {#if selectedDep.latest_version}<span>Latest: <strong>{selectedDep.latest_version}</strong></span>{/if}
            {#if selectedDep.scope}<span>Scope: {selectedDep.scope}</span>{/if}
          </div>

          {#if selectedDep.managed_in}
            {@const mLoc = selectedDep.managed_in}
            <div class="ide-da-callout">
              <span>Version managed in parent manifest:</span>
              <button type="button" class="btn small ghost" onclick={() => onjumplocation({ path: mLoc.path, line: mLoc.line, column: 1 })} data-testid="ide-dep-managed-jump">
                🎯↑ {mLoc.path.split('/').pop()}:{mLoc.line}
              </button>
            </div>
          {/if}

          {#if selectedDep.deprecated}
            <div class="ide-da-callout is-warn">{selectedDep.deprecation_note || 'This package is marked deprecated.'}</div>
          {/if}

          {#if selectedDep.vulnerabilities?.length}
            <div class="ide-da-vulns">
              {#each selectedDep.vulnerabilities as v (v.id)}
                <div class="ide-da-vuln-card">
                  <div class="ide-da-vuln-top">
                    <Badge tone="bad" label={v.id} />
                    <span>{v.severity}</span>
                    {#if v.fixed_version}<span>Fixed in {v.fixed_version}</span>{/if}
                  </div>
                  <p>{v.summary}</p>
                </div>
              {/each}
            </div>
          {/if}

          {#if (selectedDep.latest_version && selectedDep.latest_version !== selectedDep.current_version) || (selectedDep.latest_patch && selectedDep.latest_patch !== selectedDep.current_version)}
            <div class="ide-da-actions">
              {#if selectedDep.latest_version && selectedDep.latest_version !== selectedDep.current_version}
                <button
                  type="button"
                  class="btn small primary"
                  disabled={readOnly || Boolean(busyAction)}
                  onclick={() => void runAction('update_one', selectedDep, selectedDep.latest_version)}
                  data-testid="ide-dep-upgrade-single-btn"
                >
                  Upgrade to {selectedDep.latest_version}
                </button>
              {/if}
              {#if selectedDep.latest_patch && selectedDep.latest_patch !== selectedDep.current_version && selectedDep.latest_patch !== selectedDep.latest_version}
                <button
                  type="button"
                  class="btn small ghost"
                  disabled={readOnly || Boolean(busyAction)}
                  onclick={() => void runAction('update_one', selectedDep, selectedDep.latest_patch)}
                >
                  Upgrade to patch {selectedDep.latest_patch}
                </button>
              {/if}
            </div>
          {/if}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .ide-dep-analyzer { display: flex; flex-direction: column; flex: 1; min-block-size: 0; background: var(--bg); }
  .ide-da-toolbar {
    display: flex; align-items: center; gap: var(--sp-3); padding: var(--sp-2) 10px;
    background: var(--surface); border-block-end: 1px solid var(--border); flex-wrap: wrap;
  }
  .ide-da-search {
    inline-size: 200px; block-size: 26px; padding-inline: var(--sp-3);
    background: var(--bg); color: var(--text); border: 1px solid var(--control-border);
    border-radius: var(--radius-s); font-size: var(--fs-xs);
  }
  .ide-da-actions { display: inline-flex; align-items: center; gap: var(--sp-2); margin-inline-start: auto; }
  .ide-da-empty { flex: 1; display: flex; align-items: center; justify-content: center; }
  .ide-da-panes { flex: 1; min-block-size: 0; display: flex; }
  .ide-da-list { flex: 1; min-inline-size: 0; overflow-y: auto; border-inline-end: 1px solid var(--border); padding: var(--sp-2); display: flex; flex-direction: column; gap: 2px; }
  .ide-da-row {
    display: flex; align-items: center; gap: var(--sp-2); padding: 6px var(--sp-3);
    background: transparent; border: none; border-radius: var(--radius-s);
    color: var(--text); font-size: var(--fs-xs); text-align: start; cursor: pointer;
  }
  .ide-da-row:hover, .ide-da-row.active { background: var(--surface-2); }
  .ide-da-row.active { box-shadow: inset 2px 0 0 var(--accent-solid); }
  .ide-da-pkg { flex: 1; min-inline-size: 0; font-family: var(--font-mono); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ide-da-pkg.deprecated { text-decoration: line-through; color: var(--text-dim); }
  .ide-da-ver { font-family: var(--font-mono); color: var(--text-dim); flex-shrink: 0; }
  .ide-da-detail { inline-size: 360px; max-inline-size: 46%; padding: var(--sp-4); overflow-y: auto; display: flex; flex-direction: column; gap: var(--sp-3); }
  .ide-da-detail-head { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); }
  .ide-da-title { margin: 0; font-size: var(--fs-m); font-family: var(--font-mono); font-weight: 600; overflow: hidden; text-overflow: ellipsis; }
  .ide-da-meta { display: flex; flex-wrap: wrap; gap: var(--sp-3); font-size: var(--fs-xs); color: var(--text-dim); }
  .ide-da-callout { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); padding: var(--sp-2) var(--sp-3); background: var(--surface-2); border-radius: var(--radius-s); font-size: var(--fs-xs); }
  .ide-da-callout.is-warn { background: var(--warning-soft); color: var(--warning); }
  .ide-da-vulns { display: flex; flex-direction: column; gap: var(--sp-2); }
  .ide-da-vuln-card { padding: var(--sp-3); background: var(--danger-soft); border-radius: var(--radius-s); font-size: var(--fs-xs); }
  .ide-da-vuln-top { display: flex; align-items: center; gap: var(--sp-2); margin-block-end: var(--sp-1); font-weight: 500; }
  .ide-da-vuln-card p { margin: 0; color: var(--text); }
</style>
