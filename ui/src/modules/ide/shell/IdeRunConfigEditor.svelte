<script module lang="ts">
  import { ide, type IdeRunConfig } from '../ideStore.svelte';
  import { draftDefaultsForTab, targetFieldCopy, visibleRunConfigFields, type RunConfigField } from './ideRunConfigForm';

  export interface ConfigDraft {
    id: string;
    name: string;
    runner: string;
    target_path: string;
    command: string;
    test_filter: string;
    buildFlagsText: string;
    argsText: string;
    cwd: string;
    env_file: string;
    sdk_path: string;
    inherit_shell_env: boolean;
    before_launch: string;
    maven_profiles: string[];
    spring_profiles: string[];
    skip_tests: boolean;
    offline: boolean;
    autoDetected: boolean;
    overrides: { rowId: string; key: string; value: string }[];
  }

  export const RUNNER_OPTIONS: readonly { value: string; label: string; short: string; eco: string }[] = [
    { value: 'go_test', label: 'Go · go test', short: 'go test', eco: 'go' },
    { value: 'go_run', label: 'Go · go run', short: 'go run', eco: 'go' },
    { value: 'go_build', label: 'Go · go build', short: 'go build', eco: 'go' },
    { value: 'java_junit', label: 'Java · JUnit', short: 'JUnit', eco: 'java' },
    { value: 'java_main', label: 'Java · Java Main', short: 'Java', eco: 'java' },
    { value: 'maven', label: 'Java · Maven', short: 'Maven', eco: 'java' },
    { value: 'gradle', label: 'Java · Gradle', short: 'Gradle', eco: 'java' },
    { value: 'cargo_test', label: 'Rust · cargo test', short: 'cargo test', eco: 'rust' },
    { value: 'cargo_run', label: 'Rust · cargo run', short: 'cargo run', eco: 'rust' },
    { value: 'node', label: 'JS/TS · Node file', short: 'Node', eco: 'node' },
    { value: 'npm', label: 'JS/TS · npm script', short: 'npm', eco: 'node' },
    { value: 'vitest', label: 'JS/TS · Vitest', short: 'Vitest', eco: 'node' },
    { value: 'pytest', label: 'Python · pytest', short: 'pytest', eco: 'python' },
    { value: 'python', label: 'Python · Python file', short: 'Python', eco: 'python' },
    { value: 'shell', label: 'Shell · Script (.sh / .zsh)', short: 'sh', eco: 'shell' },
    { value: 'custom', label: 'Shell · Custom command', short: 'Shell', eco: 'shell' },
  ];

  /** A new configuration starts from the active tab: its runner, name and file. */
  export function emptyConfigDraft(tab: Parameters<typeof draftDefaultsForTab>[0] = ide.activeTab): ConfigDraft {
    const start = draftDefaultsForTab(tab);
    return {
      id: '', name: start.name, runner: start.runner, target_path: start.target_path, command: '',
      test_filter: '', buildFlagsText: '', argsText: '', cwd: '', env_file: '',
      sdk_path: '', inherit_shell_env: true, before_launch: '',
      maven_profiles: [], spring_profiles: [], skip_tests: false, offline: false,
      autoDetected: false, overrides: [],
    };
  }
</script>

