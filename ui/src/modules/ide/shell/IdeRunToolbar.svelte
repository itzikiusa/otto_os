<script lang="ts">
  // GoLand-style Run / Debug configuration selector & execution cluster:
  // `[Config Selector ▾] [▶ Run] [🐞 Debug] [⏹ Stop] [⚙]`
  import { tick } from 'svelte';
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { viewport } from '../../../lib/stores/viewport.svelte';
  import { ide, type IdeRunConfig } from '../ideStore.svelte';

  let menuOpen = $state(false);
  let filter = $state('');
  let triggerEl = $state<HTMLButtonElement | undefined>(undefined);
  let popEl = $state<HTMLDivElement | undefined>(undefined);
  let filterEl = $state<HTMLInputElement | undefined>(undefined);
  let pos = $state({ left: 12, top: 48, width: 360, maxH: 400 });

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

  const savedConfigs = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    return ide.runConfigs.filter((c) => !c.auto_detected && (!q || c.name.toLowerCase().includes(q) || c.runner.toLowerCase().includes(q)));
  });

  const detectedConfigs = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    return ide.runConfigs.filter((c) => Boolean(c.auto_detected) && (!q || c.name.toLowerCase().includes(q) || c.runner.toLowerCase().includes(q)));
  });

  function clampPosition(): void {
    if (typeof window === 'undefined' || !triggerEl) return;
    const rect = triggerEl.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const width = Math.max(250, Math.min(400, vw - 16));
    const left = Math.max(8, Math.min(rect.right - width, vw - width - 8));
    const maxH = Math.max(200, Math.min(460, vh - 16));
    const measuredH = popEl ? Math.min(popEl.offsetHeight || 280, maxH) : 280;
    const top = Math.max(8, Math.min(rect.bottom + 6, vh - measuredH - 8));
    pos = { left, top, width, maxH };
  }

  async function toggleMenu(): Promise<void> {
    menuOpen = !menuOpen;
    if (menuOpen) {
      filter = '';
      clampPosition();
      await tick();
      clampPosition();
      filterEl?.focus();
    }
  }

  function closeMenu(): void {
    menuOpen = false;
  }

  $effect(() => {
    if (!menuOpen) return;
    void savedConfigs.length;
    void detectedConfigs.length;
    requestAnimationFrame(() => clampPosition());
  });

  $effect(() => {
    if (!menuOpen) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        closeMenu();
        triggerEl?.focus();
      }
    };
    const onResize = (): void => clampPosition();
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('resize', onResize);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('resize', onResize);
    };
  });

  function selectTarget(id: string): void {
    ide.selectedConfigId = id;
    closeMenu();
  }
</script>

