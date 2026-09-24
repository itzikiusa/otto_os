<script lang="ts">
  // Global context menu overlay — mount once in App.svelte.
  //
  // Positioning: a cursor menu (right-click) opens at the pointer; an anchored
  // menu (ctxMenu.showAt / a button click) opens under its trigger, lined up
  // with the trigger's start or end edge (mirrored in RTL), flips above only
  // when it fits there, and is otherwise clamped into the window. The height
  // cap is measured against window.innerHeight — never vh, which in the
  // WKWebView resolves to the SCREEN height.
  //
  // Keyboard: focus moves to the first item on open (the search box in a
  // filterable menu); ↑/↓ Home/End move, Enter/Space activate, a letter jumps
  // to the next item starting with it, Esc/Tab close. Focus returns to the
  // trigger on close (see ctxMenu.close()).
  import { tick } from 'svelte';
  import Icon, { asIcon } from './Icon.svelte';
  import { ctxMenu, type MenuItem } from '../contextmenu.svelte';

  const PAD = 8; // viewport margin
  const GAP = 4; // trigger ↔ menu

  let menuEl: HTMLDivElement | null = $state(null);

  // Final position + height cap. `ready` keeps the menu invisible for the one
  // frame before it has been measured, so it never flashes at a wrong spot.
  let cx = $state(0);
  let cy = $state(0);
  let maxH = $state(0);
  let ready = $state(false);

  const isRtl = (): boolean => document.documentElement.dir === 'rtl';

  function place(): void {
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    maxH = Math.max(80, vh - PAD * 2);
    if (!menuEl) return;
    const w = menuEl.offsetWidth;
    const h = Math.min(menuEl.offsetHeight, maxH);
    const a = ctxMenu.anchor;
    let x: number;
    let y: number;
    if (a) {
      // Logical alignment: 'start' = the trigger's leading edge.
      const alignLeft = (ctxMenu.align === 'start') !== isRtl();
      x = alignLeft ? a.left : a.right - w;
      const below = a.bottom + GAP;
      const above = a.top - GAP - h;
      // Prefer below; flip above only when it fits there entirely.
      y = below + h > vh - PAD && above >= PAD ? above : below;
    } else {
      x = isRtl() ? ctxMenu.x - w : ctxMenu.x;
      y = ctxMenu.y;
    }
    // Clamp INSIDE the viewport rather than flipping without a floor: a menu
    // taller than the window would flip to a negative top and be unreachable.
    cx = Math.max(PAD, Math.min(x, vw - w - PAD));
    cy = Math.max(PAD, Math.min(y, vh - h - PAD));
  }

  $effect(() => {
    if (!ctxMenu.open) return;
    void ctxMenu.seq; // re-run for a menu re-opened in the same tick
    ready = false;
    maxH = Math.max(80, window.innerHeight - PAD * 2);
    // Defer one frame so the menu has rendered and can be measured.
    const raf = requestAnimationFrame(async () => {
      place();
      ready = true;
      await tick();
      if (ctxMenu.filter) searchEl?.focus();
      else focusAt(0);
    });
    return () => cancelAnimationFrame(raf);
  });

  // ── Roving focus ───────────────────────────────────────────────────────────
  function itemEls(): HTMLButtonElement[] {
    return menuEl ? Array.from(menuEl.querySelectorAll<HTMLButtonElement>('.ctx-item:not(:disabled)')) : [];
  }
  function focusAt(i: number): void {
    const els = itemEls();
    if (!els.length) {
      menuEl?.focus();
      return;
    }
    els[(i + els.length) % els.length].focus();
  }
  function move(delta: number): void {
    const els = itemEls();
    const cur = els.indexOf(document.activeElement as HTMLButtonElement);
    if (cur === -1) focusAt(delta > 0 ? 0 : -1);
    else if (ctxMenu.filter && cur === 0 && delta < 0) searchEl?.focus();
    else focusAt(cur + delta);
  }
  function typeAhead(ch: string): void {
    const els = itemEls();
    if (!els.length) return;
    const start = els.indexOf(document.activeElement as HTMLButtonElement);
    for (let k = 1; k <= els.length; k++) {
      const el = els[(start + k) % els.length];
      if ((el.textContent ?? '').trim().toLowerCase().startsWith(ch)) {
        el.focus();
        return;
      }
    }
  }

  // Keys are handled on the window in the CAPTURE phase while the menu is
  // open: whatever holds focus (a terminal after a right-click, the menu, the
  // search box) the menu owns the navigation keys, and the underlying pane
  // does not also react to them.
  function onWindowKey(e: KeyboardEvent): void {
    if (!ctxMenu.open) return;
    const inSearch = !!searchEl && document.activeElement === searchEl;
    const swallow = (): void => {
      e.preventDefault();
      e.stopPropagation();
    };
    switch (e.key) {
      case 'Escape':
        swallow();
        ctxMenu.close();
        return;
      case 'Tab':
        swallow();
        ctxMenu.close();
        return;
      case 'ArrowDown':
        swallow();
        if (inSearch) focusAt(0);
        else move(1);
        return;
      case 'ArrowUp':
        swallow();
        if (!inSearch) move(-1);
        return;
      case 'Home':
      case 'End':
        if (inSearch) return; // caret movement in the field
        swallow();
        focusAt(e.key === 'Home' ? 0 : -1);
        return;
      case 'Enter':
      case ' ': {
        if (inSearch) return; // Enter → onSearchKey; Space types a space
        swallow();
        const el = document.activeElement;
        if (el instanceof HTMLButtonElement && el.closest('.ctx-menu')) el.click();
        return;
      }
    }
    if (inSearch || e.metaKey || e.ctrlKey || e.altKey || e.key.length !== 1) return;
    swallow();
    if (ctxMenu.filter && searchEl) {
      // Typing on a row of a filterable menu goes to its search box.
      searchEl.focus();
      ctxMenu.query += e.key;
    } else {
      typeAhead(e.key.toLowerCase());
    }
  }

  function clickItem(item: MenuItem): void {
    if (item.disabled) return;
    // Close FIRST: an action may open another menu (a ⋯ row that clicks a
    // collapsed "New ▾" button) and must not be closed right after.
    ctxMenu.close();
    item.action?.();
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

  let searchEl: HTMLInputElement | null = $state(null);

  /** Enter in the search box activates the first matched list item. */
  function onSearchKey(e: KeyboardEvent): void {
    if (e.key !== 'Enter') return;
    e.preventDefault();
    const first = view.items.find((it) => !it.pinned && !it.separator && it.label && !it.disabled && it.action);
    if (first) clickItem(first);
  }
</script>

<svelte:window onkeydowncapture={onWindowKey} onresize={() => ctxMenu.open && place()} />

{#if ctxMenu.open}
  <!-- Backdrop: transparent, full-screen, closes menu on any interaction -->
  <div
    class="ctx-backdrop"
    onclick={() => ctxMenu.close()}
    oncontextmenu={(e) => { e.preventDefault(); ctxMenu.close(); }}
    onwheel={() => ctxMenu.close()}
    role="presentation"
  ></div>

  <div
    bind:this={menuEl}
    class="ctx-menu glass-raised"
    class:ctx-ready={ready}
    style="left:{cx}px;top:{cy}px;max-height:{maxH}px"
    role="menu"
    aria-label="Context menu"
    tabindex="-1"
  >
    {#if ctxMenu.filter}
      <div class="ctx-search">
        <Icon name="search" size={12} />
        <input
          bind:this={searchEl}
          class="ctx-search-input"
          type="text"
          placeholder={ctxMenu.filterPlaceholder}
          aria-label={ctxMenu.filterPlaceholder}
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
          role={item.checked === undefined ? 'menuitem' : 'menuitemcheckbox'}
          aria-checked={item.checked === undefined ? undefined : item.checked}
          tabindex="-1"
          onclick={() => clickItem(item)}
          onmousemove={(e) => {
            // Pointer and keyboard share one highlight: hovering moves focus.
            const el = e.currentTarget;
            if (document.activeElement !== el && document.activeElement !== searchEl) el.focus({ preventScroll: true });
          }}
        >
          {#if item.checked !== undefined}
            <!-- Check column (checkable rows), then the row's own icon if any. -->
            {#if item.checked}
              <span class="ctx-icon ctx-check"><Icon name="check" size={13} /></span>
            {:else}
              <span class="ctx-icon-gap"></span>
            {/if}
            {#if item.icon}<span class="ctx-icon"><Icon name={asIcon(item.icon)} size={13} /></span>{/if}
          {:else if item.icon}
            <span class="ctx-icon"><Icon name={asIcon(item.icon)} size={13} /></span>
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
    z-index: var(--z-popover-backdrop);
  }

  .ctx-menu {
    position: fixed;
    z-index: var(--z-popover);
    min-width: 160px;
    max-width: 260px;
    /* Raised glass (tokens.css .glass-raised): tint, blur, --glass-border
       hairline, --glass-shadow. */
    border-radius: var(--radius-m);
    padding: 4px;
    display: flex;
    flex-direction: column;
    /* Long menus (e.g. the git "+" picker listing every registered repo) must
       scroll internally, never grow past the window: max-height is set inline
       from window.innerHeight. */
    overflow-y: auto;
    overscroll-behavior: contain;
    outline: none;
    visibility: hidden;
  }
  .ctx-menu.ctx-ready {
    visibility: visible;
  }

  .ctx-item {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    min-height: 26px;
    padding-block: 0;
    padding-inline: 6px 8px;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-m);
    cursor: pointer;
    text-align: start;
    outline: none;
    transition: background 80ms ease-out;
    flex-shrink: 0;
  }

  /* Hover and keyboard focus are one highlight (hover moves focus). */
  .ctx-item:hover:not(.disabled),
  .ctx-item:focus:not(.disabled) {
    background: var(--hover);
  }
  .ctx-item:focus-visible {
    box-shadow: inset 0 0 0 2px var(--accent);
  }

  .ctx-item.danger {
    color: var(--danger);
  }

  .ctx-item.danger:hover:not(.disabled),
  .ctx-item.danger:focus:not(.disabled) {
    background: var(--danger-soft);
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
  .ctx-check {
    color: var(--accent-text);
  }

  .ctx-item.danger .ctx-icon {
    color: var(--danger);
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
    flex-shrink: 0;
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
    flex-shrink: 0;
  }
  .ctx-search-input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-m);
    outline: none;
  }
  .ctx-more {
    padding: 5px 8px 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    text-align: center;
  }
</style>