<script lang="ts">
  import { announceShown } from '../../../lib/announce.svelte';
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import PathField from '../../../lib/components/PathField.svelte';
  import Switch from '../../../lib/components/Switch.svelte';
  import { plural } from '../../../lib/plural';
  import { toasts } from '../../../lib/toast.svelte';
  import { parseDotenvBlock } from './IdeShellEnvCard.svelte';

  interface Props {
    cfgDraft: ConfigDraft;
    isNewDraft: boolean;
    dotenvPath: string;
    cfgValidationError: string;
    onselect: (cfg: IdeRunConfig) => void;
    onnew: () => void;
    onduplicate: () => void;
    onopentoolchains: () => void;
  }

  let {
    cfgDraft = $bindable(),
    isNewDraft,
    dotenvPath,
    cfgValidationError,
    onselect,
    onnew,
    onduplicate,
    onopentoolchains,
  }: Props = $props();

  let cfgPasteOpen = $state(false);
  let cfgPasteRaw = $state('');
  // null = follow the draft (open when an advanced field has a value); a click pins it.
  let advancedPinned = $state<boolean | null>(null);
  let advancedFor = $state('');

  const savedConfigs = $derived(ide.runConfigs.filter((c) => !c.auto_detected));
  const detectedConfigs = $derived(ide.runConfigs.filter((c) => c.auto_detected));
  const cfgRunnerEco = $derived(RUNNER_OPTIONS.find((r) => r.value === cfgDraft.runner)?.eco ?? 'go');

  const cfgSdkOptions = $derived.by(() => {
    const found = ide.toolchains.find((t) => t.ecosystem === cfgRunnerEco);
    return (found?.installed ?? []).map((sdk) => ({
      label: `${sdk.version} (${sdk.source})`,
      value: cfgRunnerEco === 'rust' ? sdk.version : sdk.path,
    }));
  });

  const fields = $derived(visibleRunConfigFields(cfgDraft.runner, cfgDraft));
  const targetCopy = $derived(targetFieldCopy(cfgDraft.runner));
  // "Advanced" opens by itself when something in it is set (also when a runner
  // switch moves a value there) and stays open while it is edited or cleared.
  $effect(() => {
    const key = `${cfgDraft.id}|${isNewDraft}`;
    const auto = fields.advancedOpen;
    if (key !== advancedFor) { advancedFor = key; advancedPinned = auto ? true : null; return; }
    if (auto && advancedPinned === null) advancedPinned = true;
  });
  const advancedOpen = $derived(advancedPinned ?? fields.advancedOpen);

  function runnerShortLabel(runner: string): string {
    return RUNNER_OPTIONS.find((r) => r.value === runner)?.short ?? runner;
  }

  function addOverrideRow(): void {
    cfgDraft.overrides = [
      ...cfgDraft.overrides,
      { rowId: `ov-${Date.now()}-${cfgDraft.overrides.length}`, key: '', value: '' },
    ];
  }

  function removeOverrideRow(rowId: string): void {
    cfgDraft.overrides = cfgDraft.overrides.filter((r) => r.rowId !== rowId);
  }

  function applyCfgPastedEnv(): void {
    const items = parseDotenvBlock(cfgPasteRaw);
    if (items.length === 0) return;
    const map = new Map(cfgDraft.overrides.map((r) => [r.key.trim(), r]));
    for (let i = 0; i < items.length; i++) {
      const { key, value } = items[i];
      const existing = map.get(key);
      if (existing) existing.value = value;
      else map.set(key, { rowId: `ov-paste-${Date.now()}-${i}`, key, value });
    }
    cfgDraft.overrides = Array.from(map.values());
    cfgPasteRaw = '';
    cfgPasteOpen = false;
    toasts.success(`Imported ${plural(items.length, 'override')}`);
  }
</script>

