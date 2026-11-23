<script lang="ts">
  // Project Structure sections (`Project`, `Modules`, `SDKs` across all 8 ecosystems)
  // for the IntelliJ / GoLand Unified Settings modal.
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LazyMount from '../../../lib/components/LazyMount.svelte';
  import Tabs, { type TabItem } from '../../../lib/components/Tabs.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { plural } from '../../../lib/plural';
  import { toasts } from '../../../lib/toast.svelte';
  import { ide, type IdeDetectedSdk, type IdeInstalledSdk, type IdeToolchainEcosystem } from '../ideStore.svelte';
  import { ideSettings } from './ideSettingsStore.svelte';

  const FolderPickerLazy = lazyComponent(() => import('../../../lib/components/FolderPicker.svelte'));

  interface Props { section: string; onnavigate: (id: string) => void }
  let { section, onnavigate }: Props = $props();

  const ALL_ECOS: readonly { id: string; label: string }[] = [
    { id: 'go', label: 'Go' }, { id: 'java', label: 'Java' }, { id: 'rust', label: 'Rust' }, { id: 'node', label: 'Node / TS' },
    { id: 'python', label: 'Python' }, { id: 'ios', label: 'iOS / Xcode' }, { id: 'android', label: 'Android' },
    { id: 'flutter', label: 'Flutter' }, { id: 'shell', label: 'Shell' }, { id: 'json', label: 'JSON' }, { id: 'csv', label: 'CSV' },
  ];

  let activeEco = $state('go');
  let dlVersions = $state<Record<string, string>>({});
  let folderPickAction = $state<'module' | 'sdk' | null>(null);
  let sdkPickEco = $state('go');
  let busyInstall = $state<string | null>(null);
  let venvCreatingFor = $state<string | null>(null);

  async function createModuleVenv(modPath: string): Promise<void> {
    venvCreatingFor = modPath;
    try {
      const res = await ide.createPythonVenv('.venv', undefined, modPath);
      if (res?.interpreter_path) {
        ideSettings.draft.moduleSdkMap = { ...ideSettings.draft.moduleSdkMap, [modPath]: res.interpreter_path };
        toasts.success(`Python .venv ready: ${res.interpreter_path}`);
      }
    } finally {
      venvCreatingFor = null;
    }
  }

  const platformBadge = $derived(`${ide.targetOs || 'macOS'} · ${ide.targetArch || 'arm64'}${ide.target.kind === 'ssh' ? ` (${ide.target.connection_name || 'SSH'})` : ''}`);
  const allInstalledSdks = $derived(
    ide.toolchains.flatMap((eco) => eco.installed.map((sdk) => ({ eco: eco.ecosystem, ecoName: eco.display_name || eco.ecosystem, sdk }))),
  );
  const activePrimarySdkPath = $derived(
    ideSettings.envDraft.go_root || ideSettings.envDraft.java_home || ideSettings.envDraft.python_interpreter || ideSettings.envDraft.node_home || ideSettings.envDraft.rust_toolchain || '',
  );
  const activePrimarySdkLabel = $derived.by(() => {
    if (!activePrimarySdkPath) return '';
    const hit = allInstalledSdks.find((x) => x.sdk.path === activePrimarySdkPath || x.sdk.version === activePrimarySdkPath);
    return hit ? `${hit.ecoName} ${hit.sdk.version} (${hit.sdk.path})` : activePrimarySdkPath;
  });

  const ecoTabs = $derived<readonly TabItem<string>[]>(
    ALL_ECOS.map((e) => {
      const cnt = ide.toolchains.find((t) => t.ecosystem === e.id)?.installed.length ?? 0;
      return { id: e.id, label: e.label, count: cnt > 0 ? cnt : undefined };
    }),
  );
  const currentEco = $derived<IdeToolchainEcosystem | null>(ide.toolchains.find((e) => e.ecosystem === activeEco) ?? null);
  const currentDetected = $derived<IdeDetectedSdk[]>(ide.detectedSdks.filter((d) => d.ecosystem === activeEco));
  const selectedDlVer = $derived.by(() => {
    const list = currentEco?.downloadable ?? [];
    if (list.length === 0) return '';
    const cur = dlVersions[activeEco];
    if (cur && list.some((d) => d.version === cur)) return cur;
    return (list.find((d) => d.lts) ?? list[0])?.version ?? '';
  });

  const isBroken = (sdk: IdeInstalledSdk): boolean => sdk.broken === true || sdk.valid === false;

  function assignSdkToDraft(ecosystem: string, pathOrVer: string): void {
    if (ecosystem === 'java') ideSettings.patchEnv({ java_home: pathOrVer });
    else if (ecosystem === 'go') ideSettings.patchEnv({ go_root: pathOrVer });
    else if (ecosystem === 'python') ideSettings.patchEnv({ python_interpreter: pathOrVer });
    else if (ecosystem === 'node') ideSettings.patchEnv({ node_home: pathOrVer });
    else if (ecosystem === 'rust') ideSettings.patchEnv({ rust_toolchain: pathOrVer });
    else if (ecosystem === 'ios') ideSettings.patchEnv({ xcode_developer_dir: pathOrVer });
    else if (ecosystem === 'android') ideSettings.patchEnv({ android_home: pathOrVer });
    else if (ecosystem === 'flutter') ideSettings.patchEnv({ flutter_root: pathOrVer });
  }

  async function pickDetectedSdk(det: IdeDetectedSdk): Promise<void> {
    const p = det.home_path || det.path || '';
    if (!p) return;
    const registered = await ide.addLocalToolchain(det.ecosystem, p);
    assignSdkToDraft(det.ecosystem, registered?.path || p);
    toasts.success(`Selected ${det.label || `${det.ecosystem.toUpperCase()} ${det.version}`}`);
  }

  function openProjectSdkMenu(e: MouseEvent): void {
    const anchor = e.currentTarget instanceof HTMLElement ? e.currentTarget : null;
    const items: MenuItem[] = [
      {
        label: 'No SDK',
        checked: !activePrimarySdkPath,
        action: () => ideSettings.patchEnv({ go_root: '', java_home: '', python_interpreter: '', node_home: '', rust_toolchain: '' }),
      },
    ];
    if (allInstalledSdks.length > 0) {
      items.push({ separator: true }, { label: 'Configured SDKs', disabled: true });
      for (const item of allInstalledSdks) {
        const broken = isBroken(item.sdk);
        items.push({
          label: `${item.ecoName} ${item.sdk.version}${broken ? ' [Invalid / Missing]' : ''}`,
          hint: item.sdk.path,
          danger: broken,
          checked: activePrimarySdkPath === item.sdk.path,
          action: () => assignSdkToDraft(item.eco, item.sdk.path),
        });
      }
    }
    items.push(
      { separator: true },
      { label: 'Download SDK…', icon: 'download', action: () => onnavigate('project.sdks') },
      { label: 'Add SDK from disk…', icon: 'folder', action: () => { sdkPickEco = 'go'; folderPickAction = 'sdk'; } },
    );
    if (ide.detectedSdks.length > 0) {
      items.push({ separator: true }, { label: 'Detected SDKs', disabled: true });
      for (const det of ide.detectedSdks) {
        const detPath = det.home_path || det.path;
        items.push({
          label: det.label || `${det.ecosystem.toUpperCase()} ${det.version}`,
          hint: detPath,
          action: () => void pickDetectedSdk(det),
        });
      }
    }
    ctxMenu.showAt(anchor, items, { align: 'start' });
  }

  async function triggerInstall(eco: string, comp: 'sdk' | 'lsp' | 'dap', version?: string): Promise<void> {
    busyInstall = `${eco}:${comp}`;
    try { await ide.installToolchain(eco, comp, version); } finally { busyInstall = null; }
  }

  function inferModuleTech(modPath: string, modName: string): string {
    const entries = ide.childrenByDir[modPath] ?? (modPath === ide.target.root ? ide.treeRootEntries : []);
    const names = new Set(entries.map((e) => e.name));
    if (names.has('go.mod') || modName.startsWith('go_')) return 'Go';
    if (names.has('pom.xml') || names.has('build.gradle') || names.has('build.gradle.kts')) return 'Java';
    if (names.has('Cargo.toml')) return 'Rust';
    if (names.has('package.json')) return 'TS / Node';
    if (names.has('pyproject.toml') || names.has('requirements.txt')) return 'Python';
    return 'Polyglot';
  }

  async function onPickedFolder(picked: string): Promise<void> {
    const act = folderPickAction;
    folderPickAction = null;
    if (!picked) return;
    if (act === 'module') await ide.addProjectFolder(picked);
    else if (act === 'sdk') {
      const reg = await ide.addLocalToolchain(sdkPickEco, picked);
      assignSdkToDraft(sdkPickEco, reg?.path || picked);
    }
  }
