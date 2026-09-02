<script lang="ts">
  // Shared service-view toolbar: title, client-side filter box, refresh +
  // auto-refresh (10 s) toggle, plus a slot for view-specific controls (region
  // switcher, state filter…). `/` focuses the filter from anywhere in the view.
  import type { Snippet } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    title: string;
    subtitle?: string;
    filter?: string;
    filterPlaceholder?: string;
    loading?: boolean;
    auto?: boolean;
    onrefresh?: () => void;
    children?: Snippet;
    actions?: Snippet;
  }
  let {
    title,
    subtitle = '',
    filter = $bindable(''),
    filterPlaceholder = 'Filter…',
    loading = false,
    auto = $bindable(false),
    onrefresh,
    children,
    actions,
  }: Props = $props();

  let filterEl = $state<HTMLInputElement | null>(null);

  // Auto-refresh timer: re-armed whenever the toggle flips; cleared on unmount.
  $effect(() => {
    if (!auto || !onrefresh) return;
    const fn = onrefresh;
    const t = setInterval(() => fn(), 10_000);
    return () => clearInterval(t);
  });

  function onKey(e: KeyboardEvent): void {
    if (e.key !== '/' || e.metaKey || e.ctrlKey || e.altKey) return;
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
    if (t?.closest('.cm-editor')) return;
    e.preventDefault();
    filterEl?.focus();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="vt">
  <div class="vt-title">
    <h2>{title}</h2>
    {#if subtitle}<span class="sub">{subtitle}</span>{/if}
  </div>
  {#if children}<div class="vt-extra">{@render children()}</div>{/if}
  <label class="vt-filter">
    <Icon name="search" size={13} />
    <input
      bind:this={filterEl}
      bind:value={filter}
      type="search"
      placeholder={filterPlaceholder}
      aria-label={filterPlaceholder}
    />
  </label>
  {#if actions}{@render actions()}{/if}
  {#if onrefresh}
    <button
      class="icon-btn"
      class:spin={loading}
      onclick={() => onrefresh()}
      title="Refresh"
      aria-label="Refresh"
      disabled={loading}
    >
      <Icon name="refresh" size={14} />
    </button>
    <label class="auto" title="Auto-refresh every 10 s">
      <input type="checkbox" bind:checked={auto} />
      <span>Auto</span>
    </label>
  {/if}
</div>

<style>
  .vt {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    position: sticky;
    top: 0;
    z-index: 3;
  }
  .vt-title {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    white-space: nowrap;
  }
  .sub {
    font-size: 11.5px;
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .vt-extra {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .vt-filter {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1 1 160px;
    min-width: 120px;
    max-width: 320px;
    margin-left: auto;
    padding: 0 8px;
    height: 28px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
    color: var(--text-dim);
  }
  .vt-filter input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 12.5px;
    outline: none;
  }
  .icon-btn {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text);
    cursor: pointer;
  }
  .icon-btn:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .icon-btn.spin :global(svg) {
    animation: spin 0.9s linear infinite;
  }
  .auto {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11.5px;
    color: var(--text-dim);
    cursor: pointer;
    user-select: none;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (max-width: 640px) {
    .vt-filter {
      flex-basis: 100%;
      max-width: none;
      margin-left: 0;
      order: 10;
    }
  }
</style>
