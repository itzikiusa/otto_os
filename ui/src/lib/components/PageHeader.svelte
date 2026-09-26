<script lang="ts">
  // Shared page chrome: the ONE header row every top-level module page renders
  // (the Agents page keeps its session TabBar instead). Modelled on the macOS
  // unified toolbar — title + actions in a single fixed-height bar:
  //
  //   [icon] Title  [badge]  subtitle…  [tabs (inline)]  [secondary…] [primary] [⋯]
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
  import Icon, { type IconName } from './Icon.svelte';
  import { ui, isTauri } from '../stores/ui.svelte';
  import { viewport } from '../stores/viewport.svelte';
  import { startWindowDrag } from '../windowDrag';
  import { isPopout, isEmbedded } from '../desktop';
  import { sidePane } from '../stores/sidePane.svelte';
  import { embedChrome } from '../stores/embedChrome.svelte';
  import PaneControls from './PaneControls.svelte';
  import AgentDrivingBar from './AgentDrivingBar.svelte';
  import { uiControl } from '../stores/uiControl.svelte';
  import { ctxMenu, type MenuItem } from '../contextmenu.svelte';

  interface Props {
    title: string;
    /** Optional Icon name drawn dim before the title. */
    icon?: IconName;
    /** One short dim line after the title, on its baseline (ellipsized). */
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
  let titleEl: HTMLElement | undefined = $state();
  let iconEl: HTMLElement | undefined = $state();
  let badgeEl: HTMLElement | undefined = $state();
  /** The controls currently collapsed into the "⋯" menu (DOM order). */
  let collapsed: HTMLElement[] = $state([]);

  // Inside the side-by-side pane the page's top row also carries the pane's
  // controls (Swap / Open in main pane / Close — lib/stores/embedChrome).
  $effect(() => (rootEl ? embedChrome.claim(rootEl) : undefined));
  const hostsPane = $derived(isEmbedded && !!rootEl && embedChrome.owner === rootEl);
  // Agent UI control: the page's top header hosts the "‹agent› is driving…"
  // strip under its row (lib/components/AgentDrivingBar.svelte).
  $effect(() => (rootEl ? uiControl.claimBar(rootEl) : undefined));
  const hostsBar = $derived(!!rootEl && uiControl.barOwner === rootEl);
  // A pop-out window's traffic lights live in its own title strip (shell).
  // In a side-by-side split they sit over whichever pane leads: the main
  // pane's header when the side pane trails, the side pane's when it leads.
  const padTraffic = $derived(
    (isTauri &&
      viewport.isDesktop &&
      !ui.railExpanded &&
      !isPopout &&
      !(sidePane.showing && sidePane.placement === 'leading')) ||
      (hostsPane && embedChrome.padTraffic),
  );
  // A phone row has no room for title + tabs + actions: tabs drop to the
  // second row there regardless of the requested placement.
  const tabsBelow = $derived(!!tabs && (tabsPlacement === 'below' || viewport.isPhone));

  const FIELD = 'select, input, textarea';
  const GAP = 6; // keep in sync with .ph-actions gap
  const MORE_W = 28; // "⋯" button width (lives inside the wrap, after the actions)
  // Focus-ring room each side of a scroller (.ph-actions-wrap padding): the
  // global :focus-visible ring is a 2px outline at a 1px offset, so 3px.
  const RING = 3;

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
      // What can never collapse (selects, primaries, data-keep) must always
      // fit: reserve it (+ the ⋯ button) as the wrap's min width, so the
      // title block and inline tabs yield — the subtitle ellipsizes — rather
      // than the kept controls being clipped off the start of the row.
      const keep = visible.filter((k) => !canCollapse(k));
      let keepW = keep.reduce((s, k) => s + (widthOf.get(k) ?? 0), 0) + GAP * Math.max(0, keep.length - 1);
      if (keep.length < visible.length) keepW += MORE_W + (keep.length ? GAP : 0);
      // …but the title block never gives up its title line (h1 + badges).
      let titleMin = 0;
      if (rootEl && titleEl) {
        const iconW = iconEl ? iconEl.offsetWidth + 8 : 0;
        const badgeW = badgeEl ? badgeEl.offsetWidth + 8 : 0;
        titleMin = Math.ceil(titleEl.scrollWidth + iconW + badgeW);
        rootEl.style.setProperty('--ph-title-min', `${titleMin}px`);
      }
      // When even the kept controls can't fit next to the title (a phone), the
      // reservation is capped and the actions scroll horizontally instead —
      // every control stays reachable, none is clipped away.
      const row = wrapEl.parentElement;
      const lead = row?.querySelector<HTMLElement>(':scope > .ph-leading');
      const inlineTabs = row?.querySelector<HTMLElement>(':scope > .ph-tabs-inline');
      const pane = row?.querySelector<HTMLElement>(':scope > .pane-controls');
      const room = row
        ? row.clientWidth -
          48 -
          titleMin -
          (lead ? lead.offsetWidth + 12 : 0) -
          (inlineTabs ? inlineTabs.offsetWidth + 12 : 0) -
          (pane ? pane.offsetWidth + 12 : 0)
        : Infinity;
      wrapEl.style.minWidth = `${Math.max(0, Math.min(Math.ceil(keepW) + RING * 2, room))}px`;
      let need = visible.reduce((s, k) => s + (widthOf.get(k) ?? 0), 0) + GAP * Math.max(0, visible.length - 1);
      // The inline tabs' cap: never less than half the row (a header with
      // many actions keeps today's split), but when the actions leave room
      // the tabs take it — a fixed 50% left the Git repo tabs squeezed to
      // "bo_co…" next to an empty half-row (the page's toolbar lives on the
      // row below, so its header has no actions at all).
      if (row && rootEl) {
        const reserved =
          48 +
          titleMin +
          (lead ? lead.offsetWidth + 12 : 0) +
          (pane ? pane.offsetWidth + 12 : 0) +
          (visible.length ? need + RING * 2 + 12 : 0);
        const tabsCap = Math.max(row.clientWidth * 0.5, row.clientWidth - reserved);
        rootEl.style.setProperty('--ph-tabs-max', `${Math.floor(tabsCap)}px`);
      }
      // The title block's cap: 45% of the row by default, but a header with
      // only one or two actions lets a long title use the room they leave
      // instead of ellipsizing next to an empty stretch of toolbar.
      if (row && rootEl) {
        const others =
          48 +
          (lead ? lead.offsetWidth + 12 : 0) +
          (inlineTabs ? inlineTabs.offsetWidth + 12 : 0) +
          (pane ? pane.offsetWidth + 12 : 0) +
          (visible.length ? need + RING * 2 + 12 : 0);
        const cap = Math.max(row.clientWidth * 0.45, row.clientWidth - others);
        rootEl.style.setProperty('--ph-title-max', `${Math.floor(cap)}px`);
      }
      // clientWidth includes the wrap's focus-ring padding on each side.
      let avail = wrapEl.clientWidth - RING * 2;
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
    // A retitled page (another item selected) changes the title's minimum.
    const line = titleEl?.parentElement;
    if (line) mo.observe(line, { childList: true, subtree: true, characterData: true });
    return () => {
      ro.disconnect();
      mo.disconnect();
      if (raf) cancelAnimationFrame(raf);
    };
  });

  function labelOf(el: HTMLElement): string {
    // A button's visible text beats its `title`: the title is usually a
    // sentence of explanation ("Tidy layout into rows"), which made a long,
    // clipped menu row where the button itself read "Tidy". Text with no
    // letters (a bare count, a glyph) still defers to the title.
    const text = (el.textContent ?? '').replace(/\s+/g, ' ').trim();
    return (
      el.dataset.label ||
      el.getAttribute('aria-label') ||
      (/\p{L}/u.test(text) ? text : '') ||
      el.getAttribute('title') ||
      text ||
      'Action'
    );
  }

  function rowFor(el: HTMLElement): MenuItem {
    const disabled = (el as HTMLButtonElement).disabled === true || el.getAttribute('aria-disabled') === 'true';
    const label = labelOf(el);
    // The control's tooltip rides along (it usually says WHY a disabled
    // button is disabled — that reason was lost in the ⋯ menu).
    const tip = el.getAttribute('title');
    return {
      label,
      icon: el.dataset.icon,
      danger: el.classList.contains('danger'),
      disabled,
      title: tip && tip !== label ? tip : undefined,
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
    // Directly under the button, end edges aligned (mirrored in RTL);
    // ContextMenu measures the real menu and clamps it into the viewport.
    ctxMenu.showAt(e.currentTarget as HTMLElement, items, { align: 'end' });
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<header
  bind:this={rootEl}
  class="page-header-bar chrome-material {klass}"
  class:tauri-pad={padTraffic}
  class:has-below={tabsBelow}
  data-tauri-drag-region
  onmousedown={startWindowDrag}
  data-testid="page-header"
>
  <div class="ph-row">
    {#if leading}<div class="ph-leading">{@render leading()}</div>{/if}
    <div class="ph-title-block">
      {#if icon}<span class="ph-icon" bind:this={iconEl}><Icon name={icon} size={16} /></span>{/if}
      <div class="ph-titles">
        <div class="ph-title-line">
          <h1 class="ph-title" title={title} bind:this={titleEl}>
            {#each crumbs as c (c.label)}
              <button class="ph-crumb" onclick={c.onclick}>{c.label}</button><span class="ph-sep" aria-hidden="true">/</span>
            {/each}
            {#if titleContent}{@render titleContent()}{:else}{title}{/if}
          </h1>
          {#if badge}<span class="ph-badge" bind:this={badgeEl}>{@render badge()}</span>{/if}
        </div>
        {#if subtitle}<div class="ph-sub" title={subtitle}>{subtitle}</div>{/if}
      </div>
    </div>
    {#if tabs && !tabsBelow}
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
          <Icon name="more" size={16} />
        </button>
      {/if}
    </div>
    {#if hostsPane}<PaneControls />{/if}
  </div>
  {#if tabs && tabsBelow}
    <div class="ph-tabs-below">{@render tabs()}</div>
  {/if}
  {#if hostsBar}<AgentDrivingBar />{/if}
</header>

<style>
  .page-header-bar {
    /* One chrome height app-wide. */
    --ph-h: 46px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    /* Chrome: the toolbar glass over the ambient backdrop (tokens.css
       .chrome-material, applied in the markup), a quiet hairline under it. */
    border-bottom: 1px solid var(--separator);
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
    /* Natural width (no reserved minimum, so short titles don't leave a gap
       before inline tabs), capped at --ph-title-max (measured in JS: 45% of
       the row, or everything the actions leave free when there are few of
       them). When the actions need the room it shrinks — the subtitle
       ellipsizes — but never below its title line (--ph-title-min). */
    flex: 0 1 auto;
    min-width: min(var(--ph-title-min, 0px), var(--ph-title-max, 45%));
    max-width: var(--ph-title-max, 45%);
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
  /* Title and subtitle share ONE baseline (the Design Hall section-header
     look), so the title sits at the same height on every page whether or not
     it has a subtitle — a stacked subtitle pushed the title up against the
     top edge and made headers jump between modules. */
  .ph-titles {
    display: flex;
    align-items: baseline;
    gap: 10px;
    min-width: 0;
  }
  .ph-title-line {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    /* The subtitle yields first; the title only ellipsizes past the block. */
    flex: 0 0 auto;
    max-width: 100%;
  }
  .ph-title {
    margin: 0;
    font-size: var(--fs-l);
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
    color: var(--accent-text);
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
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .ph-sub {
    flex: 1 1 auto;
    min-width: 0;
    font-size: var(--fs-s);
    line-height: 20px;
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ph-tabs-inline {
    display: flex;
    align-items: center;
    /* Tabs keep their width (the subtitle yields first, then the actions
       scroll); only a tab set wider than the room the title and actions
       leave (--ph-tabs-max, measured in JS; never below half the row) is
       capped and scrolls. On a phone the tabs move to the row below. */
    flex: 0 0 auto;
    max-width: var(--ph-tabs-max, 50%);
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
    /* A scroller clips on both axes: leave room for the tabs' focus rings
       (and a badge's overhang) instead of shaving them off. */
    padding: 3px;
    margin: -3px;
  }
  .ph-tabs-inline::-webkit-scrollbar {
    display: none;
  }
  .ph-actions-wrap {
    flex: 1 1 0;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    /* Normally everything fits (lower-priority controls collapse into ⋯);
       if the never-collapse ones still don't, scroll rather than clip. */
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
    /* Room for focus rings on the edge buttons (2px outline + 1px offset =
       3px; keep in sync with RING in the script). */
    padding: 3px;
    margin: -3px;
  }
  .ph-actions-wrap::-webkit-scrollbar {
    display: none;
  }
  .ph-actions {
    /* Right-aligned via auto margin (not justify-content: flex-end), so an
       overflowing row scrolls from its start instead of spilling off it. */
    margin-inline-start: auto;
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: nowrap;
    flex-shrink: 0;
    white-space: nowrap;
  }
  /* One control height in the toolbar row, whether a page passed .btn or
     .btn.small — mixed 22/26px buttons made headers look assembled from
     different kits. */
  .ph-actions :global(.btn) {
    height: 26px;
    font-size: var(--fs-m);
  }
  .ph-actions :global(.btn.small) {
    padding: 0 10px;
  }
  .ph-actions > :global([data-ph-hidden]) {
    display: none !important;
  }
  .ph-more {
    flex-shrink: 0;
    width: 28px;
    height: 28px;
  }
  .ph-tabs-below {
    display: flex;
    align-items: center;
    /* Block padding: focus-ring room (this row scrolls, so it clips). */
    padding: 3px 16px 3px 20px;
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
      padding: 3px 10px;
    }
  }
</style>
