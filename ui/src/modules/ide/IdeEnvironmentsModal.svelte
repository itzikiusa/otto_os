<script lang="ts">
  // Environments & Run/Debug Configurations modal for the Otto Full IDE.
  // Tab 1: Active SDKs/virtualenvs (with 1-click `.venv` creation), `.env` file, custom PATH,
  //        and Environment Variables with macOS Keychain secret storage (never echoed).
  // Tab 2: Saved Run/Debug Configurations across Go, Java, Rust, JS/TS, Python, and Shell.
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

  interface Props {
    open?: boolean;
    onclose?: () => void;
    initialTab?: 'env' | 'configs';
  }

  let { open, onclose, initialTab = 'env' }: Props = $props();
  type EnvTab = 'env' | 'configs';

  interface EditableEnvRow {
    rowId: string;
    key: string;
    value: string;
    secret: boolean;
    has_secret: boolean;
    replaceSecret: boolean;
  }

  interface ConfigDraft {
    id: string;
    name: string;
    runner: string;
    target_path: string;
    command: string;
    test_filter: string;
    argsText: string;
    cwd: string;
    before_launch: string;
    overrides: { rowId: string; key: string; value: string }[];
  }

  const RUNNER_OPTIONS: readonly { value: string; label: string; short: string }[] = [
    { value: 'go_test', label: 'Go · go test', short: 'go test' },
    { value: 'go_run', label: 'Go · go run', short: 'go run' },
    { value: 'java_junit', label: 'Java · JUnit', short: 'JUnit' },
    { value: 'java_main', label: 'Java · Java Main', short: 'Java' },
    { value: 'maven', label: 'Java · Maven', short: 'Maven' },
    { value: 'gradle', label: 'Java · Gradle', short: 'Gradle' },
    { value: 'cargo_test', label: 'Rust · cargo test', short: 'cargo test' },
    { value: 'cargo_run', label: 'Rust · cargo run', short: 'cargo run' },
    { value: 'node', label: 'JS/TS · Node file', short: 'Node' },
    { value: 'npm', label: 'JS/TS · npm script', short: 'npm' },
    { value: 'vitest', label: 'JS/TS · Vitest', short: 'Vitest' },
    { value: 'pytest', label: 'Python · pytest', short: 'pytest' },
    { value: 'python', label: 'Python · Python file', short: 'Python' },
    { value: 'custom', label: 'Shell · Custom command', short: 'Shell' },
  ];

  const isOpen = $derived(open === undefined ? ide.environmentsOpen : open);

  let activeTab = $state<EnvTab>('env');
  let javaHome = $state('');
  let goRoot = $state('');
  let rustToolchain = $state('');
  let nodeHome = $state('');
  let pythonInterpreter = $state('');
  let dotenvPath = $state('');
  let extraPath = $state('');
  let envRows = $state<EditableEnvRow[]>([]);
  let envValidationError = $state('');
  let savingEnv = $state(false);
  let creatingVenv = $state(false);

  let isNewDraft = $state(false);
  let cfgDraft = $state<ConfigDraft>(emptyConfigDraft());
  let cfgValidationError = $state('');
  let savingCfg = $state(false);
  let hydratedSnapshot = '';

  function emptyConfigDraft(): ConfigDraft {
    return {
      id: '', name: '', runner: 'go_test', target_path: '', command: '',
      test_filter: '', argsText: '', cwd: '', before_launch: '', overrides: [],
    };
  }

  function configToDraft(cfg: IdeRunConfig): ConfigDraft {
    return {
      id: cfg.id,
      name: cfg.name,
      runner: cfg.runner || 'custom',
      target_path: cfg.target_path ?? '',
      command: cfg.command ?? '',
      test_filter: cfg.test_filter ?? '',
      argsText: (cfg.args ?? []).join(' '),
      cwd: cfg.cwd ?? '',
      before_launch: cfg.before_launch ?? '',
      overrides: Object.entries(cfg.env_overrides ?? {}).map(([key, value], idx) => ({
        rowId: `ov-${idx}-${key}`, key, value,
      })),
    };
  }

  function close(): void {
    ide.environmentsOpen = false;
    onclose?.();
  }

  $effect(() => {
    if (!isOpen) { hydratedSnapshot = ''; return; }
    activeTab = initialTab;
    if (ide.hasTarget) {
      void ide.loadEnvironments();
      if (ide.toolchains.length === 0 && !ide.toolchainsLoading) void ide.loadToolchains();
    }
  });

  $effect(() => {
    if (!isOpen || !ide.projectEnv) return;
    const snap = JSON.stringify({ env: ide.projectEnv, cfgs: ide.runConfigs.map((c) => c.id) });
    if (snap === hydratedSnapshot) return;
    hydratedSnapshot = snap;
    const e = ide.projectEnv;
    javaHome = e.java_home ?? '';
    goRoot = e.go_root ?? '';
    rustToolchain = e.rust_toolchain ?? '';
    nodeHome = e.node_home ?? '';
    pythonInterpreter = e.python_interpreter ?? '';
    dotenvPath = e.dotenv_path ?? '';
    extraPath = e.extra_path ?? '';
    envRows = (e.env_vars ?? []).map((v, idx) => ({
      rowId: `ev-${idx}-${v.key}`,
      key: v.key,
      value: v.secret ? '' : (v.value ?? ''),
      secret: Boolean(v.secret),
      has_secret: Boolean(v.has_secret),
      replaceSecret: false,
    }));
    const selected = ide.runConfigs.find((c) => c.id === ide.selectedConfigId) ?? ide.runConfigs[0] ?? null;
    if (selected) { isNewDraft = false; cfgDraft = configToDraft(selected); }
    else { isNewDraft = true; cfgDraft = emptyConfigDraft(); }
  });

  const modalTabs = $derived<readonly TabItem<EnvTab>[]>([
    { id: 'env', label: 'Project environment' },
    { id: 'configs', label: 'Run / debug configurations', count: ide.runConfigs.length > 0 ? ide.runConfigs.length : undefined },
  ]);

  function sdkOptionsFor(eco: string): { label: string; value: string }[] {
    const found = ide.toolchains.find((t) => t.ecosystem === eco);
    return (found?.installed ?? []).map((sdk) => ({
      label: `${sdk.version} (${sdk.source})`,
      value: eco === 'rust' ? sdk.version : sdk.path,
    }));
  }

  function venvInterpreterValue(v: IdePythonVenv): string { return v.interpreter_path || v.path; }
  function runnerShortLabel(runner: string): string { return RUNNER_OPTIONS.find((r) => r.value === runner)?.short ?? runner; }

  function addEnvRow(secret: boolean): void {
    envRows = [
      ...envRows,
      { rowId: `ev-new-${Date.now()}-${envRows.length}`, key: '', value: '', secret, has_secret: false, replaceSecret: secret },
    ];
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

  async function handleSaveEnvironment(): Promise<void> {
    envValidationError = '';
    const cleaned: IdeEnvVar[] = [];
    for (let i = 0; i < envRows.length; i++) {
      const row = envRows[i];
      const key = row.key.trim();
      if (!key && !row.value.trim() && !row.has_secret) continue;
      if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) {
        envValidationError = `Invalid variable name "${key || '(empty)'}" — use letters, digits, and underscores.`;
        return;
      }
      if (row.secret) {
        if (row.has_secret && !row.replaceSecret && !row.value) cleaned.push({ key, secret: true, has_secret: true });
        else cleaned.push({ key, value: row.value, secret: true, has_secret: Boolean(row.value) });
      } else {
        cleaned.push({ key, value: row.value, secret: false });
      }
    }

    savingEnv = true;
    try {
      const ok = await ide.saveEnvironment({
        java_home: javaHome.trim() || null,
        go_root: goRoot.trim() || null,
        rust_toolchain: rustToolchain.trim() || null,
        node_home: nodeHome.trim() || null,
        python_interpreter: pythonInterpreter.trim() || null,
        dotenv_path: dotenvPath.trim() || null,
        extra_path: extraPath.trim() || null,
        env_vars: cleaned,
      });
      if (ok) { hydratedSnapshot = ''; await ide.loadEnvironments(); }
    } finally { savingEnv = false; }
  }

  function selectConfig(cfg: IdeRunConfig): void {
    cfgValidationError = '';
    isNewDraft = false;
    ide.selectedConfigId = cfg.id;
    cfgDraft = configToDraft(cfg);
  }

  function startNewConfig(): void {
    cfgValidationError = '';
    isNewDraft = true;
    cfgDraft = emptyConfigDraft();
  }

  function addOverrideRow(): void {
    cfgDraft.overrides = [...cfgDraft.overrides, { rowId: `ov-${Date.now()}-${cfgDraft.overrides.length}`, key: '', value: '' }];
  }

  function removeOverrideRow(rowId: string): void {
    cfgDraft.overrides = cfgDraft.overrides.filter((r) => r.rowId !== rowId);
  }

  function parseArgs(raw: string): string[] {
    const tokens = raw.match(/(?:[^\s"']+|"[^"]*"|'[^']*')+/g) ?? [];
    return tokens.map((t) => t.replace(/^['"]|['"]$/g, ''));
  }

  async function saveCurrentConfig(runAfterSave = false): Promise<void> {
    cfgValidationError = '';
    const name = cfgDraft.name.trim();
    if (!name) { cfgValidationError = 'Enter a configuration name before saving.'; return; }
    const overrides: Record<string, string> = {};
    for (let i = 0; i < cfgDraft.overrides.length; i++) {
      const row = cfgDraft.overrides[i];
      const k = row.key.trim();
      if (!k && !row.value.trim()) continue;
      if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(k)) { cfgValidationError = `Invalid override variable "${k || '(empty)'}".`; return; }
      overrides[k] = row.value;
    }

    savingCfg = true;
    try {
      const saved = await ide.saveRunConfig({
        id: isNewDraft ? undefined : cfgDraft.id || undefined,
        name,
        runner: cfgDraft.runner,
        target_path: cfgDraft.target_path.trim() || null,
        command: cfgDraft.command.trim() || null,
        test_filter: cfgDraft.test_filter.trim() || null,
        args: parseArgs(cfgDraft.argsText),
        cwd: cfgDraft.cwd.trim() || null,
        before_launch: cfgDraft.before_launch.trim() || null,
        env_overrides: overrides,
      });
      if (saved) {
        isNewDraft = false;
        cfgDraft = configToDraft(saved);
        toasts.success('Saved run configuration', saved.name);
        if (runAfterSave) { await ide.startRun({ configId: saved.id }); close(); }
      }
    } finally { savingCfg = false; }
  }

  async function confirmDeleteConfig(): Promise<void> {
    if (!cfgDraft.id || isNewDraft) return;
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
  <Modal title="Environments & run configurations" width={760} dismissable={!savingEnv && !savingCfg} onclose={close}>
    <div class="env-shell">
      {#if !ide.hasTarget}
        <EmptyState
          icon="folder"
          title="Open a workspace first"
          body="Select a local folder or SSH target before configuring SDK paths, Keychain secrets, and run configurations."
          actionLabel="Choose target…"
          actionKind="secondary"
          onaction={() => { close(); ide.sshOpen = true; }}
        />
      {:else}
        <Tabs label="Environment and configuration tabs" tabs={modalTabs} value={activeTab} onchange={(id) => (activeTab = id)} idBase="ide-env" />

        <LoadState what="project environment" loading={ide.envLoading} error={ide.envError} empty={false} onretry={() => ide.loadEnvironments()}>
          {#if activeTab === 'env'}
            <div class="env-pane" role="tabpanel" id="ide-env-panel-env" aria-labelledby="ide-env-tab-env">
              <div class="env-sdk-grid">
                <div class="field">
                  <label for="env-java-home">Active JDK (JAVA_HOME)</label>
                  <input id="env-java-home" class="input" type="text" dir="ltr" list="env-java-datalist" placeholder="System default JDK" bind:value={javaHome} />
                  <datalist id="env-java-datalist">
                    {#each sdkOptionsFor('java') as opt (opt.value)}<option value={opt.value}>{opt.label}</option>{/each}
                  </datalist>
                </div>

                <div class="field">
                  <label for="env-go-root">Go SDK (GOROOT)</label>
                  <input id="env-go-root" class="input" type="text" dir="ltr" list="env-go-datalist" placeholder="System default go" bind:value={goRoot} />
                  <datalist id="env-go-datalist">
                    {#each sdkOptionsFor('go') as opt (opt.value)}<option value={opt.value}>{opt.label}</option>{/each}
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
                          <input
                            class="input env-val-in"
                            type="password"
                            dir="ltr"
                            autocomplete="off"
                            aria-label="Secret value for {row.key || 'variable'}"
                            placeholder="Secret stored in macOS Keychain"
                            bind:value={row.value}
                          />
                        {:else}
                          <input
                            class="input env-val-in"
                            type="text"
                            dir="ltr"
                            aria-label="Value for {row.key || 'variable'}"
                            placeholder="Value"
                            bind:value={row.value}
                          />
                        {/if}

                        <div class="env-secret-toggle">
                          <Switch checked={row.secret} label="Store {row.key || 'variable'} in Keychain" onchange={(next) => toggleRowSecret(idx, next)} />
                          <span class="env-sec-tag">{row.secret ? 'Keychain' : 'Plain'}</span>
                        </div>

                        <button
                          type="button"
                          class="icon-btn"
                          aria-label="Remove environment variable"
                          title="Remove environment variable"
                          onclick={() => removeEnvRow(row.rowId)}
                        >
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
            <div class="env-cfg-layout" role="tabpanel" id="ide-env-panel-configs" aria-labelledby="ide-env-tab-configs">
              <aside class="env-cfg-rail" aria-label="Saved run configurations">
                <div class="env-rail-head">
                  <span class="env-sec-title">Saved ({ide.runConfigs.length})</span>
                  <button type="button" class="btn small" onclick={startNewConfig}>
                    <Icon name="plus" size={12} />
                    New
                  </button>
                </div>

                {#if ide.runConfigs.length === 0}
                  <div class="env-empty-note">No saved configurations yet — fill in the form to create your first one.</div>
                {:else}
                  <div class="env-cfg-list">
                    {#each ide.runConfigs as cfg (cfg.id)}
                      <button
                        type="button"
                        class="env-cfg-item"
                        class:is-active={!isNewDraft && cfgDraft.id === cfg.id}
                        aria-pressed={!isNewDraft && cfgDraft.id === cfg.id}
                        onclick={() => selectConfig(cfg)}
                      >
                        <span class="env-cfg-name">{cfg.name}</span>
                        <Badge tone="neutral" variant="outline" label={runnerShortLabel(cfg.runner)} />
                      </button>
                    {/each}
                  </div>
                {/if}
              </aside>

              <div class="env-cfg-editor">
                <div class="env-sdk-grid">
                  <div class="field">
                    <label for="cfg-name">Configuration name</label>
                    <input id="cfg-name" class="input" type="text" dir="auto" placeholder="e.g. Go package tests" bind:value={cfgDraft.name} />
                  </div>
                  <div class="field">
                    <label for="cfg-runner">Runner</label>
                    <select id="cfg-runner" class="input" bind:value={cfgDraft.runner}>
                      {#each RUNNER_OPTIONS as opt (opt.value)}<option value={opt.value}>{opt.label}</option>{/each}
                    </select>
                  </div>
                </div>

                <div class="env-sdk-grid">
                  <div class="field">
                    <label for="cfg-target">Target file / package / script</label>
                    <input id="cfg-target" class="input" type="text" dir="ltr" placeholder="./cmd/server or src/main.py" bind:value={cfgDraft.target_path} />
                  </div>
                  <div class="field">
                    <label for="cfg-filter">Test filter / pattern</label>
                    <input id="cfg-filter" class="input" type="text" dir="ltr" placeholder="TestAdd or -k test_auth" bind:value={cfgDraft.test_filter} />
                  </div>
                </div>

                <div class="env-sdk-grid">
                  <div class="field">
                    <label for="cfg-args">Arguments</label>
                    <input id="cfg-args" class="input" type="text" dir="ltr" placeholder="--verbose --port 8080" bind:value={cfgDraft.argsText} />
                  </div>
                  <div class="field">
                    <label for="cfg-cwd">Working directory</label>
                    <input id="cfg-cwd" class="input" type="text" dir="ltr" placeholder={ide.target.root || 'Workspace root'} bind:value={cfgDraft.cwd} />
                  </div>
                </div>

                <div class="env-sdk-grid">
                  <div class="field">
                    <label for="cfg-cmd">Custom command (optional)</label>
                    <input id="cfg-cmd" class="input" type="text" dir="ltr" placeholder="make integration-test" bind:value={cfgDraft.command} />
                  </div>
                  <div class="field">
                    <label for="cfg-before">Before launch command</label>
                    <input id="cfg-before" class="input" type="text" dir="ltr" placeholder="go generate ./..." bind:value={cfgDraft.before_launch} />
                  </div>
                </div>

                <div class="env-ovr-box">
                  <div class="env-rail-head">
                    <span class="env-sec-title">Environment overrides</span>
                    <button type="button" class="btn small" onclick={addOverrideRow}>
                      <Icon name="plus" size={12} />
                      Add override
                    </button>
                  </div>
                  {#if cfgDraft.overrides.length > 0}
                    <div class="env-var-list">
                      {#each cfgDraft.overrides as ov (ov.rowId)}
                        <div class="env-ovr-row">
                          <input class="input env-key-in" type="text" dir="ltr" aria-label="Override variable name" placeholder="KEY" bind:value={ov.key} />
                          <input class="input env-val-in" type="text" dir="ltr" aria-label="Override value" placeholder="Value" bind:value={ov.value} />
                          <button type="button" class="icon-btn" aria-label="Remove override" title="Remove override" onclick={() => removeOverrideRow(ov.rowId)}>
                            <Icon name="trash" size={13} />
                          </button>
                        </div>
                      {/each}
                    </div>
                  {/if}
                </div>

                {#if cfgValidationError}
                  <div class="env-err" use:announceShown={{ text: cfgValidationError, assertive: true }}>{cfgValidationError}</div>
                {/if}
              </div>
            </div>
          {/if}
        </LoadState>
      {/if}
    </div>

    {#snippet footer()}
      {#if activeTab === 'configs' && ide.hasTarget}
        <div class="env-footer-bar">
          <div>
            {#if !isNewDraft && cfgDraft.id}
              <button type="button" class="btn danger" disabled={savingCfg} onclick={confirmDeleteConfig}>
                <Icon name="trash" size={13} />
                Delete…
              </button>
            {/if}
          </div>
          <div class="env-footer-end">
            <button type="button" class="btn ghost" disabled={savingCfg} onclick={close}>Cancel</button>
            <button type="button" class="btn" disabled={savingCfg} onclick={() => saveCurrentConfig(true)}>
              <Icon name="play" size={12} />
              Save &amp; run
            </button>
            <button type="button" class="btn primary" disabled={savingCfg} onclick={() => saveCurrentConfig(false)}>
              {savingCfg ? 'Saving…' : 'Save configuration'}
            </button>
          </div>
        </div>
      {:else}
        <button type="button" class="btn ghost" disabled={savingEnv} onclick={close}>Cancel</button>
        {#if ide.hasTarget}
          <button type="button" class="btn primary" disabled={savingEnv} onclick={handleSaveEnvironment}>
            {savingEnv ? 'Saving…' : 'Save environment'}
          </button>
        {/if}
      {/if}
    {/snippet}
  </Modal>
{/if}

<style>
  .env-shell { display: flex; flex-direction: column; gap: 12px; min-width: 0; }
  .env-pane { display: flex; flex-direction: column; gap: 4px; padding-block-start: 4px; min-width: 0; }
  .env-sdk-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; }
  .env-inline-bar { display: flex; align-items: center; gap: 8px; }
  .env-inline-bar input { flex: 1; min-width: 0; }
  .env-chip-row { display: flex; flex-wrap: wrap; gap: 6px; margin-block-start: 4px; }

  .env-vars-box,
  .env-ovr-box {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    border: 1px solid var(--border);
  }
  .env-vars-head, .env-rail-head { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px; }
  .env-vars-btns { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .env-sec-title { margin: 0; font-size: var(--fs-s); font-weight: 600; color: var(--text); }
  .env-sec-sub, .env-empty-note { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); }
  .env-var-list { display: flex; flex-direction: column; gap: 6px; }
  .env-var-item, .env-ovr-row { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .env-key-in { flex: 0 1 180px; min-width: 120px; font-family: var(--font-mono); }
  .env-val-in { flex: 1; min-width: 0; }

  .env-secret-mask {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    min-width: 0;
    padding-inline: 8px;
    height: 28px;
    border-radius: var(--radius-s);
    background: var(--surface);
    border: 1px solid var(--border);
  }
  .env-dots { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); margin-inline-end: auto; }
  .env-secret-toggle { display: flex; align-items: center; gap: 6px; flex-shrink: 0; }
  .env-sec-tag { font-size: var(--fs-xs); color: var(--text-dim); min-inline-size: 48px; }

  .env-cfg-layout { display: grid; grid-template-columns: 220px minmax(0, 1fr); gap: 14px; padding-block-start: 4px; min-width: 0; }
  .env-cfg-rail {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    border: 1px solid var(--border);
    min-width: 0;
  }
  .env-cfg-list { display: flex; flex-direction: column; gap: 4px; max-block-size: 320px; overflow-y: auto; }
  .env-cfg-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    border: 1px solid transparent;
    background: transparent;
    color: var(--text);
    text-align: start;
    cursor: pointer;
    transition: background var(--dur-fast) var(--ease-out);
  }
  .env-cfg-item:hover { background: var(--hover); }
  .env-cfg-item.is-active { background: var(--accent-soft); border-color: var(--accent-line); color: var(--accent-text); }
  .env-cfg-name { font-size: var(--fs-s); font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .env-cfg-editor { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .env-err { font-size: var(--fs-xs); color: var(--danger); }
  .env-footer-bar { display: flex; align-items: center; justify-content: space-between; gap: 12px; inline-size: 100%; }
  .env-footer-end { display: flex; align-items: center; gap: 8px; }

  @media (max-width: 640px) {
    .env-sdk-grid, .env-cfg-layout { grid-template-columns: 1fr; }
    .env-var-item, .env-ovr-row { flex-wrap: wrap; }
  }
</style>
