<script lang="ts">
  // GoLand-style Run / Debug configuration selector & execution cluster:
  // `[Config Selector ▾] [▶ Run] [🐞 Debug] [⏹ Stop] [⚙]`
  import Icon from '../../../lib/components/Icon.svelte';
  import LazyMount from '../../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { viewport } from '../../../lib/stores/viewport.svelte';
  import { ide, type IdeRunConfig } from '../ideStore.svelte';
  import { bridgeEnvCache } from './networkBridgeGuard.svelte';
  import { formatBridgeToolbarLabel } from './networkBridgeModel';

  const IdeRunConfigPopoverLazy = lazyComponent(() => import('./IdeRunConfigPopover.svelte'));

  let menuOpen = $state(false);
  let triggerEl = $state<HTMLButtonElement | undefined>(undefined);

  const isRunning = $derived(ide.activeRun?.status === 'running');
  const isDebugging = $derived(
    Boolean(ide.activeDebug && ide.activeDebug.state !== 'terminated' && ide.activeDebug.state !== 'error'),
  );
  const selectedConfig = $derived<IdeRunConfig | null>(
    ide.runConfigs.find((c) => c.id === ide.selectedConfigId) ?? ide.runConfigs[0] ?? null,
  );
  const selectorLabel = $derived(
    selectedConfig?.name || (ide.activeTab ? `Current file (${ide.activeTab.name})` : 'Current file'),
  );
  const bridgeLabel = $derived(
    ide.target.kind === 'local' ? formatBridgeToolbarLabel(ide.projectEnv?.network_bridge) : null,
  );
  const isProdBridge = $derived(
    ide.target.kind === 'local' && bridgeEnvCache.isProdBridge(ide.projectEnv?.network_bridge),
  );

  $effect(() => {
    if (ide.target.kind === 'local' && ide.projectEnv?.network_bridge?.enabled) {
      void bridgeEnvCache.ensureLoaded(ide.target.workspace_id);
    }
  });
</script>

