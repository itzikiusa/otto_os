<script lang="ts">
  // Environments & Run/Debug Configurations modal for the Otto Full IDE.
  // Tab 1: Login shell env (`~/.zshrc`/`~/.zshenv`), SDKs/venvs, `.env`, PATH, Keychain secrets.
  // Tab 2: GoLand-grade Run/Debug Configurations across Go, Java, Rust, JS/TS, Python, and Shell.
  import { untrack } from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import Switch from '../../lib/components/Switch.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ide, type IdeEnvVar, type IdePythonVenv, type IdeRunConfig } from './ideStore.svelte';
  import IdeNetworkBridgeCard from './shell/IdeNetworkBridgeCard.svelte';
  import IdeRunConfigEditor, { type ConfigDraft, emptyConfigDraft } from './shell/IdeRunConfigEditor.svelte';
  import IdeShellEnvCard from './shell/IdeShellEnvCard.svelte';
  import { confirmProdNetworkBridge } from './shell/networkBridgeGuard.svelte';
  import {
    emptyNetworkBridgeDraft,
    normalizeBridgeDraftSnapshot,
    serializeNetworkBridgeDraft,
    type IdeNetworkBridgeDraft,
  } from './shell/networkBridgeModel';

  interface Props {
    open?: boolean;
    onclose?: () => void;
    initialTab?: 'env' | 'configs';
  }

  let { open, onclose = () => (ide.environmentsOpen = false), initialTab = 'env' }: Props = $props();
  type EnvTab = 'env' | 'configs';

  interface EditableEnvRow {
    rowId: string;
    key: string;
    value: string;
    secret: boolean;
    has_secret: boolean;
    replaceSecret: boolean;
  }

  const isOpen = $derived(open === undefined ? ide.environmentsOpen : open);

  let activeTab = $state<EnvTab>('env');
  let inheritShellEnv = $state(true);
  let javaHome = $state('');
  let goRoot = $state('');
  let goBuildTagsText = $state('');
  let rustToolchain = $state('');
  let nodeHome = $state('');
  let pythonInterpreter = $state('');
  let dotenvPath = $state('');
  let extraPath = $state('');
  let envRows = $state<EditableEnvRow[]>([]);
  let bridgeDraft = $state<IdeNetworkBridgeDraft>(emptyNetworkBridgeDraft(null));
  let envValidationError = $state('');
  let savingEnv = $state(false);
  let creatingVenv = $state(false);

  let isNewDraft = $state(false);
  let cfgDraft = $state<ConfigDraft>(emptyConfigDraft());
  let cfgValidationError = $state('');
  let savingCfg = $state(false);
  let hydratedSnapshot = '';
  let baselineEnvSnap = $state('');
  let baselineCfgSnap = $state('');

  function computeEnvSnap(): string {
    return JSON.stringify({
      inheritShellEnv,
      javaHome: javaHome.trim(),
      goRoot: goRoot.trim(),
      goBuildTagsText: goBuildTagsText.trim(),
      rustToolchain: rustToolchain.trim(),
      nodeHome: nodeHome.trim(),
      pythonInterpreter: pythonInterpreter.trim(),
      dotenvPath: dotenvPath.trim(),
      extraPath: extraPath.trim(),
      bridge: ide.target.kind === 'local' ? normalizeBridgeDraftSnapshot(bridgeDraft) : null,
      envRows: envRows
        .map((r) => ({
          key: r.key.trim(),
          value: r.value,
          secret: r.secret,
          has_secret: r.has_secret,
          replaceSecret: r.replaceSecret,
        }))
        .filter((r) => Boolean(r.key || r.value.trim() || r.has_secret)),
    });
  }

  function computeCfgSnap(draft: ConfigDraft = cfgDraft, newDraft: boolean = isNewDraft): string {
    return JSON.stringify({
      isNewDraft: newDraft,
      id: draft.id,
      name: draft.name.trim(),
      runner: draft.runner,
      target_path: draft.target_path.trim(),
      command: draft.command.trim(),
      test_filter: draft.test_filter.trim(),
      buildFlagsText: draft.buildFlagsText.trim(),
      argsText: draft.argsText.trim(),
      cwd: draft.cwd.trim(),
      env_file: draft.env_file.trim(),
      sdk_path: draft.sdk_path.trim(),
      inherit_shell_env: draft.inherit_shell_env,
      before_launch: draft.before_launch.trim(),
      maven_profiles: draft.maven_profiles,
      spring_profiles: draft.spring_profiles,
      skip_tests: draft.skip_tests,
      offline: draft.offline,
      overrides: draft.overrides
        .map((o) => ({ key: o.key.trim(), value: o.value }))
        .filter((o) => Boolean(o.key || o.value.trim())),
    });
  }

  const currentEnvSnap = $derived(computeEnvSnap());
  const currentCfgSnap = $derived(computeCfgSnap());
  const envDirty = $derived(Boolean(baselineEnvSnap) && currentEnvSnap !== baselineEnvSnap);
  const cfgDirty = $derived(Boolean(baselineCfgSnap) && currentCfgSnap !== baselineCfgSnap);

  function configToDraft(cfg: IdeRunConfig): ConfigDraft {
    return {
      id: cfg.auto_detected ? '' : cfg.id,
      name: cfg.name,
      runner: cfg.runner || 'custom',
      target_path: cfg.target_path ?? '',
      command: cfg.command ?? '',
      test_filter: cfg.test_filter ?? '',
      buildFlagsText: (cfg.build_flags ?? []).join(' '),
      argsText: (cfg.args ?? []).join(' '),
      cwd: cfg.cwd ?? '',
      env_file: cfg.env_file ?? '',
      sdk_path: cfg.sdk_path ?? '',
      inherit_shell_env: cfg.inherit_shell_env !== false,
      before_launch: cfg.before_launch ?? '',
      maven_profiles: [...(cfg.maven_profiles ?? [])],
      spring_profiles: [...(cfg.spring_profiles ?? [])],
      skip_tests: Boolean(cfg.skip_tests),
      offline: Boolean(cfg.offline),
      autoDetected: Boolean(cfg.auto_detected),
      overrides: Object.entries(cfg.env_overrides ?? {}).map(([key, value], idx) => ({
        rowId: `ov-${idx}-${key}`, key, value,
      })),
    };
  }

  function closeForce(): void {
    ide.environmentsOpen = false;
    onclose();
  }

  async function requestClose(): Promise<void> {
    if (savingEnv || savingCfg) return;
    if (envDirty || cfgDirty) {
      const discard = await confirmer.ask(
        'You have unsaved changes in project environment or run configurations. Discard them?',
        {
          title: 'Discard unsaved changes?',
          confirmLabel: 'Discard changes',
          danger: true,
        },
      );
      if (!discard) return;
    }
    closeForce();
  }

  function openToolchainsManager(): void {
    ide.toolchainsOpen = true;
  }

  $effect(() => {
    if (!isOpen) {
      hydratedSnapshot = '';
      baselineEnvSnap = '';
      baselineCfgSnap = '';
      return;
    }
    activeTab = initialTab;
    untrack(() => {
      if (ide.hasTarget) {
        void ide.loadEnvironments();
        if (ide.toolchains.length === 0 && !ide.toolchainsLoading) {
          void ide.loadToolchains({ includeCatalog: false });
        }
      }
    });
  });

  $effect(() => {
    if (!isOpen || !ide.projectEnv) return;
    const snap = JSON.stringify({ env: ide.projectEnv, cfgs: ide.runConfigs.map((c) => c.id) });
    if (snap === hydratedSnapshot) return;
    hydratedSnapshot = snap;
    const e = ide.projectEnv;
    inheritShellEnv = e.inherit_shell_env !== false;
    javaHome = e.java_home ?? '';
    goRoot = e.go_root ?? '';
    goBuildTagsText = (e.go_build_tags ?? []).join(', ');
    rustToolchain = e.rust_toolchain ?? '';
    nodeHome = e.node_home ?? '';
    pythonInterpreter = e.python_interpreter ?? '';
    dotenvPath = e.dotenv_path ?? '';
    extraPath = e.extra_path ?? '';
    bridgeDraft = emptyNetworkBridgeDraft(e.network_bridge);
    envRows = (e.env_vars ?? []).map((v, idx) => ({
      rowId: `ev-${idx}-${v.key}`,
      key: v.key,
      value: v.secret ? '' : (v.value ?? ''),
      secret: Boolean(v.secret),
      has_secret: Boolean(v.has_secret),
      replaceSecret: false,
    }));
    const selected = ide.runConfigs.find((c) => c.id === ide.selectedConfigId) ?? ide.runConfigs[0] ?? null;
    if (selected) {
      isNewDraft = Boolean(selected.auto_detected);
      cfgDraft = configToDraft(selected);
    } else {
      isNewDraft = true;
      cfgDraft = emptyConfigDraft();
    }
    baselineEnvSnap = computeEnvSnap();
    baselineCfgSnap = computeCfgSnap(cfgDraft, isNewDraft);
  });

  const modalTabs = $derived<readonly TabItem<EnvTab>[]>([
    { id: 'env', label: 'Project environment', dirty: envDirty },
    {
      id: 'configs',
      label: 'Run / debug configurations',
      count: ide.runConfigs.length > 0 ? ide.runConfigs.length : undefined,
      dirty: cfgDirty,
    },
  ]);

  function sdkOptionsFor(eco: string): { label: string; value: string }[] {
    const found = ide.toolchains.find((t) => t.ecosystem === eco);
    return (found?.installed ?? []).map((sdk) => ({
      label: `${sdk.version} (${sdk.source})`,
      value: eco === 'rust' ? sdk.version : sdk.path,
    }));
  }

  function venvInterpreterValue(v: IdePythonVenv): string { return v.interpreter_path || v.path; }

  function addEnvRow(secret: boolean): void {
    envRows = [...envRows, { rowId: `ev-new-${Date.now()}-${envRows.length}`, key: '', value: '', secret, has_secret: false, replaceSecret: secret }];
  }

  function importPastedEnvRows(items: { key: string; value: string }[]): void {
    const byKey = new Map(envRows.map((r) => [r.key.trim(), r]));
    for (let i = 0; i < items.length; i++) {
      const { key, value } = items[i];
      const existing = byKey.get(key);
      if (existing) {
        existing.value = value;
        if (existing.secret) existing.replaceSecret = true;
      } else {
        byKey.set(key, { rowId: `ev-paste-${Date.now()}-${i}`, key, value, secret: false, has_secret: false, replaceSecret: false });
      }
    }
    envRows = Array.from(byKey.values());
  }

  function removeEnvRow(rowId: string): void { envRows = envRows.filter((r) => r.rowId !== rowId); }

  function toggleRowSecret(idx: number, nextSecret: boolean): void {
    envRows = envRows.map((row, i) =>
      i === idx ? { ...row, secret: nextSecret, value: nextSecret ? '' : row.value, replaceSecret: nextSecret && !row.has_secret } : row,
    );
  }

  async function handleCreateVenv(): Promise<void> {
    creatingVenv = true;
    try {
      const created = await ide.createPythonVenv('.venv');
      if (created) pythonInterpreter = created.interpreter_path || created.path;
    } finally { creatingVenv = false; }
  }

  async function handleSaveEnvironment(): Promise<boolean> {
    envValidationError = '';
    const cleaned: IdeEnvVar[] = [];
    for (let i = 0; i < envRows.length; i++) {
      const row = envRows[i];
      const key = row.key.trim();
      if (!key && !row.value.trim() && !row.has_secret) continue;
      if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) {
        envValidationError = `Invalid variable name "${key || '(empty)'}" — use letters, digits, and underscores.`;
        return false;
      }
      if (row.secret) {
        if (row.has_secret && !row.replaceSecret && !row.value) cleaned.push({ key, secret: true, has_secret: true });
        else cleaned.push({ key, value: row.value, secret: true, has_secret: Boolean(row.value) });
      } else {
        cleaned.push({ key, value: row.value, secret: false });
      }
    }

    const serializedBridge = serializeNetworkBridgeDraft(bridgeDraft);
    if (serializedBridge.error) {
      envValidationError = serializedBridge.error;
      return false;
    }

    let prodConfirmed = false;
    if (ide.target.kind === 'local' && serializedBridge.config.enabled) {
      const allowed = await confirmProdNetworkBridge(
        serializedBridge.config,
        'save',
        ide.target.workspace_id,
      );
      if (!allowed) return false;
      prodConfirmed = true;
    }

    const parsedGoTags = goBuildTagsText
      .split(/[,\s]+/)
      .map((t) => t.trim())
      .filter((t) => /^[A-Za-z0-9_.-]+$/.test(t));

    savingEnv = true;
    try {
      const ok = await ide.saveEnvironment({
        inherit_shell_env: inheritShellEnv,
        java_home: javaHome.trim() || null,
        go_root: goRoot.trim() || null,
        go_build_tags: parsedGoTags,
        rust_toolchain: rustToolchain.trim() || null,
        node_home: nodeHome.trim() || null,
        python_interpreter: pythonInterpreter.trim() || null,
        dotenv_path: dotenvPath.trim() || null,
        extra_path: extraPath.trim() || null,
        network_bridge: ide.target.kind === 'local' ? serializedBridge.config : undefined,
        env_vars: cleaned,
        ...(prodConfirmed ? { confirm_prod_targets: true } : {}),
      });
      if (ok) {
        if (typeof window !== 'undefined') {
          window.dispatchEvent(new CustomEvent('otto:ide-reconnect-lsp', { detail: { rootPath: ide.target.root } }));
        }
        hydratedSnapshot = '';
        await ide.loadEnvironments();
        baselineEnvSnap = computeEnvSnap();
      }
      return ok;
    } finally {
      savingEnv = false;
    }
  }

  function selectConfig(cfg: IdeRunConfig): void {
    cfgValidationError = '';
    ide.selectedConfigId = cfg.id;
    isNewDraft = Boolean(cfg.auto_detected);
    cfgDraft = configToDraft(cfg);
    baselineCfgSnap = computeCfgSnap(cfgDraft, isNewDraft);
  }

  function startNewConfig(): void {
    cfgValidationError = '';
    isNewDraft = true;
    cfgDraft = emptyConfigDraft();
    baselineCfgSnap = computeCfgSnap(cfgDraft, isNewDraft);
  }

  function duplicateConfig(): void {
    cfgValidationError = '';
    isNewDraft = true;
    cfgDraft = { ...cfgDraft, id: '', name: `${cfgDraft.name || 'Configuration'} (copy)`, autoDetected: false };
  }

  function parseArgs(raw: string): string[] {
    const tokens = raw.match(/(?:[^\s"']+|"[^"]*"|'[^']*')+/g) ?? [];
    return tokens.map((t) => t.replace(/^['"]|['"]$/g, ''));
  }

  async function saveCurrentConfig(afterAction: 'none' | 'run' | 'debug' = 'none'): Promise<boolean> {
    cfgValidationError = '';
    const name = cfgDraft.name.trim();
    if (!name) { cfgValidationError = 'Enter a configuration name before saving.'; return false; }
    const overrides: Record<string, string> = {};
    for (let i = 0; i < cfgDraft.overrides.length; i++) {
      const row = cfgDraft.overrides[i];
      const k = row.key.trim();
      if (!k && !row.value.trim()) continue;
      if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(k)) { cfgValidationError = `Invalid override variable "${k || '(empty)'}".`; return false; }
      overrides[k] = row.value;
    }

    const hadEnvChanges = envDirty;
    if (hadEnvChanges) {
      const envOk = await handleSaveEnvironment();
      if (!envOk) {
        if (envValidationError) activeTab = 'env';
        return false;
      }
    }

    savingCfg = true;
    try {
      const saved = await ide.saveRunConfig({
        id: isNewDraft || cfgDraft.autoDetected ? undefined : cfgDraft.id || undefined,
        name,
        runner: cfgDraft.runner,
        target_path: cfgDraft.target_path.trim() || null,
        command: cfgDraft.command.trim() || null,
        test_filter: cfgDraft.test_filter.trim() || null,
        build_flags: parseArgs(cfgDraft.buildFlagsText),
        args: parseArgs(cfgDraft.argsText),
        cwd: cfgDraft.cwd.trim() || null,
        env_file: cfgDraft.env_file.trim() || null,
        sdk_path: cfgDraft.sdk_path.trim() || null,
        inherit_shell_env: cfgDraft.inherit_shell_env,
        before_launch: cfgDraft.before_launch.trim() || null,
        maven_profiles: cfgDraft.maven_profiles,
        spring_profiles: cfgDraft.spring_profiles,
        skip_tests: cfgDraft.skip_tests,
        offline: cfgDraft.offline,
        env_overrides: overrides,
      });
      if (!saved) return false;
      isNewDraft = false;
      cfgDraft = configToDraft(saved);
      baselineCfgSnap = computeCfgSnap(cfgDraft, isNewDraft);
      toasts.success('Saved run configuration', saved.name);
      if (afterAction === 'run') {
        await ide.startRun({ configId: saved.id, skipProdConfirm: hadEnvChanges });
        closeForce();
      } else if (afterAction === 'debug') {
        await ide.startDebug({ configId: saved.id, skipProdConfirm: hadEnvChanges });
        closeForce();
      }
      return true;
    } finally {
      savingCfg = false;
    }
  }

  async function confirmDeleteConfig(): Promise<void> {
    if (!cfgDraft.id || isNewDraft || cfgDraft.autoDetected) return;
    const targetName = cfgDraft.name || cfgDraft.id;
    const confirmed = await confirmer.ask(`Delete "${targetName}" from saved run / debug configurations?`, {
      title: 'Delete run configuration?',
      confirmLabel: 'Delete configuration',
      danger: true,
    });
    if (!confirmed) return;
    await ide.deleteRunConfig(cfgDraft.id);
    const next = ide.runConfigs[0];
    if (next) selectConfig(next);
    else startNewConfig();
  }
</script>

{#if isOpen}
  <Modal title="Environments & run configurations" width={820} dismissable={!savingEnv && !savingCfg} onclose={() => void requestClose()}>
    <div class="env-shell">
      {#if !ide.hasTarget}
        <EmptyState
          icon="folder"
          title="Open a workspace first"
          body="Select a local folder or SSH target before configuring SDK paths, shell env, Keychain secrets, and run configurations."
          actionLabel="Choose target…"
          actionKind="secondary"
          onaction={() => { closeForce(); ide.sshOpen = true; }}
        />
      {:else}
        <Tabs label="Environment and configuration tabs" tabs={modalTabs} value={activeTab} onchange={(id) => (activeTab = id)} idBase="ide-env" />

        <LoadState what="project environment" loading={ide.envLoading} error={ide.envError} empty={false} onretry={() => ide.loadEnvironments()}>
          {#if activeTab === 'env'}
            <div class="env-pane" role="tabpanel" id="ide-env-panel-env" aria-labelledby="ide-env-tab-env">
              <IdeShellEnvCard {inheritShellEnv} onchangeInherit={(next) => (inheritShellEnv = next)} onpasteVars={importPastedEnvRows} />
              {#if ide.target.kind === 'local'}
                <IdeNetworkBridgeCard bind:draft={bridgeDraft} idPrefix="ide-env-nb" />
              {/if}

              <div class="env-sec-bar">
                <span class="env-sec-title">SDKs &amp; language runtimes</span>
                <button type="button" class="btn small ghost" onclick={openToolchainsManager} data-testid="ide-env-open-toolchains">
                  <Icon name="download" size={12} />
                  Download / Manage SDKs…
                </button>
              </div>

              <div class="env-sdk-grid">
                <div class="field">
                  <label for="env-go-root">Go SDK (GOROOT)</label>
                  <input id="env-go-root" class="input" type="text" dir="ltr" list="env-go-datalist" placeholder="System default go" bind:value={goRoot} />
                  <datalist id="env-go-datalist">
                    {#each sdkOptionsFor('go') as opt (opt.value)}<option value={opt.value}>{opt.label}</option>{/each}
                  </datalist>
                </div>

                <div class="field">
                  <label for="env-go-tags">Go build tags (-tags)</label>
                  <input id="env-go-tags" class="input" type="text" dir="ltr" placeholder="component, integration" bind:value={goBuildTagsText} data-testid="ide-env-go-build-tags" />
                </div>

                <div class="field">
                  <label for="env-java-home">Active JDK (JAVA_HOME)</label>
                  <input id="env-java-home" class="input" type="text" dir="ltr" list="env-java-datalist" placeholder="System default JDK" bind:value={javaHome} />
                  <datalist id="env-java-datalist">
                    {#each sdkOptionsFor('java') as opt (opt.value)}<option value={opt.value}>{opt.label}</option>{/each}
                  </datalist>
                </div>

                <div class="field">
                  <label for="env-rust-tc">Rust toolchain</label>
                  <input id="env-rust-tc" class="input" type="text" dir="ltr" list="env-rust-datalist" placeholder="stable" bind:value={rustToolchain} />
                  <datalist id="env-rust-datalist">
                    <option value="stable">stable</option>
                    <option value="nightly">nightly</option>
                    {#each sdkOptionsFor('rust') as opt (opt.value)}<option value={opt.value}>{opt.label}</option>{/each}
                  </datalist>
                </div>

                <div class="field">
                  <label for="env-node-home">Node.js runtime</label>
                  <input id="env-node-home" class="input" type="text" dir="ltr" list="env-node-datalist" placeholder="System default node" bind:value={nodeHome} />
                  <datalist id="env-node-datalist">
                    {#each sdkOptionsFor('node') as opt (opt.value)}<option value={opt.value}>{opt.label}</option>{/each}
                  </datalist>
                </div>
              </div>

              <div class="field">
                <label for="env-python-bin">Python interpreter / virtualenv</label>
                <div class="env-inline-bar">
                  <input id="env-python-bin" class="input" type="text" dir="ltr" list="env-py-datalist" placeholder=".venv/bin/python or python3" bind:value={pythonInterpreter} />
                  <button type="button" class="btn" disabled={creatingVenv} onclick={handleCreateVenv}>
                    <Icon name="plus" size={12} />
                    {creatingVenv ? 'Creating .venv…' : 'Create .venv'}
                  </button>
                </div>
                <datalist id="env-py-datalist">
                  {#each ide.pythonVenvs as v (v.path)}<option value={venvInterpreterValue(v)}>{v.kind} · {v.version} ({v.path})</option>{/each}
                  {#each sdkOptionsFor('python') as opt (opt.value)}<option value={opt.value}>{opt.label}</option>{/each}
                </datalist>
                {#if ide.pythonVenvs.length > 0}
                  <div class="env-chip-row">
                    {#each ide.pythonVenvs as venv (venv.path)}
                      {@const val = venvInterpreterValue(venv)}
                      <button type="button" class="chip" aria-pressed={pythonInterpreter === val} onclick={() => (pythonInterpreter = val)}>
                        {venv.kind}: {venv.path} ({venv.version})
                      </button>
                    {/each}
                  </div>
                {/if}
              </div>

              <div class="env-sdk-grid">
                <div class="field">
                  <label for="env-dotenv-path">.env file</label>
                  <input id="env-dotenv-path" class="input" type="text" dir="ltr" list="env-dotenv-datalist" placeholder=".env" bind:value={dotenvPath} />
                  <datalist id="env-dotenv-datalist">
                    {#each ide.dotenvFiles as df (df)}<option value={df}>{df}</option>{/each}
                  </datalist>
                  {#if ide.dotenvFiles.length > 0}
                    <div class="env-chip-row">
                      {#each ide.dotenvFiles as df (df)}
                        <button type="button" class="chip" aria-pressed={dotenvPath === df} onclick={() => (dotenvPath = df)}>{df}</button>
                      {/each}
                    </div>
                  {/if}
                </div>

                <div class="field">
                  <label for="env-extra-path">Custom PATH entries</label>
                  <input id="env-extra-path" class="input" type="text" dir="ltr" placeholder="/opt/homebrew/bin:$HOME/go/bin" bind:value={extraPath} />
                </div>
              </div>

              <section class="env-vars-box" aria-labelledby="env-vars-heading">
                <div class="env-vars-head">
                  <div>
                    <h3 id="env-vars-heading" class="env-sec-title">Environment variables &amp; Keychain secrets</h3>
                    <p class="env-sec-sub">Secret values live in macOS Keychain and are injected into runs and debuggers without echoing plaintext.</p>
                  </div>
                  <div class="env-vars-btns">
                    <button type="button" class="btn small" onclick={() => addEnvRow(false)}>
                      <Icon name="plus" size={12} />
                      Add variable
                    </button>
                    <button type="button" class="btn small" onclick={() => addEnvRow(true)}>
                      <Icon name="lock" size={12} />
                      Add Keychain secret
                    </button>
                  </div>
                </div>

                {#if envRows.length === 0}
                  <div class="env-empty-note">No project-specific environment variables configured yet.</div>
                {:else}
                  <div class="env-var-list">
                    {#each envRows as row, idx (row.rowId)}
                      <div class="env-var-item">
                        <input class="input env-key-in" type="text" dir="ltr" aria-label="Environment variable name" placeholder="VARIABLE_NAME" bind:value={row.key} />
                        {#if row.secret && row.has_secret && !row.replaceSecret}
                          <div class="env-secret-mask">
                            <Badge tone="ok" label="Stored in Keychain" />
                            <span class="env-dots" aria-hidden="true">••••••••••••</span>
                            <button type="button" class="btn small ghost" onclick={() => (row.replaceSecret = true)}>Replace…</button>
                          </div>
                        {:else if row.secret}
                          <input class="input env-val-in" type="password" dir="ltr" autocomplete="off" aria-label="Secret value for {row.key || 'variable'}" placeholder="Secret stored in macOS Keychain" bind:value={row.value} />
                        {:else}
                          <input class="input env-val-in" type="text" dir="ltr" aria-label="Value for {row.key || 'variable'}" placeholder="Value" bind:value={row.value} />
                        {/if}
                        <div class="env-secret-toggle">
                          <Switch checked={row.secret} label="Store {row.key || 'variable'} in Keychain" onchange={(next) => toggleRowSecret(idx, next)} />
                          <span class="env-sec-tag">{row.secret ? 'Keychain' : 'Plain'}</span>
                        </div>
                        <button type="button" class="icon-btn" aria-label="Remove environment variable" title="Remove environment variable" onclick={() => removeEnvRow(row.rowId)}>
                          <Icon name="trash" size={13} />
                        </button>
                      </div>
                    {/each}
                  </div>
                {/if}

                {#if envValidationError}
                  <div class="env-err" use:announceShown={{ text: envValidationError, assertive: true }}>{envValidationError}</div>
                {/if}
              </section>
            </div>
          {:else}
            <IdeRunConfigEditor
              bind:cfgDraft
              {isNewDraft}
              {dotenvPath}
              {cfgValidationError}
              onselect={selectConfig}
              onnew={startNewConfig}
              onduplicate={duplicateConfig}
              onopentoolchains={openToolchainsManager}
            />
          {/if}
        </LoadState>
      {/if}
    </div>

    {#snippet footer()}
      {#if activeTab === 'configs' && ide.hasTarget}
        <div class="env-footer-bar">
          <div>
            {#if !isNewDraft && cfgDraft.id && !cfgDraft.autoDetected}
              <button type="button" class="btn danger" disabled={savingCfg} onclick={confirmDeleteConfig}>
                <Icon name="trash" size={13} />
                Delete…
              </button>
            {/if}
          </div>
          <div class="env-footer-end">
            <button type="button" class="btn ghost" disabled={savingCfg} onclick={() => void requestClose()}>Cancel</button>
            <button type="button" class="btn" disabled={savingCfg || savingEnv} onclick={() => void saveCurrentConfig('debug')} data-testid="ide-cfg-save-debug">
              <Icon name="bug" size={12} />
              Save &amp; debug
            </button>
            <button type="button" class="btn" disabled={savingCfg || savingEnv} onclick={() => void saveCurrentConfig('run')}>
              <Icon name="play" size={12} />
              Save &amp; run
            </button>
            <button type="button" class="btn primary" disabled={savingCfg || savingEnv} onclick={() => void saveCurrentConfig('none')}>
              {savingCfg || savingEnv ? 'Saving…' : 'Save configuration'}
            </button>
          </div>
        </div>
      {:else}
        <button type="button" class="btn ghost" disabled={savingEnv} onclick={() => void requestClose()}>Cancel</button>
        {#if ide.hasTarget}
          <button type="button" class="btn primary" disabled={savingEnv} onclick={() => void handleSaveEnvironment()}>
            {savingEnv ? 'Saving…' : 'Save environment'}
          </button>
        {/if}
      {/if}
    {/snippet}
  </Modal>
{/if}

<style>
  .env-shell { display: flex; flex-direction: column; gap: 12px; min-inline-size: 0; }
  .env-pane { display: flex; flex-direction: column; gap: 8px; padding-block-start: 4px; min-inline-size: 0; }
  .env-sec-bar { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding-block-start: 4px; }
  .env-sdk-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; }
  .env-inline-bar { display: flex; align-items: center; gap: 8px; }
  .env-inline-bar input { flex: 1; min-inline-size: 0; }
  .env-chip-row { display: flex; flex-wrap: wrap; gap: 6px; margin-block-start: 4px; }
  .env-vars-box {
    display: flex; flex-direction: column; gap: 8px; padding: 12px;
    border-radius: var(--radius-m); background: var(--surface-2); border: 1px solid var(--border);
  }
  .env-vars-head { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px; }
  .env-vars-btns { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .env-sec-title { margin: 0; font-size: var(--fs-s); font-weight: 600; color: var(--text); }
  .env-sec-sub, .env-empty-note { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); }
  .env-var-list { display: flex; flex-direction: column; gap: 6px; }
  .env-var-item { display: flex; align-items: center; gap: 8px; min-inline-size: 0; }
  .env-key-in { flex: 0 1 180px; min-inline-size: 120px; font-family: var(--font-mono); }
  .env-val-in { flex: 1; min-inline-size: 0; }
  .env-secret-mask {
    display: flex; align-items: center; gap: 8px; flex: 1; min-inline-size: 0;
    padding-inline: 8px; block-size: 28px; border-radius: var(--radius-s);
    background: var(--surface); border: 1px solid var(--border);
  }
  .env-dots { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); margin-inline-end: auto; }
  .env-secret-toggle { display: flex; align-items: center; gap: 6px; flex-shrink: 0; }
  .env-sec-tag { font-size: var(--fs-xs); color: var(--text-dim); min-inline-size: 48px; }
  .env-err { font-size: var(--fs-xs); color: var(--danger); }
  .env-footer-bar { display: flex; align-items: center; justify-content: space-between; gap: 12px; inline-size: 100%; }
  .env-footer-end { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }

  @media (max-width: 640px) {
    .env-sdk-grid { grid-template-columns: 1fr; }
    .env-var-item { flex-wrap: wrap; }
  }
</style>
