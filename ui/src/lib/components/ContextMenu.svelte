<script lang="ts">
  // Global context menu overlay — mount once in App.svelte.
  import Icon from './Icon.svelte';
  import { ctxMenu } from '../contextmenu.svelte';

  // DOM reference for clamping
  let menuEl: HTMLDivElement | null = $state(null);

  // Clamped position, recomputed whenever open/position changes
  let cx = $state(0);
  let cy = $state(0);

  $effect(() => {
    if (!ctxMenu.open) return;
    // Defer one tick so the menu has been rendered and we can read its size
    requestAnimationFrame(() => {
      if (!menuEl) {
        cx = ctxMenu.x;
        cy = ctxMenu.y;
        return;
      }
      const w = menuEl.offsetWidth;
      const h = menuEl.offsetHeight;
      cx = ctxMenu.x + w > window.innerWidth ? ctxMenu.x - w : ctxMenu.x;
      cy = ctxMenu.y + h > window.innerHeight ? ctxMenu.y - h : ctxMenu.y;
    });
    cx = ctxMenu.x;
    cy = ctxMenu.y;
  });

  function handleBackdropKey(e: KeyboardEvent): void {
    if (e.key === 'Escape') ctxMenu.close();
  }

  function clickItem(item: typeof ctxMenu.items[number]): void {
    if (item.disabled) return;
    item.action?.();
    ctxMenu.close();
  }
</script>

{#if ctxMenu.open}
  <!-- Backdrop: transparent, full-screen, closes menu on any interaction -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="ctx-backdrop"
    onclick={() => ctxMenu.close()}
    oncontextmenu={(e) => { e.preventDefault(); ctxMenu.close(); }}
    onkeydown={handleBackdropKey}
    onwheel={() => ctxMenu.close()}
    role="presentation"
  ></div>

  <div
    bind:this={menuEl}
    class="ctx-menu"
    style="left:{cx}px;top:{cy}px"
    role="menu"
    aria-label="Context menu"
  >
    {#each ctxMenu.items as item, i (i)}
      {#if item.separator || !item.label}
        <div class="ctx-sep" role="separator"></div>
      {:else}
        <button
          class="ctx-item"
          class:danger={item.danger}
          class:disabled={item.disabled}
          disabled={item.disabled}
          role="menuitem"
          onclick={() => clickItem(item)}
        >
          {#if item.icon}
            <span class="ctx-icon"><Icon name={item.icon} size={13} /></span>
          {:else}
            <span class="ctx-icon-gap"></span>
          {/if}
          <span class="ctx-label">{item.label}</span>
        </button>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .ctx-backdrop {
    position: fixed;
    inset: 0;
    z-index: 9998;
  }

  .ctx-menu {
    position: fixed;
    z-index: 9999;
    min-width: 160px;
    max-width: 260px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: var(--shadow);
    padding: 4px;
    display: flex;
    flex-direction: column;
    /* slightly translucent backdrop effect */
    backdrop-filter: blur(12px) saturate(1.3);
    -webkit-backdrop-filter: blur(12px) saturate(1.3);
  }

  .ctx-item {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    height: 26px;
    padding: 0 8px 0 6px;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: 12.5px;
    cursor: pointer;
    text-align: left;
    transition: background 80ms ease-out;
  }

  .ctx-item:hover:not(.disabled) {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }

  .ctx-item.danger {
    color: var(--status-exited);
  }

  .ctx-item.danger:hover:not(.disabled) {
    background: color-mix(in srgb, var(--status-exited) 14%, transparent);
  }

  .ctx-item.disabled {
    opacity: 0.4;
    cursor: default;
  }

  .ctx-icon {
    display: flex;
    align-items: center;
    color: var(--text-dim);
    flex-shrink: 0;
  }

  .ctx-item.danger .ctx-icon {
    color: var(--status-exited);
  }

  .ctx-icon-gap {
    width: 13px;
    flex-shrink: 0;
  }

  .ctx-label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ctx-sep {
    height: 1px;
    background: var(--border);
    margin: 3px 4px;
  }
</style>