{#if !viewport.isPhone}
  <div class="run-cluster" data-keep role="group" aria-label="Launch controls">
    {#if bridgeLabel}
      <button
        type="button"
        class="run-bridge-btn"
        class:is-prod={isProdBridge}
        title={isProdBridge
          ? 'Active PRODUCTION cluster / SSH network bridge — click to configure'
          : 'Active cluster / SSH network bridge — click to configure'}
        onclick={() => ide.openEnvironmentsModal('env')}
        data-testid="ide-netbridge-toolbar-pill"
      >
        <Icon name="helm" size={12} />
        {#if isProdBridge}
          <span class="run-bridge-prod">PROD</span>
        {/if}
        <span class="run-cfg-label" dir="ltr">{bridgeLabel}</span>
      </button>
    {/if}
    <button
      bind:this={triggerEl}
      type="button"
      class="run-cfg-btn"
      aria-label="Select launch target"
      title={`Launch target: ${selectorLabel}`}
      aria-haspopup="dialog"
      aria-expanded={menuOpen}
      onclick={() => (menuOpen = !menuOpen)}
      onpointerenter={() => IdeRunConfigPopoverLazy.prefetch()}
      data-testid="ide-run-config-select"
    >
      <Icon name="play" size={12} />
      <span class="run-cfg-label" dir="ltr">{selectorLabel}</span>
      <Icon name="chevronDown" size={12} />
    </button>

    <button
      type="button"
      class="btn small"
      class:primary={!isRunning && !isDebugging}
      onclick={() => void ide.startRun()}
      title="Run selected target (⌃R)"
      data-testid="ide-run-btn"
    >
      <Icon name="play" size={12} />
      Run
    </button>

    <button
      type="button"
      class="icon-btn"
      aria-label="Run with coverage"
      title="Run with coverage"
      onclick={() => void ide.startRun({ coverage: true })}
      data-testid="ide-run-coverage-btn"
    >
      <Icon name="shield" size={14} />
    </button>

    <button
      type="button"
      class="icon-btn"
      aria-label="Debug selected target (⌃D)"
      title="Debug selected target (⌃D)"
      onclick={() => void ide.startDebug()}
      data-testid="ide-debug-btn"
    >
      <Icon name="bug" size={14} />
    </button>

    {#if isRunning || isDebugging}
      <button
        type="button"
        class="btn small primary"
        onclick={() => void ide.stopActiveExecution()}
        title="Stop active session (⌘F2)"
        data-testid="ide-stop-btn"
      >
        <Icon name="stop" size={12} />
        Stop
      </button>
    {/if}

    <button
      type="button"
      class="icon-btn"
      aria-label="Launch settings…"
      title="Launch settings…"
      onclick={() => ide.openEnvironmentsModal('configs')}
      data-testid="ide-edit-configs-btn"
    >
      <Icon name="gear" size={14} />
    </button>
  </div>
{:else}
  {#if bridgeLabel && isProdBridge}
    <button
      type="button"
      class="run-bridge-btn is-prod is-compact"
      title="Active PRODUCTION cluster / SSH network bridge — click to configure"
      onclick={() => ide.openEnvironmentsModal('env')}
      data-testid="ide-netbridge-toolbar-pill"
    >
      <span class="run-bridge-prod">PROD</span>
    </button>
  {/if}
  {#if isRunning || isDebugging}
    <button
      type="button"
      class="btn small primary"
      onclick={() => void ide.stopActiveExecution()}
      title="Stop active session"
      data-testid="ide-stop-btn"
    >
      <Icon name="stop" size={12} />
      Stop
    </button>
  {:else}
    <button
      type="button"
      class="btn small primary"
      onclick={() => void ide.startRun()}
      title="Run selected target"
      data-testid="ide-run-btn"
    >
      <Icon name="play" size={12} />
      Run
    </button>
  {/if}
  <button
    type="button"
    class="icon-btn"
    data-overflow="3"
    data-icon="shield"
    aria-label="Run with coverage"
    title="Run with coverage"
    onclick={() => void ide.startRun({ coverage: true })}
    data-testid="ide-run-coverage-btn"
  >
    <Icon name="shield" size={14} />
  </button>
  <button
    type="button"
    class="icon-btn"
    data-overflow="4"
    data-icon="bug"
    aria-label="Debug"
    title="Debug"
    onclick={() => void ide.startDebug()}
    data-testid="ide-debug-btn"
  >
    <Icon name="bug" size={14} />
  </button>
{/if}

{#if menuOpen}
  <LazyMount
    lazy={IdeRunConfigPopoverLazy}
    what="launch targets"
    quiet
    props={{ triggerEl, selectedConfig, onclose: () => (menuOpen = false) }}
    onerror={() => (menuOpen = false)}
  />
{/if}

<style>
  .run-cluster { display: inline-flex; align-items: center; gap: 4px; min-inline-size: 0; }
  .run-cfg-btn {
    display: inline-flex; align-items: center; gap: 6px; max-inline-size: min(170px, 16vw);
    min-inline-size: 0; block-size: 26px; padding-inline: 8px; background: var(--surface-2);
    color: var(--text); border: 1px solid var(--border); border-radius: var(--radius-s);
    font-size: var(--fs-xs); font-weight: 500; cursor: pointer;
  }
  .run-cfg-btn:hover { border-color: var(--border-strong); background: var(--hover); }
  .run-bridge-btn {
    display: inline-flex; align-items: center; gap: 4px; max-inline-size: 168px;
    min-inline-size: 0; block-size: 26px; padding-inline: 8px; background: var(--accent-soft);
    color: var(--accent-text); border: 1px solid var(--accent-line); border-radius: var(--radius-s);
    font-size: var(--fs-xs); font-weight: 500; cursor: pointer;
  }
  .run-bridge-btn:hover { background: var(--hover); }
  .run-bridge-btn.is-prod {
    background: var(--danger-soft);
    color: var(--danger);
    border-color: color-mix(in srgb, var(--danger) 35%, transparent);
  }
  .run-bridge-btn.is-compact { padding-inline: 6px; flex-shrink: 0; }
  .run-bridge-prod { font-weight: 600; letter-spacing: 0.06em; flex-shrink: 0; }
  .run-cfg-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-inline-size: 0; }
  @media (max-width: 1024px) {
    .run-cfg-btn { max-inline-size: 150px; }
    .run-bridge-btn { max-inline-size: 136px; }
  }
</style>
