<script lang="ts">
  // Lazy-loaded Launch target selector popover for `IdeRunToolbar.svelte`.
  import { tick } from 'svelte';
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { ide, type IdeRunConfig } from '../ideStore.svelte';

  interface Props {
    triggerEl: HTMLButtonElement | undefined;
    selectedConfig: IdeRunConfig | null;
    onclose: () => void;
  }

  let { triggerEl, selectedConfig, onclose }: Props = $props();

  let filter = $state('');
  let popEl = $state<HTMLDivElement | undefined>(undefined);
  let filterEl = $state<HTMLInputElement | undefined>(undefined);
  let pos = $state({ left: 12, top: 48, width: 360, maxH: 400 });

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

  $effect(() => {
    clampPosition();
    void tick().then(() => {
      clampPosition();
      filterEl?.focus();
    });
  });

  $effect(() => {
    void savedConfigs.length;
    void detectedConfigs.length;
    requestAnimationFrame(() => clampPosition());
  });

  $effect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        onclose();
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
    onclose();
  }
</script>

<div class="run-backdrop" role="presentation" onclick={onclose}></div>
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
        <button type="button" class="run-item" class:is-active={active} onclick={() => selectTarget(cfg.id)}>
          <span class="run-item-name" dir="ltr">{cfg.name}</span>
          <Badge tone="neutral" variant="outline" label={cfg.runner} />
        </button>
      {/each}
    {/if}

    {#if detectedConfigs.length > 0}
      <div class="run-sec">Auto-detected</div>
      {#each detectedConfigs as cfg (cfg.id)}
        {@const active = selectedConfig?.id === cfg.id}
        <button type="button" class="run-item" class:is-active={active} onclick={() => selectTarget(cfg.id)}>
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
    <button type="button" class="run-foot-btn" onclick={() => { onclose(); ide.openEnvironmentsModal('configs'); }}>
      <Icon name="gear" size={12} />
      <span>Edit configurations…</span>
    </button>
    <button type="button" class="run-foot-btn" onclick={() => { onclose(); ide.openSettings('build.env'); }}>
      <Icon name="terminal" size={12} />
      <span>Shell env &amp; vars…</span>
    </button>
    <button type="button" class="run-foot-btn" onclick={() => { onclose(); ide.openSettings('project.sdks'); }}>
      <Icon name="box" size={12} />
      <span>SDKs…</span>
    </button>
  </div>
</div>

<style>
  .run-backdrop { position: fixed; inset: 0; z-index: var(--z-popover-backdrop); }
  .run-popover {
    position: fixed; z-index: var(--z-popover); display: flex; flex-direction: column;
    background: var(--surface); border: 1px solid var(--border-strong); border-radius: var(--radius-m); box-shadow: var(--shadow-card); overflow: hidden;
  }
  .run-filter-row { display: flex; align-items: center; gap: 6px; padding-block: 6px; padding-inline: 10px; border-block-end: 1px solid var(--border); background: var(--surface-2); color: var(--text-dim); }
  .run-filter-row:focus-within { border-color: var(--accent); box-shadow: inset 0 -1px 0 var(--accent); }
  .run-filter-in { flex: 1; min-inline-size: 0; border: 0; background: transparent; color: var(--text); font-size: var(--fs-xs); outline: none; }
  .run-list { flex: 1; min-block-size: 0; overflow-y: auto; padding: 6px; display: flex; flex-direction: column; gap: 2px; }
  .run-sec { padding-block: 4px 2px; padding-inline: 6px; font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim); }
  .run-empty { padding: 8px 6px; font-size: var(--fs-xs); color: var(--text-dim); }
  .run-item {
    display: flex; align-items: center; justify-content: space-between; gap: 8px; padding-block: 6px; padding-inline: 8px;
    border: 1px solid transparent; border-radius: var(--radius-s); background: transparent; color: var(--text); font-size: var(--fs-xs); text-align: start; cursor: pointer;
  }
  .run-item:hover { background: var(--hover); }
  .run-item.is-active { background: var(--accent-soft); border-color: var(--accent-line); color: var(--accent-text); font-weight: 600; }
  .run-item-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-inline-size: 0; }
  .run-footer { display: flex; flex-wrap: wrap; gap: 4px; padding: 6px; border-block-start: 1px solid var(--border); background: var(--surface-2); }
  .run-foot-btn {
    display: inline-flex; align-items: center; gap: 4px; padding-block: 4px; padding-inline: 8px;
    border: 1px solid var(--border); border-radius: var(--radius-s); background: var(--surface); color: var(--text); font-size: var(--fs-xs); cursor: pointer;
  }
  .run-foot-btn:hover { background: var(--hover); }
</style>
