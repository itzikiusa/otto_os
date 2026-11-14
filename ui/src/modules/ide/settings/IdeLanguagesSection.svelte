<script lang="ts">
  // Languages & Frameworks sections (`Go`, `Java`, `Node/TS`, `Python`, `Rust`,
  // `Shell`, `JSON & Schemas`, `CSV Formats`) modeled on GoLand & IntelliJ IDEA.
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LazyMount from '../../../lib/components/LazyMount.svelte';
  import Switch from '../../../lib/components/Switch.svelte';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { ide, type IdeDetectedSdk, type IdeInstalledSdk } from '../ideStore.svelte';
  import { ideSettings } from './ideSettingsStore.svelte';

  const FolderPickerLazy = lazyComponent(() => import('../../../lib/components/FolderPicker.svelte'));

  interface Props { section: string; onnavigate: (id: string) => void }
  let { section, onnavigate }: Props = $props();

  let javaDropOpen = $state(false);
  let pickerEco = $state<string | null>(null);
  let creatingVenv = $state(false);

  const goEco = $derived(ide.toolchains.find((e) => e.ecosystem === 'go') ?? null);
  const javaEco = $derived(ide.toolchains.find((e) => e.ecosystem === 'java') ?? null);
  const nodeEco = $derived(ide.toolchains.find((e) => e.ecosystem === 'node') ?? null);
  const pyEco = $derived(ide.toolchains.find((e) => e.ecosystem === 'python') ?? null);
  const rustEco = $derived(ide.toolchains.find((e) => e.ecosystem === 'rust') ?? null);
  const goDetected = $derived(ide.detectedSdks.filter((d) => d.ecosystem === 'go'));
  const javaDetected = $derived(ide.detectedSdks.filter((d) => d.ecosystem === 'java'));
  const gopathVal = $derived(ide.shellEnv?.vars?.GOPATH || ide.effectiveEnv?.GOPATH || '~/go');
  const goprivateVal = $derived(ide.shellEnv?.vars?.GOPRIVATE || ide.effectiveEnv?.GOPRIVATE || '');
  const isBroken = (s: IdeInstalledSdk): boolean => s.broken === true || s.valid === false;
  const selectedJavaSdkObj = $derived((javaEco?.installed ?? []).find((s) => s.path === ideSettings.envDraft.java_home) ?? null);

  function openGoPlusMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Local…', icon: 'folder', action: () => (pickerEco = 'go') },
      { label: 'Download…', icon: 'download', action: () => onnavigate('project.sdks') },
    ]);
  }

  async function selectDetected(det: IdeDetectedSdk): Promise<void> {
    javaDropOpen = false;
    const p = det.home_path || det.path || '';
    if (!p) return;
    const reg = await ide.addLocalToolchain(det.ecosystem, p);
    const chosen = reg?.path || p;
    if (det.ecosystem === 'java') ideSettings.patchEnv({ java_home: chosen });
    else if (det.ecosystem === 'go') ideSettings.patchEnv({ go_root: chosen });
    toasts.success(`Registered ${det.label || `${det.ecosystem.toUpperCase()} ${det.version}`}`);
  }

  function isGoEnabledInModule(modPath: string): boolean {
    const val = ideSettings.draft.goEnabledModules[modPath];
    return val === undefined ? true : val;
  }

  function toggleGoModule(modPath: string, next: boolean): void {
    ideSettings.draft.goEnabledModules = { ...ideSettings.draft.goEnabledModules, [modPath]: next };
  }

  async function handlePickedSdk(picked: string): Promise<void> {
    const eco = pickerEco;
    pickerEco = null;
    if (!picked || !eco) return;
    const reg = await ide.addLocalToolchain(eco, picked);
    const val = reg?.path || picked;
    if (eco === 'go') ideSettings.patchEnv({ go_root: val });
    else if (eco === 'java') ideSettings.patchEnv({ java_home: val });
    else if (eco === 'node') ideSettings.patchEnv({ node_home: val });
    else if (eco === 'python') ideSettings.patchEnv({ python_interpreter: val });
  }

  async function handleCreateVenv(): Promise<void> {
    creatingVenv = true;
    try {
      const res = await ide.createPythonVenv('.venv');
      if (res?.interpreter_path) ideSettings.patchEnv({ python_interpreter: res.interpreter_path });
    } finally { creatingVenv = false; }
  }