</script>

{#if section === 'project.project'}
  <div class="st-sec" data-testid="ide-settings-project">
    <div class="st-field">
      <label class="st-lbl" for="st-proj-name">Name:</label>
      <input id="st-proj-name" class="input" type="text" dir="auto" placeholder="Project name…" bind:value={ideSettings.projectNameDraft} data-testid="ide-settings-project-name" />
    </div>

    <div class="st-field">
      <span class="st-lbl" id="st-proj-sdk-lbl">SDK:</span>
      <div class="sdk-drop-wrap">
        <button type="button" class="sdk-drop-trigger" aria-labelledby="st-proj-sdk-lbl" aria-haspopup="menu" onclick={openProjectSdkMenu} data-testid="ide-project-sdk-picker">
          {#if activePrimarySdkLabel}<span class="sdk-val" dir="ltr">{activePrimarySdkLabel}</span>{:else}<span class="sdk-no-val">No SDK</span>{/if}
          <Icon name="chevronDown" size={12} />
        </button>
      </div>
    </div>
  </div>

{:else if section === 'project.modules'}
  <div class="st-sec" data-testid="ide-settings-modules">
    <div class="st-toolbar">
      <span class="st-count">{plural(ide.projectFolders.length, 'module root')} attached</span>
      <div class="st-toolbar-btns">
        <button type="button" class="btn small ghost" onclick={() => (folderPickAction = 'module')}><Icon name="plus" size={12} />Add Folder…</button>
        <button type="button" class="btn small" onclick={() => (ide.bulkRepoPickerMode = 'attach')} data-testid="ide-settings-bulk-attach"><Icon name="layers" size={12} />Bulk Attach Repos…</button>
      </div>
    </div>

    <div class="mod-list" data-testid="ide-settings-modules-list">
      {#each ide.projectFolders as mod (mod.path)}
        {@const curModSdk = ideSettings.draft.moduleSdkMap[mod.path] ?? ''}
        <div class="mod-row">
          <Icon name="folder" size={14} />
          <div class="mod-copy">
            <div class="mod-title">
              <span class="mod-name" dir="ltr">{mod.name}</span>
              {#if mod.isPrimary}<Badge tone="accent" variant="outline" label="Primary" />{/if}
              <Badge tone="neutral" variant="outline" label={inferModuleTech(mod.path, mod.name)} />
            </div>
            <span class="mod-path" dir="ltr">{mod.path}</span>
          </div>
          <select
            class="input mod-sdk-sel"
            aria-label={`SDK for module ${mod.name}`}
            value={curModSdk}
            onchange={(e) => { ideSettings.draft.moduleSdkMap = { ...ideSettings.draft.moduleSdkMap, [mod.path]: (e.currentTarget as HTMLSelectElement).value }; }}
          >
            <option value="">Project SDK</option>
            {#if curModSdk && !allInstalledSdks.some((s) => s.sdk.path === curModSdk) && !ide.pythonVenvs.some((pv) => (pv.interpreter_path || pv.path) === curModSdk) && !ide.detectedSdks.some((det) => (det.home_path || det.path) === curModSdk)}
              <option value={curModSdk}>{curModSdk}</option>
            {/if}
            {#each allInstalledSdks as s (`${s.eco}:${s.sdk.path}`)}<option value={s.sdk.path}>{s.ecoName} {s.sdk.version}</option>{/each}
            {#each ide.pythonVenvs as pv (`venv:${pv.interpreter_path || pv.path}`)}<option value={pv.interpreter_path || pv.path}>Python {pv.kind} {pv.version} ({pv.path})</option>{/each}
            {#each ide.detectedSdks as det (`det:${det.ecosystem}:${det.home_path || det.path}`)}<option value={det.home_path || det.path}>{det.label || `${det.ecosystem.toUpperCase()} ${det.version}`}</option>{/each}
          </select>
          <button
            type="button"
            class="btn small ghost"
            disabled={venvCreatingFor === mod.path}
            title={`Create Python .venv in ${mod.name}`}
            onclick={() => void createModuleVenv(mod.path)}
            data-testid="ide-module-create-venv"
          >
            {venvCreatingFor === mod.path ? 'Creating…' : '+ .venv'}
          </button>
          {#if !mod.isPrimary}
            <button type="button" class="btn small ghost" onclick={() => void ide.removeProjectFolder(mod.path)}>Remove</button>
          {/if}
        </div>
      {/each}
    </div>
  </div>

{:else}
  <div class="st-sec" data-testid="ide-settings-sdks">
    <div class="st-toolbar">
      <Badge tone="info" variant="outline" label={`Toolchain target · ${platformBadge}`} />
      <button type="button" class="btn small ghost" disabled={ide.toolchainsLoading} onclick={() => void ide.loadToolchains({ includeCatalog: true })}>
        <Icon name="refresh" size={12} />{ide.toolchainsLoading ? 'Scanning…' : 'Refresh'}
      </button>
    </div>

    <Tabs label="SDK ecosystems" tabs={ecoTabs} value={activeEco} onchange={(id) => (activeEco = id)} size="s" />

    {#if activeEco === 'shell' || activeEco === 'json' || activeEco === 'csv'}
      <div class="st-card">
        <div class="st-card-title">{ALL_ECOS.find((x) => x.id === activeEco)?.label} Ecosystem</div>
        <p class="st-help">Built-in fast syntax analysis, formatting, and table/schema inspection are active out of the box with zero external SDK download required.</p>
        {#if activeEco === 'csv'}
          <button type="button" class="btn small" onclick={() => onnavigate('lang.csv')}>Configure CSV preferences…</button>
        {:else if activeEco === 'shell'}
          <button type="button" class="btn small" onclick={() => onnavigate('plugins')}>Manage Shell Language Server…</button>
        {/if}
      </div>
    {:else}
      <div class="st-card">
        <div class="st-toolbar">
          <span class="st-card-title">Installed {activeEco.toUpperCase()} SDKs</span>
          <button type="button" class="btn small ghost" onclick={() => { sdkPickEco = activeEco; folderPickAction = 'sdk'; }}><Icon name="folder" size={12} />Add from disk…</button>
        </div>
        {#if (currentEco?.installed.length ?? 0) === 0}
          <p class="st-help">No {activeEco.toUpperCase()} SDK registered yet — pick a detected SDK or download an official archive below.</p>
        {:else}
          {#each currentEco?.installed ?? [] as sdk (sdk.path)}
            <div class="mod-row">
              <div class="mod-copy">
                <div class="mod-title">
                  <span class="mod-name" dir="ltr">{sdk.version}</span>
                  <Badge tone={isBroken(sdk) ? 'bad' : 'neutral'} variant="outline" label={isBroken(sdk) ? 'Invalid' : sdk.source} />
                </div>
                <span class="mod-path" dir="ltr">{sdk.path}</span>
              </div>
              <button type="button" class="btn small ghost" onclick={() => assignSdkToDraft(activeEco, sdk.path)}>Use in Project</button>
            </div>
          {/each}
        {/if}
      </div>

      {#if currentDetected.length > 0}
        <div class="st-card">
          <span class="st-card-title">Detected SDKs on Host ({currentDetected.length})</span>
          {#each currentDetected as det (`${det.home_path || det.path}`)}
            <div class="mod-row">
              <div class="mod-copy">
                <span class="mod-name" dir="ltr">{det.label || `${det.ecosystem} ${det.version}`}</span>
                <span class="mod-path" dir="ltr">{det.home_path || det.path}</span>
              </div>
              <button type="button" class="btn small" onclick={() => void pickDetectedSdk(det)}>Register</button>
            </div>
          {/each}
        </div>
      {/if}

      <div class="st-card">
        <span class="st-card-title">Download Official Release &amp; Tools</span>
        <div class="dl-row">
          {#if (currentEco?.downloadable.length ?? 0) > 0}
            <select class="input" aria-label="Downloadable SDK version" value={selectedDlVer} onchange={(e) => (dlVersions = { ...dlVersions, [activeEco]: (e.currentTarget as HTMLSelectElement).value })}>
              {#each currentEco?.downloadable ?? [] as rel (rel.version)}<option value={rel.version}>{rel.label || rel.version}{rel.lts ? ' (LTS)' : ''}</option>{/each}
            </select>
            <button type="button" class="btn small primary" disabled={busyInstall !== null} onclick={() => void triggerInstall(activeEco, 'sdk', selectedDlVer)}><Icon name="download" size={12} />Download SDK</button>
          {/if}
          {#if currentEco?.lsp_status || currentEco?.lsp}
            {@const lsp = currentEco.lsp_status ?? currentEco.lsp!}
            {@const lspOk = Boolean(lsp.available ?? lsp.installed)}
            <button type="button" class="btn small ghost" disabled={busyInstall !== null} onclick={() => void triggerInstall(activeEco, 'lsp')}>{lspOk ? `LSP: ${lsp.name} ✓` : `Install ${lsp.name}`}</button>
          {/if}
          {#if currentEco?.dap_status || currentEco?.dap}
            {@const dap = currentEco.dap_status ?? currentEco.dap!}
            {@const dapOk = Boolean(dap.available ?? dap.installed)}
            <button type="button" class="btn small ghost" disabled={busyInstall !== null} onclick={() => void triggerInstall(activeEco, 'dap')}>{dapOk ? `DAP: ${dap.name} ✓` : `Install ${dap.name}`}</button>
          {/if}
        </div>
      </div>
    {/if}
  </div>
{/if}

{#if folderPickAction}
  <LazyMount
    lazy={FolderPickerLazy}
    what="folder picker"
    quiet
    props={{ title: folderPickAction === 'module' ? 'Add module folder to project' : `Select ${sdkPickEco.toUpperCase()} SDK directory`, start: ide.target.kind === 'local' ? ide.target.root : '', onpick: (p: string) => void onPickedFolder(p), onclose: () => (folderPickAction = null) }}
    onerror={() => (folderPickAction = null)}
  />
{/if}

<style>
  .st-sec { display: flex; flex-direction: column; gap: 14px; }
  .st-field { display: flex; flex-direction: column; gap: 4px; }
  .st-lbl { font-size: var(--fs-xs); font-weight: 600; color: var(--text); }
  .st-toolbar { display: flex; align-items: center; justify-content: space-between; gap: 8px; flex-wrap: wrap; }
  .st-toolbar-btns, .dl-row { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .st-count { font-size: var(--fs-xs); color: var(--text-dim); font-weight: 500; }
  .sdk-drop-wrap { position: relative; }
  .sdk-drop-trigger { display: flex; align-items: center; justify-content: space-between; gap: 8px; inline-size: 100%; padding-block: 6px; padding-inline: 10px; border: 1px solid var(--control-border); border-radius: var(--radius-s); background: var(--surface-2); color: var(--text); font-size: var(--fs-s); cursor: pointer; text-align: start; }
  .sdk-no-val { color: var(--text-dim); }
  .sdk-val { font-family: var(--font-mono); font-size: var(--fs-xs); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .mod-list, .st-card { border: 1px solid var(--border); border-radius: var(--radius-s); background: var(--surface); padding: 10px; display: flex; flex-direction: column; gap: 8px; }
  .st-card-title { font-size: var(--fs-s); font-weight: 600; color: var(--text); }
  .st-help { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); line-height: 1.5; }
  .mod-row { display: flex; align-items: center; gap: 10px; padding-block: 6px; padding-inline: 8px; border: 1px solid var(--border); border-radius: var(--radius-s); background: var(--surface-2); }
  .mod-copy { flex: 1; min-inline-size: 0; display: flex; flex-direction: column; gap: 2px; }
  .mod-title { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .mod-name { font-size: var(--fs-s); font-weight: 600; color: var(--text); }
  .mod-path { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .mod-sdk-sel { max-inline-size: 180px; font-size: var(--fs-xs); }
</style>