{#if !viewport.isPhone}
  <div class="run-cluster" role="group" aria-label="Launch controls">
    <button
      bind:this={triggerEl}
      type="button"
      class="run-cfg-btn"
      aria-label="Select launch target"
      title={`Launch target: ${selectorLabel}`}
      aria-haspopup="dialog"
      aria-expanded={menuOpen}
      onclick={() => void toggleMenu()}
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
      aria-label="Debug"
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
      aria-label="Launch settings"
      title="Launch settings…"
      onclick={() => ide.openEnvironmentsModal('configs')}
      data-testid="ide-edit-configs-btn"
    >
      <Icon name="gear" size={14} />
    </button>
  </div>
{:else}
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
  <div class="run-backdrop" role="presentation" onclick={closeMenu}></div>
  <div
    bind:this={popEl}
    class="run-popover"
    role="dialog"
    aria-label="Launch targets"
    style:inset-inline-start="{pos.left}px"
    style:top="{pos.top}px"
    style:inline-size="{pos.width}px"
    style:max-block-size="{pos.maxH}px"
    data-testid="ide-run-config-popover"
  >
    {#if ide.runConfigs.length > 4}
      <div class="run-filter-row">
        <Icon name="search" size={12} />
        <input
          bind:this={filterEl}
          class="run-filter-in"
          type="search"
          dir="ltr"
          aria-label="Filter launch targets"
          placeholder="Filter targets…"
          bind:value={filter}
        />
      </div>
    {/if}

    <div class="run-list">
      {#if savedConfigs.length > 0}
        <div class="run-sec">Saved configurations</div>
        {#each savedConfigs as cfg (cfg.id)}
          {@const active = selectedConfig?.id === cfg.id}
          <button
            type="button"
            class="run-item"
            class:is-active={active}
            onclick={() => selectTarget(cfg.id)}
          >
            <span class="run-item-name" dir="ltr">{cfg.name}</span>
            <Badge tone="neutral" variant="outline" label={cfg.runner} />
          </button>
        {/each}
      {/if}

      {#if detectedConfigs.length > 0}
        <div class="run-sec">Auto-detected</div>
        {#each detectedConfigs as cfg (cfg.id)}
          {@const active = selectedConfig?.id === cfg.id}
          <button
            type="button"
            class="run-item"
            class:is-active={active}
            onclick={() => selectTarget(cfg.id)}
          >
            <span class="run-item-name" dir="ltr">{cfg.name}</span>
            <Badge tone="info" variant="outline" label="auto" />
          </button>
        {/each}
      {/if}

      {#if savedConfigs.length === 0 && detectedConfigs.length === 0}
        <div class="run-empty">Runs the active editor file automatically, or create a named configuration below.</div>
      {/if}
    </div>

    <div class="run-footer">
      <button
        type="button"
        class="run-foot-btn"
        onclick={() => { closeMenu(); ide.openEnvironmentsModal('configs'); }}
      >
        <Icon name="gear" size={12} />
        <span>Edit configurations…</span>
      </button>
      <button
        type="button"
        class="run-foot-btn"
        onclick={() => { closeMenu(); ide.openEnvironmentsModal('env'); }}
      >
        <Icon name="terminal" size={12} />
        <span>Shell env &amp; vars…</span>
      </button>
      <button
        type="button"
        class="run-foot-btn"
        onclick={() => { closeMenu(); ide.toolchainsOpen = true; }}
      >
        <Icon name="box" size={12} />
        <span>SDKs…</span>
      </button>
    </div>
  </div>
{/if}

<style>
  .run-cluster {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-inline-size: 0;
  }
  .run-cfg-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-inline-size: min(260px, 24vw);
    min-inline-size: 0;
    block-size: 26px;
    padding-inline: 8px;
    background: var(--surface-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    font-weight: 500;
    cursor: pointer;
  }
  .run-cfg-btn:hover {
    border-color: var(--border-strong);
    background: var(--hover);
  }
  .run-cfg-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-inline-size: 0;
  }
  .run-backdrop {
    position: fixed;
    inset: 0;
    z-index: var(--z-popover-backdrop);
  }
  .run-popover {
    position: fixed;
    z-index: var(--z-popover);
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-m);
    box-shadow: var(--shadow-card);
    overflow: hidden;
  }
  .run-filter-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-block: 6px;
    padding-inline: 10px;
    border-block-end: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .run-filter-row:focus-within {
    border-color: var(--accent);
    box-shadow: inset 0 -1px 0 var(--accent);
  }
  .run-filter-in {
    flex: 1;
    min-inline-size: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-xs);
    outline: none;
  }
  .run-list {
    flex: 1;
    min-block-size: 0;
    overflow-y: auto;
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .run-sec {
    padding-block: 4px 2px;
    padding-inline: 6px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
  }
  .run-empty {
    padding: 8px 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .run-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding-block: 6px;
    padding-inline: 8px;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: var(--fs-xs);
    text-align: start;
    cursor: pointer;
  }
  .run-item:hover {
    background: var(--hover);
  }
  .run-item.is-active {
    background: var(--accent-soft);
    border-color: var(--accent-line);
    color: var(--accent-text);
    font-weight: 600;
  }
  .run-item-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-inline-size: 0;
  }
  .run-footer {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 6px;
    border-block-start: 1px solid var(--border);
    background: var(--surface-2);
  }
  .run-foot-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding-block: 4px;
    padding-inline: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .run-foot-btn:hover {
    background: var(--hover);
  }
  @media (max-width: 1024px) {
    .run-cfg-btn {
      max-inline-size: 150px;
    }
  }
</style>