</script>

{#if section === 'lang.go'}
  <div class="lg-sec" data-testid="ide-settings-lang-go">
    <div class="lg-card">
      <div class="lg-head-row">
        <span class="lg-title">GOROOT</span>
        {#if goEco?.lsp_status || goEco?.lsp}
          {@const ok = Boolean(goEco.lsp_status?.available ?? goEco.lsp?.installed)}
          <Badge tone={ok ? 'ok' : 'neutral'} variant="outline" label={ok ? 'gopls active' : 'gopls missing'} />
        {/if}
        {#if goEco?.dap_status || goEco?.dap}
          {@const ok = Boolean(goEco.dap_status?.available ?? goEco.dap?.installed)}
          <Badge tone={ok ? 'ok' : 'neutral'} variant="outline" label={ok ? 'dlv active' : 'dlv missing'} />
        {/if}
      </div>

      <div class="goroot-bar">
        <select
          class="input goroot-sel"
          aria-label="GOROOT toolchain"
          value={ideSettings.envDraft.go_root}
          onchange={(e) => ideSettings.patchEnv({ go_root: (e.currentTarget as HTMLSelectElement).value })}
          data-testid="ide-goroot-select"
        >
          <option value="">&lt;Auto-detect from PATH&gt;</option>
          {#each goEco?.installed ?? [] as sdk (sdk.path)}
            <option value={sdk.path}>Go {sdk.version} ({sdk.path}){isBroken(sdk) ? ' [Invalid]' : ''}</option>
          {/each}
          {#each goDetected as det (`det:${det.home_path || det.path}`)}
            <option value={det.home_path || det.path}>{det.label || `Detected Go ${det.version}`} ({det.home_path || det.path})</option>
          {/each}
        </select>
        <button type="button" class="icon-btn goroot-plus" aria-label="Add or download GOROOT" title="Add Local GOROOT or Download Go SDK" onclick={openGoPlusMenu} data-testid="ide-goroot-add-btn">
          <Icon name="plus" size={14} />
        </button>
      </div>

      <div class="lg-kv-grid">
        <div class="lg-kv"><span class="lg-kv-k">GOPATH</span><code class="lg-kv-v" dir="ltr">{gopathVal}</code></div>
        <div class="lg-kv"><span class="lg-kv-k">GOPRIVATE</span><code class="lg-kv-v" dir="ltr">{goprivateVal || '(not set — reload in Shell Environment)'}</code></div>
      </div>
    </div>

    <div class="lg-card">
      <div class="lg-switch-row">
        <Switch checked={ideSettings.draft.goModulesEnabled} label="Enable Go modules integration" onchange={(v) => (ideSettings.draft.goModulesEnabled = v)} />
        <span class="lg-title">Enable Go modules integration (go.mod / replace directives)</span>
      </div>
      <div class="lg-field">
        <label class="lg-lbl" for="lg-goproxy">Environment / GOPROXY:</label>
        <input id="lg-goproxy" class="input mono" type="text" dir="ltr" bind:value={ideSettings.draft.goProxy} />
      </div>
    </div>

    <div class="lg-card">
      <span class="lg-title">Enable Go support in modules:</span>
      <div class="mod-checklist" role="group" aria-label="Enable Go support in modules" data-testid="ide-go-modules-checklist">
        {#if ide.projectFolders.length === 0}
          <span class="lg-sub">Open a project to configure per-module Go support.</span>
        {:else}
          {#each ide.projectFolders as mod (mod.path)}
            <label class="mod-check-row">
              <input type="checkbox" checked={isGoEnabledInModule(mod.path)} onchange={(e) => toggleGoModule(mod.path, (e.currentTarget as HTMLInputElement).checked)} />
              <Icon name="folder" size={13} />
              <span class="mod-check-name" dir="ltr">{mod.name}</span>
              <span class="mod-check-path" dir="ltr">{mod.path}</span>
            </label>
          {/each}
        {/if}
      </div>
    </div>
  </div>

{:else if section === 'lang.java'}
  <div class="lg-sec" data-testid="ide-settings-lang-java">
    <div class="lg-card">
      <div class="lg-head-row">
        <span class="lg-title">JDK &amp; Classpath</span>
        {#if javaEco?.lsp_status || javaEco?.lsp}
          {@const ok = Boolean(javaEco.lsp_status?.available ?? javaEco.lsp?.installed)}
          <Badge tone={ok ? 'ok' : 'neutral'} variant="outline" label={ok ? 'jdtls active' : 'jdtls missing'} />
        {/if}
      </div>

      <div class="lg-field">
        <span class="lg-lbl" id="java-sdk-lbl">SDK:</span>
        <button
          type="button"
          class="jdk-trigger"
          class:is-broken={Boolean(selectedJavaSdkObj && isBroken(selectedJavaSdkObj))}
          aria-labelledby="java-sdk-lbl"
          aria-expanded={javaDropOpen}
          onclick={() => (javaDropOpen = !javaDropOpen)}
          data-testid="ide-java-sdk-picker"
        >
          {#if ideSettings.envDraft.java_home}
            <span dir="ltr">
              {selectedJavaSdkObj ? `JDK ${selectedJavaSdkObj.version}` : 'Custom JDK'} ({ideSettings.envDraft.java_home})
              {#if selectedJavaSdkObj && isBroken(selectedJavaSdkObj)}[Invalid JDK path]{/if}
            </span>
          {:else}
            <span class="jdk-none">&lt;No SDK&gt;</span>
          {/if}
          <Icon name="chevronDown" size={12} />
        </button>

        {#if javaDropOpen}
          <div class="jdk-menu" role="listbox" aria-label="Java SDK options" data-testid="ide-java-sdk-menu">
            <button type="button" class="jdk-opt is-danger" onclick={() => { ideSettings.patchEnv({ java_home: '' }); javaDropOpen = false; }}>
              <span>&lt;No SDK&gt;</span>
            </button>
            {#each javaEco?.installed ?? [] as sdk (sdk.path)}
              <button type="button" class="jdk-opt" class:is-danger={isBroken(sdk)} onclick={() => { ideSettings.patchEnv({ java_home: sdk.path }); javaDropOpen = false; }}>
                <span class="jdk-main" dir="ltr">JDK {sdk.version}{isBroken(sdk) ? ' [Invalid]' : ''}</span>
                <span class="jdk-sub" dir="ltr">{sdk.path}</span>
              </button>
            {/each}
            <div class="jdk-sep"></div>
            <button type="button" class="jdk-opt" onclick={() => { javaDropOpen = false; onnavigate('project.sdks'); }}><span>↓ Download JDK…</span></button>
            <button type="button" class="jdk-opt" onclick={() => { javaDropOpen = false; pickerEco = 'java'; }}><span>Add JDK from disk…</span></button>

            <div class="jdk-hdr">Detected SDKs</div>
            {#if javaDetected.length === 0}
              <div class="jdk-empty">No unmanaged host JDKs detected outside configured list.</div>
            {:else}
              {#each javaDetected as det (`${det.home_path || det.path}`)}
                <button type="button" class="jdk-opt" onclick={() => void selectDetected(det)} data-testid="ide-detected-jdk-option">
                  <span class="jdk-main" dir="ltr">{det.label || `${det.source || 'OpenJDK'} ${det.version}`}</span>
                  <span class="jdk-sub" dir="ltr">{det.home_path || det.path}</span>
                </button>
              {/each}
            {/if}
          </div>
        {/if}
      </div>

      <div class="lg-field">
        <label class="lg-lbl" for="lg-java-lvl">Project bytecode &amp; language level:</label>
        <select id="lg-java-lvl" class="input" bind:value={ideSettings.draft.javaLanguageLevel}>
          <option value="23">23 — Latest</option>
          <option value="21">21 (LTS)</option>
          <option value="17">17 (LTS)</option>
          <option value="11">11 (LTS)</option>
          <option value="8">8</option>
        </select>
      </div>
      <p class="lg-sub">
        Multi-repo Maven (<code>pom.xml</code>) &amp; Gradle (<code>build.gradle</code>) build pipes share local <code>~/.m2</code> snapshots across attached workspace modules and attach JDWP over an ephemeral loopback port.
      </p>
    </div>
  </div>

{:else if section === 'lang.node'}
  <div class="lg-sec" data-testid="ide-settings-lang-node">
    <div class="lg-card">
      <span class="lg-title">Node.js Runtime &amp; TypeScript Language Server</span>
      <div class="goroot-bar">
        <select class="input goroot-sel" aria-label="Node runtime" value={ideSettings.envDraft.node_home} onchange={(e) => ideSettings.patchEnv({ node_home: (e.currentTarget as HTMLSelectElement).value })}>
          <option value="">&lt;System Node from PATH&gt;</option>
          {#each nodeEco?.installed ?? [] as sdk (sdk.path)}<option value={sdk.path}>Node {sdk.version} ({sdk.path})</option>{/each}
        </select>
        <button type="button" class="btn small ghost" onclick={() => (pickerEco = 'node')}>Browse…</button>
      </div>
      <div class="lg-field">
        <label class="lg-lbl" for="lg-pkg-mgr">Package manager:</label>
        <select id="lg-pkg-mgr" class="input" bind:value={ideSettings.draft.npmPackageManager}>
          <option value="npm">npm</option><option value="pnpm">pnpm</option><option value="yarn">yarn</option><option value="bun">bun</option>
        </select>
      </div>
    </div>
  </div>

{:else if section === 'lang.python'}
  <div class="lg-sec" data-testid="ide-settings-lang-python">
    <div class="lg-card">
      <span class="lg-title">Python Interpreter (.venv / uv / pyenv)</span>
      <div class="goroot-bar">
        <select class="input goroot-sel" aria-label="Python interpreter" value={ideSettings.envDraft.python_interpreter} onchange={(e) => ideSettings.patchEnv({ python_interpreter: (e.currentTarget as HTMLSelectElement).value })}>
          <option value="">&lt;Auto-detect .venv or python3&gt;</option>
          {#each ide.pythonVenvs as v (v.interpreter_path || v.path)}<option value={v.interpreter_path || v.path}>{v.name || `${v.kind} Python ${v.version}`} ({v.interpreter_path || v.path})</option>{/each}
          {#each pyEco?.installed ?? [] as sdk (sdk.path)}<option value={sdk.path}>Python {sdk.version} ({sdk.path})</option>{/each}
        </select>
        <button type="button" class="btn small ghost" onclick={() => (pickerEco = 'python')}>Browse…</button>
        <button type="button" class="btn small" disabled={creatingVenv || !ide.hasTarget} onclick={() => void handleCreateVenv()}>{creatingVenv ? 'Creating…' : 'Create .venv'}</button>
      </div>
    </div>
  </div>

{:else if section === 'lang.rust'}
  <div class="lg-sec" data-testid="ide-settings-lang-rust">
    <div class="lg-card">
      <div class="lg-head-row">
        <span class="lg-title">Rust Toolchain (rustup / rust-analyzer / codelldb)</span>
        {#if rustEco?.lsp_status || rustEco?.lsp}
          {@const ok = Boolean(rustEco.lsp_status?.available ?? rustEco.lsp?.installed)}
          <Badge tone={ok ? 'ok' : 'neutral'} variant="outline" label={ok ? 'rust-analyzer ✓' : 'rust-analyzer missing'} />
        {/if}
        {#if rustEco?.dap_status || rustEco?.dap}
          {@const dap = rustEco.dap_status ?? rustEco.dap!}
          {@const ok = Boolean(dap.available ?? dap.installed)}
          <Badge tone={ok ? 'ok' : 'neutral'} variant="outline" label={ok ? `${dap.name} ✓` : 'lldb missing'} />
        {/if}
      </div>
      <div class="lg-field">
        <label class="lg-lbl" for="lg-rust-tc">Active toolchain channel:</label>
        <select id="lg-rust-tc" class="input" value={ideSettings.envDraft.rust_toolchain} onchange={(e) => ideSettings.patchEnv({ rust_toolchain: (e.currentTarget as HTMLSelectElement).value })}>
          <option value="">Default rustup toolchain (stable)</option>
          {#each rustEco?.installed ?? [] as sdk (sdk.path)}<option value={sdk.version}>{sdk.version} ({sdk.path})</option>{/each}
        </select>
      </div>
      <div class="lg-switch-row">
        <Switch checked={ideSettings.draft.cargoCheckOnSave} label="Run cargo check on save" onchange={(v) => (ideSettings.draft.cargoCheckOnSave = v)} />
        <span class="lg-sub">Run <code>cargo check</code> diagnostics on save</span>
      </div>
    </div>
  </div>

{:else if section === 'lang.shell'}
  <div class="lg-sec">
    <div class="lg-card">
      <label class="lg-lbl" for="lg-sh-dial">Default Shell Dialect:</label>
      <select id="lg-sh-dial" class="input" bind:value={ideSettings.draft.shellDialect}>
        <option value="zsh">zsh (macOS default)</option><option value="bash">bash</option><option value="POSIX sh">POSIX sh</option>
      </select>
    </div>
  </div>

{:else if section === 'lang.json'}
  <div class="lg-sec">
    <div class="lg-card">
      <div class="lg-switch-row">
        <Switch checked={ideSettings.draft.jsonValidateSchemas} label="Validate JSON and JSONL syntax" onchange={(v) => (ideSettings.draft.jsonValidateSchemas = v)} />
        <span class="lg-title">Enable live JSON / JSONL syntax &amp; JSONPath breadcrumbs</span>
      </div>
    </div>
  </div>

{:else}
  <div class="lg-sec">
    <div class="lg-card">
      <div class="lg-field">
        <label class="lg-lbl" for="lg-csv-delim">Default CSV / TSV delimiter:</label>
        <select id="lg-csv-delim" class="input" bind:value={ideSettings.draft.csvDelimiter}>
          <option value="auto">Auto-detect (, / tab / ;)</option><option value=",">Comma (,)</option><option value={'\t'}>Tab (TSV)</option><option value=";">Semicolon (;)</option>
        </select>
      </div>
      <div class="lg-switch-row">
        <Switch checked={ideSettings.draft.csvFirstRowHeader} label="First row is header" onchange={(v) => (ideSettings.draft.csvFirstRowHeader = v)} />
        <span class="lg-sub">Treat first row as sticky table header</span>
      </div>
      <div class="lg-switch-row">
        <Switch checked={ideSettings.draft.csvRainbowColumns} label="Highlight CSV columns" onchange={(v) => (ideSettings.draft.csvRainbowColumns = v)} />
        <span class="lg-sub">Align and tint columns in Text &amp; Table views</span>
      </div>
    </div>
  </div>
{/if}

{#if pickerEco}
  <LazyMount
    lazy={FolderPickerLazy}
    what="SDK folder picker"
    quiet
    props={{ title: `Select ${pickerEco.toUpperCase()} SDK directory`, start: ide.target.kind === 'local' ? ide.target.root : '', onpick: (p: string) => void handlePickedSdk(p), onclose: () => (pickerEco = null) }}
    onerror={() => (pickerEco = null)}
  />
{/if}

<style>
  .lg-sec { display: flex; flex-direction: column; gap: 14px; }
  .lg-card { border: 1px solid var(--border); border-radius: var(--radius-s); background: var(--surface); padding: 12px; display: flex; flex-direction: column; gap: 10px; }
  .lg-head-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .lg-title { font-size: var(--fs-s); font-weight: 600; color: var(--text); }
  .lg-lbl { font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim); }
  .lg-sub { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); line-height: 1.5; }
  .lg-field { display: flex; flex-direction: column; gap: 4px; }
  .lg-switch-row { display: flex; align-items: center; gap: 10px; }
  .goroot-bar { display: flex; align-items: center; gap: 6px; }
  .goroot-sel { flex: 1; min-inline-size: 0; font-family: var(--font-mono); font-size: var(--fs-xs); }
  .goroot-plus { border: 1px solid var(--control-border); border-radius: var(--radius-s); background: var(--surface-2); }
  .mono { font-family: var(--font-mono); font-size: var(--fs-xs); }
  .lg-kv-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 8px; }
  .lg-kv { padding: 6px 8px; border-radius: var(--radius-s); background: var(--surface-2); border: 1px solid var(--border); display: flex; flex-direction: column; gap: 2px; }
  .lg-kv-k { font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim); }
  .lg-kv-v { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text); word-break: break-all; }
  .mod-checklist { max-block-size: 180px; overflow-y: auto; border: 1px solid var(--border); border-radius: var(--radius-s); background: var(--surface-2); padding: 4px; display: flex; flex-direction: column; gap: 2px; }
  .mod-check-row { display: flex; align-items: center; gap: 8px; padding: 6px 8px; border-radius: var(--radius-s); cursor: pointer; font-size: var(--fs-xs); }
  .mod-check-row:hover { background: var(--hover); }
  .mod-check-name { font-weight: 600; color: var(--text); }
  .mod-check-path { font-family: var(--font-mono); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; margin-inline-start: auto; max-inline-size: 260px; }
  .jdk-trigger { display: flex; align-items: center; justify-content: space-between; gap: 8px; inline-size: 100%; padding: 6px 10px; border: 1px solid var(--control-border); border-radius: var(--radius-s); background: var(--surface-2); color: var(--text); font-family: var(--font-mono); font-size: var(--fs-xs); cursor: pointer; text-align: start; }
  .jdk-trigger.is-broken, .jdk-none { color: var(--danger); }
  .jdk-menu { margin-block-start: 4px; max-block-size: 260px; overflow-y: auto; border: 1px solid var(--border-strong); border-radius: var(--radius-s); background: var(--surface); box-shadow: var(--shadow-card); padding: 4px; display: flex; flex-direction: column; gap: 2px; }
  .jdk-hdr { padding: 6px 8px 2px; font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim); }
  .jdk-sep { block-size: 1px; background: var(--border); margin-block: 4px; }
  .jdk-empty { padding: 6px 8px; font-size: var(--fs-xs); color: var(--text-dim); }
  .jdk-opt { display: flex; flex-direction: column; align-items: flex-start; gap: 2px; padding: 6px 8px; border: 0; border-radius: var(--radius-s); background: transparent; color: var(--text); font-size: var(--fs-xs); cursor: pointer; text-align: start; }
  .jdk-opt:hover { background: var(--hover); }
  .jdk-opt.is-danger { color: var(--danger); }
  .jdk-main { font-weight: 500; }
  .jdk-sub { font-family: var(--font-mono); color: var(--text-dim); }
</style>
