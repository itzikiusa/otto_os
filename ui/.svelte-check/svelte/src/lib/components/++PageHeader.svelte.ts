///<reference types="svelte" />
;// Shared page chrome: the ONE header row every top-level module page renders
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
import { onMount, tick } from 'svelte';
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
import { FitGate, StyleWriter, WidthCache, nodeSig } from './headerFit';
;

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
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  


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
  let moreEl: HTMLButtonElement | undefined = $state();
  // Remember the control whose focus moved into overflow, so expanding the
  // toolbar returns the keyboard to the same action rather than the page body.
  let overflowFocus: HTMLElement | null = null;
  /** The controls currently collapsed into the "⋯" menu (DOM order). */
  let collapsed: HTMLElement[] = $state([]);

  // Inside the side-by-side pane the page's top row also carries the pane's
  // controls (Swap / Open in main pane / Close — lib/stores/embedChrome).
  $effect(() => (rootEl ? embedChrome.claim(rootEl) : undefined));
  const hostsPrimaryPane = $derived(!isEmbedded && isTauri && sidePane.active && !!rootEl && embedChrome.owner === rootEl);
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
      !isEmbedded &&
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

  // SF-12: skip a pass whose inputs are unchanged, keep collapsed actions'
  // widths instead of un-hiding them to re-measure, and drop style writes
  // that store the same value (lib/components/headerFit.ts).
  const gate = new FitGate();
  const widths = new WidthCache<HTMLElement>();
  const styles = new StyleWriter();

  let measuring = false;
  function measure(): void {
    if (!actionsEl || !wrapEl || measuring) return;
    const kids = Array.from(actionsEl.children) as HTMLElement[];
    const sigs = kids.map(nodeSig);
    const row = wrapEl.parentElement;
    const lineText = titleEl?.parentElement?.textContent ?? '';
    if (!gate.shouldRun(gate.signature([rootEl, wrapEl, row ?? undefined], `${lineText}\u0003${sigs.join('\u0003')}`))) return;
    measuring = true;
    try {
      // Only an action we collapsed whose width we don't know (new, or its
      // content changed) is un-hidden to be measured; the rest are read as
      // they are (visible) or from the cache (collapsed, unchanged).
      const known = new Map<HTMLElement, number>();
      kids.forEach((k, i) => {
        if (!k.hasAttribute('data-ph-hidden')) return;
        const w = widths.get(k, sigs[i]);
        if (w === undefined) k.removeAttribute('data-ph-hidden');
        else known.set(k, w);
      });
      const widthOf = new Map<HTMLElement, number>();
      const visible: HTMLElement[] = [];
      kids.forEach((k, i) => {
        const cached = known.get(k);
        if (cached !== undefined) {
          if (cached > 0) {
            visible.push(k);
            widthOf.set(k, cached);
          }
          return;
        }
        if (k.offsetWidth > 0 || k.getClientRects().length > 0) {
          const w = k.getBoundingClientRect().width;
          visible.push(k);
          widthOf.set(k, w);
          widths.set(k, sigs[i], w);
        }
      });
      // What can never collapse (selects, primaries, data-keep) must always
      // fit: reserve it (+ the ⋯ button) as the wrap's min width, so the
      // title block and inline tabs yield — the subtitle ellipsizes — rather
      // than the kept controls being clipped off the start of the row.
      const keep = visible.filter((k) => !canCollapse(k));
      let keepW = keep.reduce((s, k) => s + (widthOf.get(k) ?? 0), 0) + GAP * Math.max(0, keep.length - 1);
      if (keep.length < visible.length) keepW += MORE_W + (keep.length ? GAP : 0);
      // …but the title block never gives up its title line (h1 + badges).
      // Every layout READ happens before the first style WRITE below.
      let titleMin = 0;
      if (rootEl && titleEl) {
        const iconW = iconEl ? iconEl.offsetWidth + 8 : 0;
        const badgeW = badgeEl ? badgeEl.offsetWidth + 8 : 0;
        titleMin = Math.ceil(titleEl.scrollWidth + iconW + badgeW);
      }
      // When even the kept controls can't fit next to the title (a phone), the
      // reservation is capped and the actions scroll horizontally instead —
      // every control stays reachable, none is clipped away.
      const lead = row?.querySelector<HTMLElement>(':scope > .ph-leading');
      const inlineTabs = row?.querySelector<HTMLElement>(':scope > .ph-tabs-inline');
      const pane = row?.querySelector<HTMLElement>(':scope > .pane-controls');
      const rowW = row ? row.clientWidth : 0;
      const leadW = lead ? lead.offsetWidth + 12 : 0;
      const tabsW = inlineTabs ? inlineTabs.offsetWidth + 12 : 0;
      const paneW = pane ? pane.offsetWidth + 12 : 0;
      const room = row ? rowW - 48 - titleMin - leadW - tabsW - paneW : Infinity;
      if (rootEl && titleEl) styles.set(rootEl, rootEl.style, '--ph-title-min', `${titleMin}px`);
      styles.set(wrapEl, wrapEl.style, 'min-width', `${Math.max(0, Math.min(Math.ceil(keepW) + RING * 2, room))}px`);
      let need = visible.reduce((s, k) => s + (widthOf.get(k) ?? 0), 0) + GAP * Math.max(0, visible.length - 1);
      // The inline tabs' cap: never less than half the row (a header with
      // many actions keeps today's split), but when the actions leave room
      // the tabs take it — a fixed 50% left the Git repo tabs squeezed to
      // "bo_co…" next to an empty half-row (the page's toolbar lives on the
      // row below, so its header has no actions at all).
      if (row && rootEl) {
        const reserved = 48 + titleMin + leadW + paneW + (visible.length ? need + RING * 2 + 12 : 0);
        const tabsCap = Math.max(rowW * 0.5, rowW - reserved);
        styles.set(rootEl, rootEl.style, '--ph-tabs-max', `${Math.floor(tabsCap)}px`);
      }
      // The title block's cap: 45% of the row by default, but a header with
      // only one or two actions lets a long title use the room they leave
      // instead of ellipsizing next to an empty stretch of toolbar.
      if (row && rootEl) {
        const others = 48 + leadW + tabsW + paneW + (visible.length ? need + RING * 2 + 12 : 0);
        const cap = Math.max(rowW * 0.45, rowW - others);
        styles.set(rootEl, rootEl.style, '--ph-title-max', `${Math.floor(cap)}px`);
      }
      // clientWidth includes the wrap's focus-ring padding on each side. (A
      // fresh layout only when a write above actually changed something —
      // the wrap's width is flex-basis 0, independent of what it contains.)
      let avail = wrapEl.clientWidth - RING * 2;
      const hidden: HTMLElement[] = [];
      if (need > avail + 0.5) {
        avail -= MORE_W + GAP;
        const order = visible
          .map((el, i) => ({ el, i, p: Number(el.dataset.overflow ?? 0) || 0 }))
          .filter((o) => canCollapse(o.el))
          .sort((a, b) => a.p - b.p || b.i - a.i);
        for (const o of order) {
          if (need <= avail + 0.5) break;
          need -= (widthOf.get(o.el) ?? 0) + GAP;
          hidden.push(o.el);
        }
      }
      const focused = document.activeElement;
      const hidingFocus = focused instanceof HTMLElement && hidden.some((el) => el.contains(focused));
      const removingFocusedMore = focused === moreEl && hidden.length === 0;
      const expandedFocus = removingFocusedMore ? overflowFocus ?? collapsed
        .flatMap((el) => el.matches('button, a, [role="button"]') ? [el] : Array.from(el.querySelectorAll<HTMLElement>('button, a, [role="button"]')))
        .find((el) => !el.matches(':disabled') && el.getAttribute('aria-disabled') !== 'true') : null;
      if (hidingFocus) overflowFocus = focused as HTMLElement;
      // Reconcile the collapsed set: touch only the attributes that change.
      for (const k of kids) {
        const hide = hidden.includes(k);
        if (hide !== k.hasAttribute('data-ph-hidden')) k.toggleAttribute('data-ph-hidden', hide);
      }
      const next = kids.filter((k) => hidden.includes(k));
      if (next.length !== collapsed.length || next.some((k, i) => k !== collapsed[i])) collapsed = next;
      if (hidingFocus || removingFocusedMore) {
        void tick().then(() => {
          // A user or another surface may have moved focus during this render.
          if (document.activeElement !== focused && document.activeElement !== document.body) return;
          const target = hidingFocus ? moreEl : expandedFocus;
          if (target?.isConnected && target.getClientRects().length && !target.matches(':disabled')) {
            target.focus({ preventScroll: true });
          }
          if (removingFocusedMore) overflowFocus = null;
        });
      }
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
    const ro = new ResizeObserver((entries) => {
      gate.observe(entries);
      schedule();
    });
    if (rootEl) ro.observe(rootEl);
    if (wrapEl) ro.observe(wrapEl);
    const row = wrapEl?.parentElement;
    if (row) ro.observe(row);
    // Buttons appear/disappear/relabel with page state — re-fit on any change
    // inside the actions (our own data-ph-hidden writes are attributes, which
    // this observer ignores, so it can't loop). A mutation that leaves every
    // action's signature as it was is dropped by the gate in measure().
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
;
async () => {


  { const $$_header0 = svelteHTML.createElement("header", {          "class":`page-header-bar chrome-material ${klass}`,...__sveltets_2_empty({"data-tauri-drag-region":true}),"onmousedown":startWindowDrag,...__sveltets_2_empty({"data-testid":`page-header`}),});rootEl = $$_header0;padTraffic;tabsBelow;
   { svelteHTML.createElement("div", { "class":`ph-row`,});
    if(leading){ { svelteHTML.createElement("div", { "class":`ph-leading`,});;__sveltets_2_ensureSnippet(leading()); }}
     { svelteHTML.createElement("div", { "class":`ph-title-block`,});
      if(icon){ { const $$_span3 = svelteHTML.createElement("span", {  "class":`ph-icon`,});iconEl = $$_span3; { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":icon,"size":16,}});} }}
       { svelteHTML.createElement("div", { "class":`ph-titles`,});
         { svelteHTML.createElement("div", { "class":`ph-title-line`,});
           { const $$_h15 = svelteHTML.createElement("h1", {    "class":`ph-title`,"title":title,});titleEl = $$_h15;
               for(let c of __sveltets_2_ensureArray(crumbs)){c.label;
               { svelteHTML.createElement("button", {   "class":`ph-crumb`,"onclick":c.onclick,});c.label; } { svelteHTML.createElement("span", {   "class":`ph-sep`,"aria-hidden":`true`,});  }
            }
            if(titleContent){;__sveltets_2_ensureSnippet(titleContent());}else{title;}
           }
          if(badge){ { const $$_span5 = svelteHTML.createElement("span", {  "class":`ph-badge`,});badgeEl = $$_span5;;__sveltets_2_ensureSnippet(badge()); }}
         }
        if(subtitle){ { svelteHTML.createElement("div", {   "class":`ph-sub`,"title":subtitle,});subtitle; }}
       }
     }
    if(tabs && !tabsBelow){
       { svelteHTML.createElement("div", { "class":`ph-tabs-inline`,});;__sveltets_2_ensureSnippet(tabs()); }
    }
     { const $$_div2 = svelteHTML.createElement("div", {  "class":`ph-actions-wrap`,});wrapEl = $$_div2;
       { const $$_div3 = svelteHTML.createElement("div", {  "class":`ph-actions`,});actionsEl = $$_div3;
        if(actions){;__sveltets_2_ensureSnippet(actions());}
       }
      if(collapsed.length){
         { const $$_button3 = svelteHTML.createElement("button", {            "class":`icon-btn ph-more`,"onclick":openMore,"title":`More actions`,"aria-label":`More actions`,"aria-haspopup":`menu`,});moreEl = $$_button3;
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":16,}});}
         }
      }
     }
    if(hostsPane){ { const $$_slortnoCenaP2C = __sveltets_2_ensureComponent(PaneControls); new $$_slortnoCenaP2C({ target: __sveltets_2_any(), props: {}});}} else if (hostsPrimaryPane){ { const $$_slortnoCenaP2C = __sveltets_2_ensureComponent(PaneControls); new $$_slortnoCenaP2C({ target: __sveltets_2_any(), props: {  "pane":`primary`,}});}}
   }
  if(tabs && tabsBelow){
     { svelteHTML.createElement("div", { "class":`ph-tabs-below`,});;__sveltets_2_ensureSnippet(tabs()); }
  }
  if(hostsBar){ { const $$_raBgnivirDtnegA1C = __sveltets_2_ensureComponent(AgentDrivingBar); new $$_raBgnivirDtnegA1C({ target: __sveltets_2_any(), props: {}});}}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PageHeader__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PageHeader__SvelteComponent_ = ReturnType<typeof PageHeader__SvelteComponent_>;
/*Ωignore_endΩ*/export default PageHeader__SvelteComponent_;