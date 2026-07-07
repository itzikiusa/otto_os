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
      // Clamp INSIDE the viewport rather than flipping to the other side of
      // the cursor: a menu taller than the window would flip to a negative
      // top and render entirely off-screen (unreachable). CSS caps the menu
      // at the viewport height, so after clamping every item is scrollable.
      const pad = 8;
      cx = Math.max(pad, Math.min(ctxMenu.x, window.innerWidth - w - pad));
      cy = Math.max(pad, Math.min(ctxMenu.y, window.innerHeight - h - pad));
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

  // ── Filterable mode ────────────────────────────────────────────────────────
  // Order-preserving view: pinned items + separators always show; other items
  // must match the query, and at most `maxVisible` of them render at once (the
  // rest collapse into a "+N more" hint until the query narrows the list).
  const view = $derived.by(() => {
    const items = ctxMenu.items;
    if (!ctxMenu.filter) return { items, hidden: 0 };
    const q = ctxMenu.query.trim().toLowerCase();
    const cap = ctxMenu.maxVisible > 0 ? ctxMenu.maxVisible : Infinity;
    const out: typeof items = [];
    let shown = 0;
    let hidden = 0;
    for (const it of items) {
      if (it.pinned || it.separator || !it.label) {
        out.push(it);
        continue;
      }
      if (q !== '' && !it.label.toLowerCase().includes(q)) continue;
      if (shown < cap) {
        out.push(it);
        shown++;
      } else {
        hidden++;
      }
    }
    return { items: out, hidden };
  });

  // Auto-focus the search input when a filterable menu opens.
  let searchEl: HTMLInputElement | null = $state(null);
  $effect(() => {
    if (ctxMenu.open && ctxMenu.filter) {
      requestAnimationFrame(() => searchEl?.focus());
    }
  });

  /** Enter in the search box activates the first matched list item. */
  function onSearchKey(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      ctxMenu.close();
      return;
    }
    if (e.key !== 'Enter') return;
    e.preventDefault();
    const first = view.items.find((it) => !it.pinned && !it.separator && it.label && !it.disabled && it.action);
    if (first) clickItem(first);
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
    {#if ctxMenu.filter}
      <div class="ctx-search">
        <Icon name="search" size={12} />
        <input
          bind:this={searchEl}
          class="ctx-search-input"
          type="text"
          placeholder={ctxMenu.filterPlaceholder}
          bind:value={ctxMenu.query}
          spellcheck="false"
          onkeydown={onSearchKey}
        />
      </div>
    {/if}
    {#each view.items as item, i (i)}
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
    {#if view.hidden > 0}
      <div class="ctx-more">+{view.hidden} more — type to narrow</div>
    {/if}
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
    /* Long menus (e.g. the git "+" picker listing every registered repo) must
       scroll internally, never grow past the window. */
    max-height: calc(100vh - 16px);
    overflow-y: auto;
    overscroll-behavior: contain;
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
    text-align: start;
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

  /* Filterable-menu search row — sticky so it stays visible while the list
     below scrolls (the menu itself is the scroll container). */
  .ctx-search {
    position: sticky;
    top: -4px; /* cancel the menu's 4px padding so it hugs the top edge */
    z-index: 1;
    display: flex;
    align-items: center;
    gap: 6px;
    margin: -4px -4px 3px;
    padding: 7px 10px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    color: var(--text-dim);
  }
  .ctx-search-input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 12.5px;
    outline: none;
  }
  .ctx-more {
    padding: 5px 8px 4px;
    font-size: 11px;
    color: var(--text-dim);
    text-align: center;
  }
</style>
