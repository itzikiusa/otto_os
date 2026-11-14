<script lang="ts">
  // IntelliJ IDEA right-side Maven / Gradle Tool Window:
  // Profiles (Maven/Gradle + Spring Boot), multi-module projects, Lifecycle, Plugins, Dependencies, Repositories.
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { ide, type IdeBuildToolModule, type IdeBuildToolProfile } from '../ideStore.svelte';
  import {
    executeBuildToolGoal,
    formatHomeShortPath,
    GRADLE_LIFECYCLE,
    loadBuildToolWorkspaceModel,
    MAVEN_LIFECYCLE,
  } from './ideBuildToolModel';

  let loading = $state(false);
  let error = $state<string | null>(null);
  let filter = $state('');
  let tool = $state<'maven' | 'gradle'>('maven');
  let profiles = $state<IdeBuildToolProfile[]>([]);
  let springProfiles = $state<string[]>([]);
  let modules = $state<IdeBuildToolModule[]>([]);
  let expanded = $state<Record<string, boolean>>({ profiles: false });
  let hoveredMod = $state<IdeBuildToolModule | null>(null);

  const labelTitle = $derived((ide.buildToolKind ?? tool) === 'gradle' ? 'Gradle' : 'Maven');
  const badgeMark = $derived((ide.buildToolKind ?? tool) === 'gradle' ? 'G' : 'm');

  async function refreshModel(): Promise<void> {
    if (!ide.hasTarget) return;
    loading = true;
    error = null;
    try {
      const res = await loadBuildToolWorkspaceModel();
      tool = res.tool;
      profiles = res.profiles;
      springProfiles = res.springProfiles;
      modules = res.modules;
      const nextExp = { ...expanded };
      if (modules.length <= 4) {
        for (const m of modules) {
          if (nextExp[`mod:${m.root_dir}`] === undefined) nextExp[`mod:${m.root_dir}`] = true;
        }
      }
      expanded = nextExp;
    } catch (e) {
      error = e instanceof Error ? e.message : 'Couldn’t load build tool projects';
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void ide.target.root;
    void ide.additionalRoots.length;
    void refreshModel();
  });

  function toggleNode(key: string): void {
    expanded = { ...expanded, [key]: !expanded[key] };
  }

  function collapseAll(): void {
    expanded = { profiles: false };
  }

  async function promptExecuteGoal(): Promise<void> {
    const primaryMod = modules[0];
    if (!primaryMod) return;
    const goal = await confirmer.promptText(`Enter ${labelTitle} goal or task to run in ${primaryMod.name}:`, {
      title: `Execute ${labelTitle} Goal`,
      placeholder: primaryMod.tool === 'gradle' ? 'clean build' : 'clean package',
      confirmLabel: 'Execute',
    });
    if (goal?.trim()) executeBuildToolGoal(primaryMod, goal.trim(), 'run');
  }

  async function promptAddProfile(kind: 'maven' | 'spring'): Promise<void> {
    const name = await confirmer.promptText(
      kind === 'spring' ? 'Activate Spring Boot profile (SPRING_PROFILES_ACTIVE):' : `Activate ${labelTitle} profile (-P):`,
      { title: kind === 'spring' ? 'Add Spring Profile' : `Add ${labelTitle} Profile`, placeholder: 'local', confirmLabel: 'Activate' },
    );
    const id = name?.trim();
    if (!id) return;
    if (kind === 'spring') {
      if (!springProfiles.includes(id)) springProfiles = [...springProfiles, id];
      if (!ide.activeSpringProfiles.includes(id)) ide.toggleSpringProfile(id);
    } else {
      if (!profiles.some((p) => p.id === id)) profiles = [...profiles, { id, name: id, kind: tool }];
      if (!ide.activeMavenProfiles.includes(id)) ide.toggleMavenProfile(id);
    }
    expanded = { ...expanded, profiles: true };
  }

  function downloadSources(): void {
    const mod = modules[0];
    if (!mod) return;
    executeBuildToolGoal(mod, mod.tool === 'gradle' ? 'dependencies' : 'dependency:sources', 'run');
  }

  const allMavenProfileIds = $derived.by(() => {
    const set = new Set(profiles.map((p) => p.id));
    for (const id of ide.activeMavenProfiles) set.add(id);
    return [...set];
  });

  const allSpringProfileIds = $derived.by(() => {
    const set = new Set(springProfiles);
    for (const id of ide.activeSpringProfiles) set.add(id);
    return [...set];
  });

  function matchesFilter(text: string): boolean {
    const q = filter.trim().toLowerCase();
    return !q || text.toLowerCase().includes(q);
  }
