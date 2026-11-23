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
  let subEl: HTMLDivElement | null = $state(null);

  // Final position + height cap. `ready` keeps the menu invisible for the one
  // frame before it has been measured, so it never flashes at a wrong spot.
  let cx = $state(0);
  let cy = $state(0);
  let maxH = $state(0);
  let ready = $state(false);

  // Cascading 1-level flyout submenu (`item.children`).
  let openSubIndex = $state(-1);
  let subAnchorRect: DOMRect | null = null;
  let subX = $state(0);
  let subY = $state(0);
  let subReady = $state(false);

  const isRtl = (): boolean => document.documentElement.dir === 'rtl';

  function hasChildren(item: MenuItem | undefined): boolean {
    return Boolean(item && !item.disabled && item.children && item.children.length > 0);
  }

  function placeSub(): void {
    const vw = window.visualViewport?.width ?? window.innerWidth;
    const vh = window.visualViewport?.height ?? window.innerHeight;
    maxH = Math.max(80, Math.round(vh) - PAD * 2);
    if (!subEl || !subAnchorRect) return;
    const w = subEl.offsetWidth;
    const h = Math.min(subEl.offsetHeight, maxH);
    const rtl = isRtl();
    const rightX = subAnchorRect.right + 2;
    const leftX = subAnchorRect.left - 2 - w;
    let x: number;
    if (!rtl) {
      x = rightX + w <= vw - PAD || leftX < PAD ? rightX : leftX;
    } else {
      x = leftX >= PAD || rightX + w > vw - PAD ? leftX : rightX;
    }
    const y = subAnchorRect.top - 4;
    subX = Math.max(PAD, Math.min(x, vw - w - PAD));
    subY = Math.max(PAD, Math.min(y, vh - h - PAD));
  }

  function openSubmenuAt(index: number, rowEl: HTMLElement | null, focusFirst = false): void {
    const item = view.items[index];
    if (!hasChildren(item) || !rowEl) {
      openSubIndex = -1;
      subAnchorRect = null;
      subReady = false;
      return;
    }
    subAnchorRect = rowEl.getBoundingClientRect();
    if (openSubIndex !== index) {
      subReady = false;
      openSubIndex = index;
    }
    requestAnimationFrame(async () => {
      placeSub();
      subReady = true;
      if (focusFirst) {
        await tick();
        focusSubAt(0);
      }
    });
  }

  function closeSubmenu(refocusParent = false): void {
    const prevIdx = openSubIndex;
    openSubIndex = -1;
    subAnchorRect = null;
    subReady = false;
    if (refocusParent && prevIdx >= 0 && menuEl) {
      const parentBtn = menuEl.querySelector<HTMLButtonElement>(`.ctx-item[data-ctx-idx="${prevIdx}"]`);
      parentBtn?.focus({ preventScroll: true });
    }
  }

  function place(): void {
    const vw = window.visualViewport?.width ?? window.innerWidth;
    const vh = window.visualViewport?.height ?? window.innerHeight;
    maxH = Math.max(80, Math.round(vh) - PAD * 2);
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
    if (!ctxMenu.open) {
      openSubIndex = -1;
      subAnchorRect = null;
      subReady = false;
      return;
    }
    void ctxMenu.seq; // re-run for a menu re-opened in the same tick
    ready = false;
    openSubIndex = -1;
    subAnchorRect = null;
    subReady = false;
    maxH = Math.max(80, Math.round(window.visualViewport?.height ?? window.innerHeight) - PAD * 2);
    // Defer one frame so the menu has rendered and can be measured.
    const raf = requestAnimationFrame(async () => {
      place();
      ready = true;
      await tick();
      if (ctxMenu.filter) searchEl?.focus({ preventScroll: true });
      else focusAt(0);
    });
    return () => cancelAnimationFrame(raf);
  });

  // ── Roving focus ───────────────────────────────────────────────────────────
  function itemEls(root: HTMLElement | null = menuEl): HTMLButtonElement[] {
    return root ? Array.from(root.querySelectorAll<HTMLButtonElement>(':scope > .ctx-item:not([aria-disabled="true"])')) : [];
  }
  function focusAt(i: number, root: HTMLElement | null = menuEl): void {
    const els = itemEls(root);
    if (!els.length) {
      root?.focus({ preventScroll: true });
      return;
    }
    els[(i + els.length) % els.length].focus({ preventScroll: true });
  }
  function focusSubAt(i: number): void {
    focusAt(i, subEl);
  }
  function move(delta: number, root: HTMLElement | null = menuEl): void {
    const els = itemEls(root);
    const cur = els.indexOf(document.activeElement as HTMLButtonElement);
    if (cur === -1) focusAt(delta > 0 ? 0 : -1, root);
    else if (root === menuEl && ctxMenu.filter && cur === 0 && delta < 0) searchEl?.focus({ preventScroll: true });
    else focusAt(cur + delta, root);
  }
  function typeAhead(ch: string, root: HTMLElement | null = menuEl): void {
    const els = itemEls(root);
    if (!els.length) return;
    const start = els.indexOf(document.activeElement as HTMLButtonElement);
    for (let k = 1; k <= els.length; k++) {
      const el = els[(start + k) % els.length];
      if ((el.textContent ?? '').trim().toLowerCase().startsWith(ch)) {
        el.focus({ preventScroll: true });
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
    const inSub = !!subEl && subEl.contains(document.activeElement);
    const activeRoot = inSub ? subEl : menuEl;
    const openKey = isRtl() ? 'ArrowLeft' : 'ArrowRight';
    const closeKey = isRtl() ? 'ArrowRight' : 'ArrowLeft';
    const swallow = (): void => {
      e.preventDefault();
      e.stopPropagation();
    };
    switch (e.key) {
      case 'Escape':
        swallow();
        if (inSub) closeSubmenu(true);
        else ctxMenu.close();
        return;
      case 'Tab':
        swallow();
        ctxMenu.close();
        return;
      case 'ArrowDown':
        swallow();
        if (inSearch) focusAt(0);
        else move(1, activeRoot);
        return;
      case 'ArrowUp':
        swallow();
        if (!inSearch) move(-1, activeRoot);
        return;
      case openKey: {
        if (inSearch || inSub) return;
        const activeBtn = document.activeElement;
        if (activeBtn instanceof HTMLButtonElement && menuEl?.contains(activeBtn)) {
          const idx = Number(activeBtn.dataset.ctxIdx ?? '-1');
          if (idx >= 0 && hasChildren(view.items[idx])) {
            swallow();
            openSubmenuAt(idx, activeBtn, true);
          }
        }
        return;
      }
      case closeKey: {
        if (inSub) {
          swallow();
          closeSubmenu(true);
        }
        return;
      }
      case 'Home':
      case 'End':
        if (inSearch) return; // caret movement in the field
        swallow();
        focusAt(e.key === 'Home' ? 0 : -1, activeRoot);
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
    if (!inSub && ctxMenu.filter && searchEl) {
      // Typing on a row of a filterable menu goes to its search box.
      searchEl.focus({ preventScroll: true });
      ctxMenu.query += e.key;
    } else {
      typeAhead(e.key.toLowerCase(), activeRoot);
    }
  }

  /** The menu caps at 260px: a long repo / branch / session name ellipsizes,
   *  so hovering a clipped label shows it whole (unclipped rows get none). An
   *  item's own `title` (e.g. why it is disabled) always wins. */
  function titleIfClipped(e: MouseEvent, item: MenuItem): void {
    const row = e.currentTarget as HTMLElement;
    if (item.title) {
      row.title = item.title;
      return;
    }
    const el = row.querySelector<HTMLElement>('.ctx-label');
    if (!el) return;
    row.title = el.scrollWidth > el.clientWidth + 1 ? (el.textContent ?? '') : '';
  }

  /** Conditional rows (`...(cond ? [...] : [])`) leave separators at the
   *  edges or doubled up; drop those so every menu reads clean. */
  function tidy(items: MenuItem[]): MenuItem[] {
    const out: MenuItem[] = [];
    for (const it of items) {
      const sep = it.separator || !it.label;
      if (sep && (out.length === 0 || out[out.length - 1].separator || !out[out.length - 1].label)) continue;
      out.push(it);
    }
    while (out.length && (out[out.length - 1].separator || !out[out.length - 1].label)) out.pop();
    return out;
  }

  function clickItem(item: MenuItem, index?: number, rowEl?: HTMLElement | null): void {
    if (item.disabled) return;
    if (hasChildren(item) && index !== undefined && rowEl) {
      openSubmenuAt(index, rowEl, true);
      return;
    }
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
    if (!ctxMenu.filter) return { items: tidy(items), hidden: 0 };
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
    return { items: tidy(out), hidden };
  });

  const activeSubItems = $derived.by((): MenuItem[] => {
    if (openSubIndex < 0 || openSubIndex >= view.items.length) return [];
    const parent = view.items[openSubIndex];
    return hasChildren(parent) ? tidy(parent.children ?? []) : [];
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

<svelte:window
  onkeydowncapture={onWindowKey}
  onresize={() => {
    if (!ctxMenu.open) return;
    place();
    if (openSubIndex >= 0) placeSub();
  }}
/>

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
    onscroll={() => { if (openSubIndex >= 0) closeSubmenu(false); }}
  >
    {#if ctxMenu.filter}
      <div class="ctx-search">
        <Icon name="search" size={12} />
        <input dir="ltr"
          bind:this={searchEl}
          class="ctx-search-input"
          type="text"
          placeholder={ctxMenu.filterPlaceholder}
          aria-label={ctxMenu.filterPlaceholder}
          bind:value={ctxMenu.query}
          spellcheck="false"
          onkeydown={onSearchKey}
          oninput={() => { if (openSubIndex >= 0) closeSubmenu(false); }}
        />
      </div>
    {/if}
    {#each view.items as item, i (i)}
      {#if item.separator || !item.label}
        <div class="ctx-sep" role="separator"></div>
      {:else}
        {@const itemHasSub = hasChildren(item)}
        <button
          class="ctx-item"
          class:danger={item.danger}
          class:disabled={item.disabled}
          class:has-open-sub={openSubIndex === i}
          data-ctx-idx={i}
          aria-disabled={item.disabled ? 'true' : undefined}
          aria-haspopup={itemHasSub ? 'menu' : undefined}
          aria-expanded={itemHasSub ? openSubIndex === i : undefined}
          role={item.checked === undefined ? 'menuitem' : 'menuitemcheckbox'}
          aria-checked={item.checked === undefined ? undefined : item.checked}
          tabindex="-1"
          onclick={(e) => clickItem(item, i, e.currentTarget)}
          onmouseenter={(e) => {
            titleIfClipped(e, item);
            if (itemHasSub) openSubmenuAt(i, e.currentTarget, false);
            else if (openSubIndex >= 0) closeSubmenu(false);
          }}
          onmousemove={(e) => {
            // Pointer and keyboard share one highlight: hovering moves focus
            // (not onto a disabled row — it only shows its tooltip).
            const el = e.currentTarget;
            if (item.disabled) return;
            if (document.activeElement !== el && document.activeElement !== searchEl) el.focus({ preventScroll: true });
          }}
        >
          <!-- One leading column: the row's icon, else a checkable row's check.
               A checkable row WITH an icon shows its check at the end, so it
               lines up with its neighbours (no second, empty column). -->
          {#if item.icon}
            <span class="ctx-icon"><Icon name={asIcon(item.icon)} size={13} /></span>
          {:else if item.checked}
            <span class="ctx-icon ctx-check"><Icon name="check" size={13} /></span>
          {:else}
            <span class="ctx-icon-gap"></span>
          {/if}
          <span class="ctx-label">{item.label}</span>
          {#if item.icon && item.checked}<span class="ctx-icon ctx-check" data-check-end><Icon name="check" size={13} /></span>{/if}
          {#if itemHasSub}
            <span class="ctx-chevron"><Icon name="chevronRight" size={12} /></span>
          {:else if item.hint}
            <span class="ctx-hint">{item.hint}</span>
          {/if}
        </button>
      {/if}
    {/each}
    {#if view.hidden > 0}
      <div class="ctx-more">+{view.hidden} more — type to narrow</div>
    {/if}
  </div>

  {#if activeSubItems.length > 0}
    <div
      bind:this={subEl}
      class="ctx-menu ctx-submenu glass-raised"
      class:ctx-ready={subReady}
      style="left:{subX}px;top:{subY}px;max-height:{maxH}px"
      role="menu"
      aria-label={view.items[openSubIndex]?.label ?? 'Submenu'}
      tabindex="-1"
    >
      {#each activeSubItems as subItem, si (si)}
        {#if subItem.separator || !subItem.label}
          <div class="ctx-sep" role="separator"></div>
        {:else}
          <button
            class="ctx-item"
            class:danger={subItem.danger}
            class:disabled={subItem.disabled}
            aria-disabled={subItem.disabled ? 'true' : undefined}
            role={subItem.checked === undefined ? 'menuitem' : 'menuitemcheckbox'}
            aria-checked={subItem.checked === undefined ? undefined : subItem.checked}
            tabindex="-1"
            onclick={() => clickItem(subItem)}
            onmouseenter={(e) => titleIfClipped(e, subItem)}
            onmousemove={(e) => {
              const el = e.currentTarget;
              if (subItem.disabled) return;
              if (document.activeElement !== el) el.focus({ preventScroll: true });
            }}
          >
            {#if subItem.icon}
              <span class="ctx-icon"><Icon name={asIcon(subItem.icon)} size={13} /></span>
            {:else if subItem.checked}
              <span class="ctx-icon ctx-check"><Icon name="check" size={13} /></span>
            {:else}
              <span class="ctx-icon-gap"></span>
            {/if}
            <span class="ctx-label">{subItem.label}</span>
            {#if subItem.icon && subItem.checked}<span class="ctx-icon ctx-check" data-check-end><Icon name="check" size={13} /></span>{/if}
            {#if subItem.hint}<span class="ctx-hint">{subItem.hint}</span>{/if}
          </button>
        {/if}
      {/each}
    </div>
  {/if}
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
  /* Shown once positioned (ctx-ready), with the shared entrance fade. */
  /* Focus rests on the menu itself only until an item takes it (keyboard open
     with nothing selected): a hairline ring says where keys go. */
  .ctx-menu:focus-visible {
    box-shadow: var(--glass-shadow), inset 0 0 0 1px var(--accent-text);
  }
  .ctx-menu.ctx-ready {
    visibility: visible;
    animation: otto-fade-in var(--dur-enter) var(--ease-out);
  }

  .ctx-item {
    display: flex;
    align-items: center;
    gap: 8px;
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
    transition: background var(--dur-fast) ease-out;
    flex-shrink: 0;
  }

  /* Hover and keyboard focus are one highlight (hover moves focus). */
  .ctx-item:hover:not(.disabled),
  .ctx-item:focus:not(.disabled),
  .ctx-item.has-open-sub:not(.disabled) {
    background: var(--hover);
  }
  .ctx-item:focus-visible {
    box-shadow: inset 0 0 0 2px var(--accent-text);
  }
  .ctx-chevron {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
    margin-inline-start: 10px;
    color: var(--text-dim);
  }

  .ctx-item.danger {
    color: var(--danger);
  }

  .ctx-item.danger:hover:not(.disabled),
  .ctx-item.danger:focus:not(.disabled) {
    background: var(--danger-soft);
  }

  .ctx-item.disabled {
    opacity: var(--disabled-opacity);
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
  /* Over a conversation on a touch screen the menu is a surface, not glass:
     text behind a translucent menu showed through its rows (vibrancy is for
     chrome only). */
  @media (pointer: coarse), (max-width: 1024px) {
    .ctx-menu {
      background: var(--surface);
      backdrop-filter: none;
      -webkit-backdrop-filter: none;
      max-width: min(300px, calc(100vw - 16px));
    }
  }
  @media (pointer: coarse) {
    .ctx-item {
      min-height: 40px;
      padding-block: 6px;
      padding-inline: 10px 12px;
    }
  }

  .ctx-item.danger .ctx-icon {
    color: var(--danger);
  }

  .ctx-icon-gap {
    width: 13px;
    flex-shrink: 0;
  }

  /* The label gives way first (its large shrink factor) but keeps at least 40%
     of the row; only then does a long hint (a path) ellipsise. Before, the hint
     never shrank and squeezed long labels to a one-letter stub. */
  .ctx-label {
    flex: 1 1000 auto;
    min-inline-size: 40%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Trailing shortcut / qualifier: shrinks (with an ellipsis) only after the
     label reached its floor, so a shortcut beside a long label stays whole. */
  .ctx-hint {
    flex: 0 1 auto;
    min-inline-size: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    margin-inline-start: 12px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .ctx-item.disabled .ctx-hint {
    opacity: 1;
  }
  .ctx-sep {
    height: 1px;
    background: var(--border);
    margin: 4px;
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
    margin: -4px -4px 4px;
    padding: 8px 12px;
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
  /* The search field is borderless in its row; the row shows the focus. */
  .ctx-search:focus-within {
    box-shadow: inset 0 -2px 0 var(--accent-text);
  }
  .ctx-more {
    padding: 4px 8px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    text-align: center;
  }
</style>