<div class="env-cfg-layout" role="tabpanel" id="ide-env-panel-configs" aria-labelledby="ide-env-tab-configs">
  <aside class="env-cfg-rail" aria-label="Saved run configurations">
    <div class="env-rail-head">
      <span class="env-sec-title">Saved ({savedConfigs.length})</span>
      <div class="env-vars-btns">
        {#if cfgDraft.name}
          <button type="button" class="icon-btn" aria-label="Duplicate configuration" title="Duplicate configuration" onclick={onduplicate}>
            <Icon name="copy" size={12} />
          </button>
        {/if}
        <button type="button" class="btn small" onclick={onnew}>
          <Icon name="plus" size={12} />
          New
        </button>
      </div>
    </div>

    {#if savedConfigs.length === 0 && detectedConfigs.length === 0}
      <div class="env-empty-note">No configurations yet — fill in the form to create your first one.</div>
    {:else}
      <div class="env-cfg-list">
        {#each savedConfigs as cfg (cfg.id)}
          <button
            type="button"
            class="env-cfg-item"
            class:is-active={!isNewDraft && cfgDraft.id === cfg.id}
            aria-pressed={!isNewDraft && cfgDraft.id === cfg.id}
            onclick={() => onselect(cfg)}
          >
            <span class="env-cfg-name">{cfg.name}</span>
            <Badge tone="neutral" variant="outline" label={runnerShortLabel(cfg.runner)} />
          </button>
        {/each}

        {#if detectedConfigs.length > 0}
          <div class="env-rail-divider">Auto-detected ({detectedConfigs.length})</div>
          {#each detectedConfigs as cfg (cfg.id)}
            <button
              type="button"
              class="env-cfg-item"
              class:is-active={ide.selectedConfigId === cfg.id && cfgDraft.autoDetected}
              aria-pressed={ide.selectedConfigId === cfg.id && cfgDraft.autoDetected}
              onclick={() => onselect(cfg)}
            >
              <span class="env-cfg-name">{cfg.name}</span>
              <Badge tone="info" variant="outline" label={runnerShortLabel(cfg.runner)} />
            </button>
          {/each}
        {/if}
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
        <select id="cfg-runner" class="input" bind:value={cfgDraft.runner} data-testid="ide-cfg-runner">
          {#each RUNNER_OPTIONS as opt (opt.value)}<option value={opt.value}>{opt.label}</option>{/each}
        </select>
      </div>
    </div>

    {#snippet fieldFor(f: RunConfigField)}
      {#if f === 'target'}
        <div class="field" data-testid="ide-cfg-field-target">
          <label for="cfg-target">{targetCopy.label}</label>
          <input id="cfg-target" class="input" type="text" dir="ltr" placeholder={targetCopy.placeholder} bind:value={cfgDraft.target_path} />
        </div>
      {:else if f === 'command'}
        <div class="field" data-testid="ide-cfg-field-command">
          <label for="cfg-cmd">Command</label>
          <input id="cfg-cmd" class="input" type="text" dir="ltr" placeholder="make integration-test" bind:value={cfgDraft.command} />
        </div>
      {:else if f === 'test_filter'}
        <div class="field" data-testid="ide-cfg-field-test_filter">
          <label for="cfg-filter">Test filter / pattern</label>
          <input id="cfg-filter" class="input" type="text" dir="ltr" placeholder="TestAdd or -k test_auth" bind:value={cfgDraft.test_filter} />
        </div>
      {:else if f === 'args'}
        <div class="field" data-testid="ide-cfg-field-args">
          <label for="cfg-args">Program arguments</label>
          <input id="cfg-args" class="input" type="text" dir="ltr" placeholder="--verbose --port 8080" bind:value={cfgDraft.argsText} />
        </div>
      {:else if f === 'cwd'}
        <div class="field" data-testid="ide-cfg-field-cwd">
          <label for="cfg-cwd">Working directory</label>
          <PathField bind:value={cfgDraft.cwd} start={ide.target.root} disabled={ide.target.kind === 'ssh'}>
            <input id="cfg-cwd" class="input" type="text" dir="ltr" placeholder={ide.target.root || 'Workspace root'} bind:value={cfgDraft.cwd} />
          </PathField>
        </div>
      {:else if f === 'build_flags'}
        <div class="field" data-testid="ide-cfg-field-build_flags">
          <label for="cfg-build-flags">Build / tool flags</label>
          <input id="cfg-build-flags" class="input" type="text" dir="ltr" placeholder="-race -v or --release" bind:value={cfgDraft.buildFlagsText} />
        </div>
      {:else if f === 'sdk'}
        <div class="field" data-testid="ide-cfg-field-sdk">
          <label for="cfg-sdk-path">SDK / toolchain override</label>
          <div class="env-inline-bar">
            <input id="cfg-sdk-path" class="input" type="text" dir="ltr" list="cfg-sdk-datalist" placeholder="Project default SDK" bind:value={cfgDraft.sdk_path} />
            <button type="button" class="btn small" title="Download or manage SDK versions" onclick={onopentoolchains} data-testid="ide-cfg-open-toolchains">
              <Icon name="download" size={12} />
              SDKs…
            </button>
          </div>
          <datalist id="cfg-sdk-datalist">
            {#each cfgSdkOptions as opt (opt.value)}<option value={opt.value}>{opt.label}</option>{/each}
          </datalist>
        </div>
      {:else if f === 'env_file'}
        <div class="field" data-testid="ide-cfg-field-env_file">
          <label for="cfg-env-file">.env file</label>
          <input id="cfg-env-file" class="input" type="text" dir="ltr" list="cfg-dotenv-datalist" placeholder={dotenvPath || '.env'} bind:value={cfgDraft.env_file} />
          <datalist id="cfg-dotenv-datalist">
            {#each ide.dotenvFiles as df (df)}<option value={df}>{df}</option>{/each}
          </datalist>
        </div>
      {:else if f === 'before_launch'}
        <div class="field" data-testid="ide-cfg-field-before_launch">
          <label for="cfg-before">Before launch command</label>
          <input id="cfg-before" class="input" type="text" dir="ltr" placeholder="go generate ./..." bind:value={cfgDraft.before_launch} />
        </div>
      {:else if f === 'inherit_shell_env'}
        <div class="env-shell-switch env-span" data-testid="ide-cfg-field-inherit_shell_env">
          <Switch
            checked={cfgDraft.inherit_shell_env}
            label="Inherit login shell environment (~/.zshrc, ~/.zshenv)"
            onchange={(next) => (cfgDraft.inherit_shell_env = next)}
          />
        </div>
      {/if}
    {/snippet}

    <div class="env-sdk-grid">
      {#each fields.primary as f (f)}{@render fieldFor(f)}{/each}
    </div>

    <div class="env-adv">
      <button
        type="button"
        class="env-adv-toggle"
        aria-expanded={advancedOpen}
        aria-controls="ide-cfg-advanced"
        onclick={() => (advancedPinned = !advancedOpen)}
        data-testid="ide-cfg-advanced-toggle"
      >
        <Icon name={advancedOpen ? 'chevronDown' : 'chevronRight'} size={12} noflip />
        Advanced
      </button>
      {#if advancedOpen}
        <div id="ide-cfg-advanced" class="env-sdk-grid" data-testid="ide-cfg-advanced">
          {#each fields.advanced as f (f)}{@render fieldFor(f)}{/each}
        </div>
      {/if}
    </div>

    <div class="env-ovr-box">
      <div class="env-rail-head">
        <span class="env-sec-title">Environment variables</span>
        <div class="env-vars-btns">
          <button type="button" class="btn small ghost" aria-expanded={cfgPasteOpen} onclick={() => (cfgPasteOpen = !cfgPasteOpen)} data-testid="ide-cfg-paste-env-btn">
            <Icon name="file" size={12} />
            Paste .env…
          </button>
          <button type="button" class="btn small" onclick={addOverrideRow}>
            <Icon name="plus" size={12} />
            Add override
          </button>
        </div>
      </div>

      {#if cfgPasteOpen}
        <div class="env-paste-inline">
          <textarea
            class="input env-paste-ta"
            rows="3"
            dir="ltr"
            aria-label="Paste KEY=VALUE environment overrides"
            placeholder={'GO_DEBUGGING_PARAMS="--headless"\nENV=local'}
            bind:value={cfgPasteRaw}
          ></textarea>
          <div class="env-paste-actions">
            <button type="button" class="btn small ghost" onclick={() => { cfgPasteOpen = false; cfgPasteRaw = ''; }}>Cancel</button>
            <button type="button" class="btn small primary" disabled={!cfgPasteRaw.trim()} onclick={applyCfgPastedEnv}>Import</button>
          </div>
        </div>
      {/if}

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

<style>
  .env-cfg-layout { display: grid; grid-template-columns: 230px minmax(0, 1fr); gap: 14px; padding-block-start: 4px; min-inline-size: 0; }
  .env-cfg-rail {
    display: flex; flex-direction: column; gap: 8px; padding: 10px;
    border-radius: var(--radius-m); background: var(--surface-2); border: 1px solid var(--border); min-inline-size: 0;
  }
  .env-rail-head { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px; }
  .env-vars-btns { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .env-sec-title { margin: 0; font-size: var(--fs-s); font-weight: 600; color: var(--text); }
  .env-empty-note { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); }
  .env-rail-divider {
    padding-block-start: 6px; margin-block-start: 4px; border-block-start: 1px solid var(--separator);
    font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim);
  }
  .env-cfg-list { display: flex; flex-direction: column; gap: 4px; max-block-size: 380px; overflow-y: auto; }
  .env-cfg-item {
    display: flex; align-items: center; justify-content: space-between; gap: 6px;
    padding: 6px 8px; border-radius: var(--radius-s); border: 1px solid transparent;
    background: transparent; color: var(--text); text-align: start; cursor: pointer;
    transition: background var(--dur-fast) var(--ease-out);
  }
  .env-cfg-item:hover { background: var(--hover); }
  .env-cfg-item.is-active { background: var(--accent-soft); border-color: var(--accent-line); color: var(--accent-text); }
  .env-cfg-name { font-size: var(--fs-s); font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-inline-size: 0; }
  .env-cfg-editor { display: flex; flex-direction: column; gap: 6px; min-inline-size: 0; }
  .env-sdk-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; }
  .env-inline-bar { display: flex; align-items: center; gap: 8px; }
  .env-inline-bar input { flex: 1; min-inline-size: 0; }
  .env-shell-switch { display: flex; align-items: center; gap: 8px; padding-block: 4px; }
  .env-span { grid-column: 1 / -1; }
  .env-adv { display: flex; flex-direction: column; gap: 6px; }
  .env-adv-toggle {
    display: inline-flex; align-items: center; gap: 4px; align-self: flex-start; padding-block: 4px; padding-inline: 2px;
    border: 0; background: transparent; color: var(--text-dim); font-size: var(--fs-xs); font-weight: 600; cursor: pointer;
  }
  .env-adv-toggle:hover { color: var(--text); }
  .env-adv-toggle:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; border-radius: var(--radius-s); }
  .env-ovr-box {
    display: flex; flex-direction: column; gap: 8px; padding: 12px;
    border-radius: var(--radius-m); background: var(--surface-2); border: 1px solid var(--border);
  }
  .env-paste-inline { display: flex; flex-direction: column; gap: 6px; }
  .env-paste-ta { font-family: var(--font-mono); font-size: var(--fs-xs); resize: vertical; }
  .env-paste-actions { display: flex; justify-content: flex-end; gap: 6px; }
  .env-var-list { display: flex; flex-direction: column; gap: 6px; }
  .env-ovr-row { display: flex; align-items: center; gap: 8px; min-inline-size: 0; }
  .env-key-in { flex: 0 1 180px; min-inline-size: 120px; font-family: var(--font-mono); }
  .env-val-in { flex: 1; min-inline-size: 0; }
  .env-err { font-size: var(--fs-xs); color: var(--danger); }

  @media (max-width: 640px) {
    /* minmax(0, …): a 1fr track's floor is its widest child, and the runner
       select's longest option pushed the form 28 px past a 320 px phone. */
    .env-sdk-grid, .env-cfg-layout { grid-template-columns: minmax(0, 1fr); }
    .env-cfg-editor :is(.field, select, input) { min-inline-size: 0; max-inline-size: 100%; }
    .env-ovr-row { flex-wrap: wrap; }
  }
</style>