</script>

{#snippet moduleTree(mod: IdeBuildToolModule, depth: number)}
  {@const modKey = `mod:${mod.root_dir}`}
  {@const isOpen = Boolean(expanded[modKey]) || Boolean(filter.trim())}
  {@const lifecycle = mod.lifecycle && mod.lifecycle.length > 0 ? mod.lifecycle : (mod.tool === 'gradle' ? GRADLE_LIFECYCLE : MAVEN_LIFECYCLE)}
  {#if matchesFilter(mod.name) || matchesFilter(mod.artifact_id ?? '') || Boolean(filter.trim())}
    <div class="bt-node" style:padding-inline-start="{depth * 12}px">
      <div class="bt-row bt-mod-row">
        <button
          type="button"
          class="bt-row-main"
          aria-expanded={isOpen}
          onmouseenter={() => (hoveredMod = mod)}
          onmouseleave={() => { if (hoveredMod?.root_dir === mod.root_dir) hoveredMod = null; }}
          onfocus={() => (hoveredMod = mod)}
          onblur={() => { if (hoveredMod?.root_dir === mod.root_dir) hoveredMod = null; }}
          onclick={() => toggleNode(modKey)}
          ondblclick={() => void ide.openFile(mod.manifest_path, { preview: false })}
          title={`Project: ${mod.project_coord || [mod.group_id, mod.artifact_id].filter(Boolean).join(':') || mod.name}\nLocation: ${mod.location_display || formatHomeShortPath(mod.root_dir)}`}
        >
          <Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} noflip />
          <span class="bt-mark">{mod.tool === 'gradle' ? 'G' : 'm'}</span>
          <span class="bt-name" dir="ltr">{mod.name}</span>
          {#if depth === 0 && mod.modules && mod.modules.length > 0}
            <span class="bt-sub">root</span>
          {/if}
        </button>
        <button
          type="button"
          class="bt-mini-btn"
          aria-label={`Open ${mod.name} build file`}
          title={`Open ${formatHomeShortPath(mod.manifest_path)}`}
          onclick={() => void ide.openFile(mod.manifest_path, { preview: false })}
        >
          <Icon name="file" size={12} />
        </button>
      </div>

      {#if isOpen}
        {@const lcKey = `${modKey}:lc`}
        <div class="bt-sub-group" style:padding-inline-start="12px">
          <button type="button" class="bt-row bt-folder-row" aria-expanded={Boolean(expanded[lcKey])} onclick={() => toggleNode(lcKey)}>
            <Icon name={expanded[lcKey] || Boolean(filter.trim()) ? 'chevronDown' : 'chevronRight'} size={12} noflip />
            <Icon name="layers" size={12} />
            <span class="bt-folder-lbl">Lifecycle</span>
          </button>
          {#if expanded[lcKey] || Boolean(filter.trim())}
            {#each lifecycle.filter((g) => matchesFilter(g)) as phase (phase)}
              <div class="bt-goal-row">
                <button
                  type="button"
                  class="bt-goal-main"
                  ondblclick={() => executeBuildToolGoal(mod, phase, 'run')}
                  onclick={() => executeBuildToolGoal(mod, phase, 'run')}
                  title={`Run ${phase} in ${mod.name}`}
                >
                  <Icon name="play" size={12} />
                  <span dir="ltr">{phase}</span>
                </button>
                <button
                  type="button"
                  class="bt-mini-btn"
                  aria-label={`Debug ${phase}`}
                  title={`Debug ${phase} in ${mod.name}`}
                  onclick={() => executeBuildToolGoal(mod, phase, 'debug')}
                >
                  <Icon name="bug" size={12} />
                </button>
              </div>
            {/each}
          {/if}

          {#if mod.plugins && mod.plugins.length > 0}
            {@const plKey = `${modKey}:pl`}
            <button type="button" class="bt-row bt-folder-row" aria-expanded={Boolean(expanded[plKey])} onclick={() => toggleNode(plKey)}>
              <Icon name={expanded[plKey] ? 'chevronDown' : 'chevronRight'} size={12} noflip />
              <Icon name="plug" size={12} />
              <span class="bt-folder-lbl">Plugins</span>
            </button>
            {#if expanded[plKey]}
              {#each mod.plugins as plug (plug.name)}
                {@const pgKey = `${plKey}:${plug.name}`}
                <div style:padding-inline-start="12px">
                  <button type="button" class="bt-row bt-folder-row" aria-expanded={Boolean(expanded[pgKey])} onclick={() => toggleNode(pgKey)}>
                    <Icon name={expanded[pgKey] ? 'chevronDown' : 'chevronRight'} size={12} noflip />
                    <span class="bt-name" dir="ltr">{plug.name}</span>
                  </button>
                  {#if expanded[pgKey]}
                    {#each plug.goals ?? [] as goal (goal)}
                      <div class="bt-goal-row">
                        <button type="button" class="bt-goal-main" onclick={() => executeBuildToolGoal(mod, goal, 'run')} title={`Run ${goal}`}>
                          <Icon name="play" size={12} />
                          <span dir="ltr">{goal}</span>
                        </button>
                        <button type="button" class="bt-mini-btn" aria-label={`Debug ${goal}`} title={`Debug ${goal}`} onclick={() => executeBuildToolGoal(mod, goal, 'debug')}>
                          <Icon name="bug" size={12} />
                        </button>
                      </div>
                    {/each}
                  {/if}
                </div>
              {/each}
            {/if}
          {/if}

          {#if mod.dependencies && mod.dependencies.length > 0}
            {@const depKey = `${modKey}:dep`}
            <button type="button" class="bt-row bt-folder-row" aria-expanded={Boolean(expanded[depKey])} onclick={() => toggleNode(depKey)}>
              <Icon name={expanded[depKey] ? 'chevronDown' : 'chevronRight'} size={12} noflip />
              <Icon name="box" size={12} />
              <span class="bt-folder-lbl">Dependencies ({mod.dependencies.length})</span>
            </button>
            {#if expanded[depKey]}
              {#each mod.dependencies as dep (dep)}
                <div class="bt-leaf" dir="ltr" title={dep}>{dep}</div>
              {/each}
            {/if}
          {/if}

          {#if mod.repositories && mod.repositories.length > 0}
            {@const repKey = `${modKey}:rep`}
            <button type="button" class="bt-row bt-folder-row" aria-expanded={Boolean(expanded[repKey])} onclick={() => toggleNode(repKey)}>
              <Icon name={expanded[repKey] ? 'chevronDown' : 'chevronRight'} size={12} noflip />
              <Icon name="server" size={12} />
              <span class="bt-folder-lbl">Repositories</span>
            </button>
            {#if expanded[repKey]}
              {#each mod.repositories as rep (`${rep.id}:${rep.url}`)}
                <div class="bt-leaf" dir="ltr" title={rep.url}>{rep.url && rep.url !== rep.id ? `${rep.id} (${rep.url})` : rep.id}</div>
              {/each}
            {/if}
          {/if}

          {#each mod.modules ?? mod.children ?? [] as child (child.root_dir)}
            {@render moduleTree(child, depth + 1)}
          {/each}
        </div>
      {/if}
    </div>
  {/if}
{/snippet}

<aside class="bt-win" aria-label={`${labelTitle} tool window`} data-testid="ide-build-tool-window">
  <header class="bt-hdr">
    <div class="bt-hdr-title">
      <span class="bt-mark is-hdr">{badgeMark}</span>
      <span>{labelTitle}</span>
      {#if ide.activeMavenProfiles.length + ide.activeSpringProfiles.length > 0}
        <Badge tone="accent" label={`${ide.activeMavenProfiles.length + ide.activeSpringProfiles.length} active`} />
      {/if}
    </div>
    <button
      type="button"
      class="icon-btn"
      aria-label={`Hide ${labelTitle} tool window`}
      title={`Hide ${labelTitle} tool window`}
      onclick={() => ide.toggleBuildTool(false)}
    >
      <Icon name="minus" size={13} />
    </button>
  </header>

  <div class="bt-toolbar" role="toolbar" aria-label={`${labelTitle} actions`}>
    <button type="button" class="icon-btn" aria-label={`Reload all ${labelTitle} projects`} title={`Reload All ${labelTitle} Projects`} onclick={() => void refreshModel()} data-testid="ide-bt-reload">
      <Icon name="refresh" size={13} />
    </button>
    <button type="button" class="icon-btn" aria-label="Download sources and documentation" title="Download Sources and Documentation" onclick={downloadSources} data-testid="ide-bt-download-sources">
      <Icon name="download" size={13} />
    </button>
    <button type="button" class="icon-btn" aria-label={`Execute ${labelTitle} goal`} title={`Execute ${labelTitle} Goal…`} onclick={() => void promptExecuteGoal()} data-testid="ide-bt-exec-goal">
      <Icon name="play" size={13} />
    </button>
    <button type="button" class="icon-btn bt-toggle" aria-pressed={ide.buildOffline} aria-label="Toggle offline mode" title="Toggle Offline Mode (-o / --offline)" onclick={() => ide.setBuildOffline(!ide.buildOffline)} data-testid="ide-bt-offline">
      <Icon name="cloud" size={13} />
    </button>
    <button type="button" class="icon-btn bt-toggle" aria-pressed={ide.buildSkipTests} aria-label="Toggle skip tests" title="Toggle 'Skip Tests' Mode (-DskipTests)" onclick={() => ide.setBuildSkipTests(!ide.buildSkipTests)} data-testid="ide-bt-skip-tests">
      <Icon name="flask" size={13} />
    </button>
    <button type="button" class="icon-btn" aria-label="Collapse all" title="Collapse All" onclick={collapseAll} data-testid="ide-bt-collapse-all">
      <Icon name="fold" size={13} />
    </button>
    <button type="button" class="icon-btn" aria-label="Build tool settings" title={`${labelTitle} Settings…`} onclick={() => ide.openSettings('build.tools')}>
      <Icon name="gear" size={13} />
    </button>
  </div>

  <div class="bt-filter-row">
    <Icon name="search" size={12} />
    <input class="bt-filter-in" type="search" dir="ltr" aria-label={`Filter ${labelTitle} modules and goals`} placeholder="Filter goals, modules, profiles…" bind:value={filter} />
  </div>

  {#if hoveredMod}
    <div class="bt-hover-card" data-testid="ide-bt-module-card">
      <div class="bt-hc-line"><strong>Project:</strong> <span dir="ltr">{hoveredMod.project_coord || [hoveredMod.group_id, hoveredMod.artifact_id].filter(Boolean).join(':') || hoveredMod.name}</span></div>
      <div class="bt-hc-line"><strong>Location:</strong> <span dir="ltr">{hoveredMod.location_display || formatHomeShortPath(hoveredMod.root_dir)}</span></div>
    </div>
  {/if}

  <div class="bt-body" role="tree" aria-label={`${labelTitle} project tree`}>
    <LoadState
      what={`${labelTitle} modules`}
      loading={loading && modules.length === 0}
      {error}
      variant="compact"
      onretry={() => void refreshModel()}
    >
      <div class="bt-node" data-testid="ide-bt-profiles">
        <div class="bt-row bt-mod-row">
          <button type="button" class="bt-row-main" aria-expanded={Boolean(expanded.profiles)} onclick={() => toggleNode('profiles')}>
            <Icon name={expanded.profiles ? 'chevronDown' : 'chevronRight'} size={12} noflip />
            <Icon name="folder" size={13} />
            <span class="bt-name">Profiles</span>
          </button>
          <button type="button" class="bt-mini-btn" aria-label="Add build profile" title={`Activate ${labelTitle} profile…`} onclick={() => void promptAddProfile('maven')}>
            <Icon name="plus" size={12} />
          </button>
        </div>

        {#if expanded.profiles}
          <div class="bt-sub-group" style:padding-inline-start="18px">
            {#each allMavenProfileIds as pid (pid)}
              <label class="bt-prof-row">
                <input type="checkbox" checked={ide.activeMavenProfiles.includes(pid)} onchange={() => ide.toggleMavenProfile(pid)} />
                <span dir="ltr">{pid}</span>
                <Badge tone="neutral" variant="outline" label={tool} />
              </label>
            {/each}
            {#each allSpringProfileIds as sp (sp)}
              <label class="bt-prof-row">
                <input type="checkbox" checked={ide.activeSpringProfiles.includes(sp)} onchange={() => ide.toggleSpringProfile(sp)} />
                <span dir="ltr">{sp}</span>
                <Badge tone="info" variant="outline" label="Spring" />
              </label>
            {/each}
            <div class="bt-prof-actions">
              <button type="button" class="btn small ghost" onclick={() => void promptAddProfile('spring')}>+ Spring profile…</button>
              <button type="button" class="btn small ghost" onclick={() => void promptAddProfile('maven')}>+ {labelTitle} profile…</button>
            </div>
          </div>
        {/if}
      </div>

      {#if modules.length === 0}
        <div class="bt-state">No {labelTitle} manifests detected in open project roots.</div>
      {:else}
        {#each modules as mod (mod.root_dir)}
          {@render moduleTree(mod, 0)}
        {/each}
      {/if}
    </LoadState>
  </div>
</aside>

<style>
  .bt-win { display: flex; flex-direction: column; block-size: 100%; min-block-size: 0; min-inline-size: 0; background: var(--surface); border-inline-start: 1px solid var(--border); }
  .bt-hdr { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding-block: 6px; padding-inline: 10px; border-block-end: 1px solid var(--border); }
  .bt-hdr-title { display: flex; align-items: center; gap: 6px; font-size: var(--fs-xs); font-weight: 600; color: var(--text); }
  .bt-mark { display: inline-flex; align-items: center; justify-content: center; inline-size: 15px; block-size: 15px; border-radius: var(--radius-s); background: var(--accent-soft); color: var(--accent-text); font-size: var(--fs-xs); font-weight: 600; flex-shrink: 0; }
  .bt-toolbar { display: flex; align-items: center; gap: 2px; padding-block: 4px; padding-inline: 8px; border-block-end: 1px solid var(--border); background: var(--surface-2); flex-wrap: wrap; }
  .bt-toggle[aria-pressed='true'] { background: var(--accent-soft); color: var(--accent-text); border-color: var(--accent-line); }
  .bt-filter-row { display: flex; align-items: center; gap: 6px; padding-block: 6px; padding-inline: 10px; border-block-end: 1px solid var(--border); color: var(--text-dim); }
  .bt-filter-row:focus-within { border-color: var(--accent); box-shadow: inset 0 -1px 0 var(--accent); }
  .bt-filter-in { flex: 1; min-inline-size: 0; border: 0; background: transparent; color: var(--text); font-size: var(--fs-xs); outline: none; }
  .bt-hover-card { padding-block: 6px; padding-inline: 10px; background: var(--surface-2); border-block-end: 1px solid var(--border); font-size: var(--fs-xs); color: var(--text-dim); display: flex; flex-direction: column; gap: 2px; }
  .bt-hc-line { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .bt-hc-line strong { color: var(--text); font-weight: 600; }
  .bt-body { flex: 1; min-block-size: 0; overflow-y: auto; padding-block: 6px; padding-inline: 6px; display: flex; flex-direction: column; gap: 2px; }
  .bt-state { padding: 14px 10px; font-size: var(--fs-xs); color: var(--text-dim); }
  .bt-row { display: flex; align-items: center; gap: 6px; inline-size: 100%; border: 0; border-radius: var(--radius-s); background: transparent; color: var(--text); font-size: var(--fs-xs); padding-block: 4px; padding-inline: 4px; text-align: start; cursor: pointer; }
  .bt-row:hover, .bt-goal-row:hover { background: var(--hover); }
  .bt-mod-row { justify-content: space-between; padding-inline-end: 2px; }
  .bt-row-main { display: flex; align-items: center; gap: 6px; flex: 1; min-inline-size: 0; border: 0; background: transparent; color: var(--text); font-size: var(--fs-xs); font-weight: 500; text-align: start; cursor: pointer; padding: 0; }
  .bt-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-inline-size: 0; }
  .bt-sub { color: var(--text-dim); font-size: var(--fs-xs); font-weight: 400; }
  .bt-folder-row { color: var(--text-dim); }
  .bt-folder-lbl { color: var(--text); }
  .bt-goal-row { display: flex; align-items: center; justify-content: space-between; border-radius: var(--radius-s); padding-inline-start: 16px; padding-inline-end: 2px; }
  .bt-goal-main { display: flex; align-items: center; gap: 6px; flex: 1; min-inline-size: 0; border: 0; background: transparent; color: var(--text); font-size: var(--fs-xs); padding-block: 4px; padding-inline: 0; text-align: start; cursor: pointer; }
  .bt-mini-btn { display: inline-flex; align-items: center; justify-content: center; inline-size: 20px; block-size: 20px; border: 0; border-radius: var(--radius-s); background: transparent; color: var(--text-dim); cursor: pointer; flex-shrink: 0; }
  .bt-mini-btn:hover { background: var(--surface-3); color: var(--text); }
  .bt-leaf { padding-block: 2px; padding-inline: 18px 6px; font-size: var(--fs-xs); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .bt-prof-row { display: flex; align-items: center; gap: 6px; padding-block: 4px; padding-inline: 4px; font-size: var(--fs-xs); color: var(--text); cursor: pointer; }
  .bt-prof-actions { display: flex; flex-wrap: wrap; gap: 4px; padding-block: 4px; }
</style>
