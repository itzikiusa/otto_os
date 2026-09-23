<script lang="ts">
  // Shared page chrome: the ONE header row every top-level module page renders
  // (the Agents page keeps its session TabBar instead). Modelled on the macOS
  // unified toolbar — title + actions in a single fixed-height bar:
  //
  //   [icon] Title  [badge]  [tabs (inline)]        [secondary…] [primary] [⋯]
  //          subtitle
  //
  // • Fixed height (--ph-h, 46px) and one title size, so every page's chrome
  //   lines up when you move between modules.
  // • It is the window's drag surface in the Tauri shell (like TabBar):
  //   `startWindowDrag` bails when the mousedown lands on a control, so the
  //   buttons inside keep working. Pads past the traffic lights when the
  //   sidebar is collapsed (the Rail is narrower than the inset).
  // • ACTION OVERFLOW: the `actions` snippet renders its controls as direct
  //   children of `.ph-actions`. When they don't fit, the lowest-priority ones
  //   collapse into a "⋯" menu (the global, viewport-clamped ctxMenu) instead
  //   of wrapping onto a second line. Collapse order: `data-overflow` value
  //   (a number — lower collapses first; default 0), then right-to-left.
  //   Never collapsed: `.primary` buttons, `[data-keep]`, and anything that
  //   holds a form field (select/input) — a menu row can't host those.
  //   A collapsed button becomes a menu row labelled by `data-label`,
  //   `aria-label`, `title` or its text (icon from `data-icon`); choosing it
  //   `.click()`s the hidden original, so its handler/state stay the source of
  //   truth. A wrapper child (e.g. a button group) becomes one row per button.
  // • `tabs` renders a segmented/tab control either INLINE (in the bar, after
  //   the title — the native-toolbar look) or BELOW (a second row directly
  //   under the bar, for wide tab sets).
  // • `crumbs` draws a breadcrumb trail before the title (sub-pages such as
  //   Kubernetes / Monitor / Fleet); `leading` hosts a back button for detail
  //   views; `titleContent` swaps the title text for richer content (a
  //   switcher button) while `title` stays the accessible name.
  //
  // Pair it with <PageBody> (the one content-width rule) and, for an empty
  // page, <EmptyState variant="page">. One primary action per page: while a
  // page's list is empty, the EmptyState owns the "New …" CTA and the header
  // omits it.
  import type { Snippet } from 'svelte';
  import { onMount } from 'svelte';
  import Icon from './Icon.svelte';
  import { ui, isTauri } from '../stores/ui.svelte';
  import { viewport } from '../stores/viewport.svelte';
  import { startWindowDrag } from '../windowDrag';
  import { ctxMenu, type MenuItem } from '../contextmenu.svelte';

  interface Props {
    title: string;
    /** Optional Icon name drawn dim before the title. */
    icon?: string;
    /** One short line under the title (truncated with an ellipsis). */
    subtitle?: string;
    /** Status pill / count next to the title. */
    badge?: Snippet;
    /** Content before the title (e.g. a back button in a detail view). */
    leading?: Snippet;
    /** Breadcrumb trail drawn before the title: `Kubernetes / Monitor / <title>`. */
    crumbs?: { label: string; onclick: () => void }[];
    /** Replaces the plain title text (e.g. a cluster switcher button). The h1
     *  still wraps it; `title` stays the accessible/tooltip name. */
    titleContent?: Snippet;
    /** Buttons/controls, right-aligned; overflow into "⋯". */
    actions?: Snippet;
    /** Segmented control / tab row. */
    tabs?: Snippet;
    tabsPlacement?: 'inline' | 'below';
    /** Extra class on the root (page-specific tweaks). */
    class?: string;
  }

  let {
    title,
    icon,
    subtitle,
    badge,
    leading,
    crumbs = [],
    titleContent,
    actions,
    tabs,
    tabsPlacement = 'inline',
    class: klass = '',
  }: Props = $props();

  let rootEl: HTMLElement | undefined = $state();
  let wrapEl: HTMLDivElement | undefined = $state();
  let actionsEl: HTMLDivElement | undefined = $state();
  /** The controls currently collapsed into the "⋯" menu (DOM order). */
  let collapsed: HTMLElement[] = $state([]);

  const padTraffic = $derived(isTauri && viewport.isDesktop && !ui.railExpanded);

  const FIELD = 'select, input, textarea';
  const GAP = 6; // keep in sync with .ph-actions gap
  const MORE_W = 28; // "⋯" button width (lives inside the wrap, after the actions)

  function canCollapse(el: HTMLElement): boolean {
    if (el.hasAttribute('data-keep')) return false;
    if (el.classList.contains('primary')) return false;
    if (el.matches(FIELD) || el.querySelector(FIELD)) return false;
    // Nothing to click → nothing a menu row could do.
    return el.matches('button, a, [role="button"]') || !!el.querySelector('button, a, [role="button"]');
  }

  let measuring = false;
  function measure(): void {
    if (!actionsEl || !wrapEl || measuring) return;
    measuring = true;
    try {
      const kids = Array.from(actionsEl.children) as HTMLElement[];
      for (const k of kids) k.removeAttribute('data-ph-hidden');
      const visible = kids.filter((k) => k.offsetWidth > 0 || k.getClientRects().length > 0);
      const widthOf = new Map(visible.map((k) => [k, k.getBoundingClientRect().width]));
      let need = visible.reduce((s, k) => s + (widthOf.get(k) ?? 0), 0) + GAP * Math.max(0, visible.length - 1);
      // clientWidth includes the wrap's 2px focus-ring padding on each side.
      let avail = wrapEl.clientWidth - 4;
      if (need <= avail + 0.5) {
        if (collapsed.length) collapsed = [];
        return;
      }
      avail -= MORE_W + GAP;
      const order = visible
        .map((el, i) => ({ el, i, p: Number(el.dataset.overflow ?? 0) || 0 }))
        .filter((o) => canCollapse(o.el))
        .sort((a, b) => a.p - b.p || b.i - a.i);
      const hidden: HTMLElement[] = [];
      for (const o of order) {
        if (need <= avail + 0.5) break;
        o.el.setAttribute('data-ph-hidden', '');
        need -= (widthOf.get(o.el) ?? 0) + GAP;
        hidden.push(o.el);
      }
      collapsed = kids.filter((k) => hidden.includes(k));
    } finally {
      measuring = false;
    }
  }

  let raf = 0;
  function schedule(): void {
    if (raf) return;
    raf = requestAnimationFrame(() => {
      raf = 0;
      measure();
    });
  }

  onMount(() => {
    measure();
    const ro = new ResizeObserver(schedule);
    if (rootEl) ro.observe(rootEl);
    if (wrapEl) ro.observe(wrapEl);
    // Buttons appear/disappear/relabel with page state — re-fit on any change
    // inside the actions (our own data-ph-hidden writes are attributes, which
    // this observer ignores, so it can't loop).
    const mo = new MutationObserver(schedule);
    if (actionsEl) mo.observe(actionsEl, { childList: true, subtree: true, characterData: true });
    return () => {
      ro.disconnect();
      mo.disconnect();
      if (raf) cancelAnimationFrame(raf);
    };
  });

  function labelOf(el: HTMLElement): string {
    return (
      el.dataset.label ||
      el.getAttribute('aria-label') ||
      el.getAttribute('title') ||
      (el.textContent ?? '').replace(/\s+/g, ' ').trim() ||
      'Action'
    );
  }

  function rowFor(el: HTMLElement): MenuItem {
    const disabled = (el as HTMLButtonElement).disabled === true || el.getAttribute('aria-disabled') === 'true';
    return {
      label: labelOf(el),
      icon: el.dataset.icon,
      danger: el.classList.contains('danger'),
      disabled,
      action: () => el.click(),
    };
  }

  function openMore(e: MouseEvent): void {
    const items: MenuItem[] = [];
    for (const el of collapsed) {
      if (el.matches('button, a, [role="button"]')) {
        items.push(rowFor(el));
      } else {
        const inner = Array.from(el.querySelectorAll<HTMLElement>('button, a, [role="button"]'));
        if (items.length && inner.length) items.push({ separator: true });
        for (const b of inner) items.push(rowFor(b));
      }
    }
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    // Anchor under the button, right-aligned-ish; ContextMenu clamps into the
    // viewport after it measures itself.
    ctxMenu.show(
      { preventDefault() {}, stopPropagation() {}, clientX: r.right - 180, clientY: r.bottom + 4 } as unknown as MouseEvent,
      items,
    );
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<header
  bind:this={rootEl}
  class="page-header-bar {klass}"
  class:tauri-pad={padTraffic}
  class:has-below={!!tabs && tabsPlacement === 'below'}
  data-tauri-drag-region
  onmousedown={startWindowDrag}
  data-testid="page-header"
>
  <div class="ph-row">
    {#if leading}<div class="ph-leading">{@render leading()}</div>{/if}
    <div class="ph-title-block">
      {#if icon}<span class="ph-icon"><Icon name={icon} size={16} /></span>{/if}
      <div class="ph-titles">
        <div class="ph-title-line">
          <h1 class="ph-title" title={title}>
            {#each crumbs as c (c.label)}
              <button class="ph-crumb" onclick={c.onclick}>{c.label}</button><span class="ph-sep" aria-hidden="true">/</span>
            {/each}
            {#if titleContent}{@render titleContent()}{:else}{title}{/if}
          </h1>
          {#if badge}<span class="ph-badge">{@render badge()}</span>{/if}
        </div>
        {#if subtitle}<div class="ph-sub" title={subtitle}>{subtitle}</div>{/if}
      </div>
    </div>
    {#if tabs && tabsPlacement === 'inline'}
      <div class="ph-tabs-inline">{@render tabs()}</div>
    {/if}
    <div class="ph-actions-wrap" bind:this={wrapEl}>
      <div class="ph-actions" bind:this={actionsEl}>
        {#if actions}{@render actions()}{/if}
      </div>
      {#if collapsed.length}
        <button
          class="icon-btn ph-more"
          onclick={openMore}
          title="More actions"
          aria-label="More actions"
          aria-haspopup="menu"
        >
          <!-- Icon.svelte has no "more" glyph; the ellipsis reads as one. -->
          <span class="ph-more-glyph" aria-hidden="true">⋯</span>
        </button>
      {/if}
    </div>
  </div>
  {#if tabs && tabsPlacement === 'below'}
    <div class="ph-tabs-below">{@render tabs()}</div>
  {/if}
</header>

<style>
  .page-header-bar {
    /* One chrome height app-wide. */
    --ph-h: 46px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    border-bottom: 1px solid var(--border);
    user-select: none;
    -webkit-user-select: none;
    min-width: 0;
  }
  .ph-row {
    height: var(--ph-h);
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 16px 0 20px;
    min-width: 0;
  }
  .tauri-pad .ph-row {
    padding-inline-start: 84px;
  }
  .ph-leading {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }
  .ph-title-block {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 0 1 auto;
    min-width: 80px;
    max-width: 50%;
  }
  /* A snippet whose content is conditional can render nothing; the empty slot
     must not eat a flex gap. */
  .ph-leading:empty,
  .ph-badge:empty {
    display: none;
  }
  .ph-icon {
    display: inline-flex;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .ph-titles {
    display: flex;
    flex-direction: column;
    justify-content: center;
    min-width: 0;
  }
  .ph-title-line {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .ph-title {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    line-height: 20px;
    letter-spacing: -0.01em;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .ph-crumb {
    border: none;
    background: none;
    padding: 0;
    font: inherit;
    font-weight: 500;
    color: var(--text-dim);
    cursor: pointer;
  }
  .ph-crumb:hover {
    color: var(--accent);
  }
  .ph-sep {
    color: var(--text-dim);
    opacity: 0.6;
    margin: 0 6px;
    font-weight: 400;
  }
  .ph-badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
    font-size: 11.5px;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .ph-sub {
    font-size: 11.5px;
    line-height: 15px;
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ph-tabs-inline {
    display: flex;
    align-items: center;
    flex: 0 1 auto;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .ph-tabs-inline::-webkit-scrollbar {
    display: none;
  }
  .ph-actions-wrap {
    flex: 1 1 0;
    min-width: 0;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 6px;
    overflow: hidden;
    /* Room for focus rings on the edge buttons. */
    padding: 3px 2px;
    margin: -3px -2px;
  }
  .ph-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: nowrap;
    flex-shrink: 0;
    white-space: nowrap;
  }
  .ph-actions > :global([data-ph-hidden]) {
    display: none !important;
  }
  .ph-more {
    flex-shrink: 0;
    width: 28px;
    height: 28px;
  }
  .ph-more-glyph {
    font-size: 16px;
    line-height: 1;
    font-weight: 700;
    letter-spacing: 0.02em;
  }
  .ph-tabs-below {
    display: flex;
    align-items: center;
    padding: 0 16px 0 20px;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .ph-tabs-below::-webkit-scrollbar {
    display: none;
  }
  @media (max-width: 640px) {
    .ph-row {
      padding: 0 10px 0 14px;
      gap: 8px;
    }
    .ph-title-block {
      max-width: 60%;
    }
    .ph-sub {
      display: none;
    }
    .ph-tabs-below {
      padding: 0 10px;
    }
  }
</style>
