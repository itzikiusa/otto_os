///<reference types="svelte" />
;
import { rowMenu } from '../lib/rowMenu';
// Collapsed 44px icon rail (⌘1 expands to the Navigator).
import { navBadge } from '../lib/navBadge';
import Icon from '../lib/components/Icon.svelte';
import NotificationBell from './NotificationBell.svelte';
import { router } from '../lib/router.svelte';
import { navPending } from '../lib/navPending.svelte';
import { ui } from '../lib/stores/ui.svelte';
import { auth } from '../lib/stores/auth.svelte';
import { plugins } from '../lib/stores/plugins.svelte';
import { ctxMenu, type MenuItem } from '../lib/contextmenu.svelte';
import { sidePane, splitMenuItems, navClick, SPLIT_HINT } from '../lib/stores/sidePane.svelte';
import { tick } from 'svelte';
import {
    activeNavId,
    availableModules,
    resolveOrder,
    sidebarSections,
    visibleOrder,
  } from '../lib/sidebar';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // The collapsed rail mirrors the same resolved module list as the Navigator
  // (shared registry → RBAC filter + plugins → user's saved order, minus hidden
  // ones), section by section with a thin separator between sections (no
  // headers or folding — the Navigator owns those): Favorites first, then the
  // rest in the user's section order. It's read-only here: reordering /
  // show-hide / favoriting happens in the expanded Navigator (and Settings →
  // Appearance). See ui.svelte.ts for persistence.
  const pluginEntries = $derived(
    plugins.list
      .filter((p) => auth.canPlugin(p.slug, 'view'))
      .map((p) => ({ id: `plugin/${p.slug}`, icon: p.icon, label: p.name })),
  );
  const sections = $derived(
    sidebarSections(
      visibleOrder(
        resolveOrder(
          availableModules((f) => auth.can(f, 'view'), pluginEntries),
          ui.sidebarOrder,
        ),
        ui.sidebarHidden,
      ),
      ui.sidebarFavorites,
      ui.sidebarGroupOrder,
    ),
  );

  // The module column scrolls when the window is short; keep the active icon
  // in view on every route change (same rule as the Navigator).
  let listEl = $state<HTMLDivElement>();
  $effect(() => {
    void router.parts.join('/');
    const el = listEl;
    if (!el) return;
    void tick().then(() => el.querySelector<HTMLElement>('.rail-btn.active')?.scrollIntoView({ block: 'nearest' }));
  });

  // Active when the route highlights the entry (plugin slug, default route →
  // Agents, database/brokers → Connections: see activeNavId).
  const activeId = $derived(activeNavId(router.parts));
  function isActive(id: string): boolean {
    return activeId === id;
  }

  // Same row menu as the Navigator (minus in-section moves): the side-by-side
  // entries, Favorites, and a way into Customize — which lives in the
  // expanded Navigator, so it expands the sidebar first.
  function moduleMenu(e: MouseEvent, id: string, label: string): void {
    const fav = ui.sidebarFavorites.includes(id);
    const split = splitMenuItems(id, label);
    const items: MenuItem[] = [
      // Plain clicks resume the module where it was left; this is the way home.
      { label: `Open ${label} main page`, icon: 'home', action: () => router.go(id) },
      { separator: true },
      ...split,
      ...(split.length ? [{ separator: true }] : []),
      fav
        ? { label: 'Remove from Favorites', icon: 'star', action: () => ui.removeSidebarFavorite(id) }
        : { label: 'Add to Favorites', icon: 'star', action: () => ui.addSidebarFavorite(id) },
      { separator: true },
      {
        label: 'Customize sidebar',
        icon: 'edit',
        action: () => {
          if (!ui.railExpanded) ui.toggleRail();
          ui.sidebarEditMode = true;
        },
      },
    ];
    ctxMenu.show(e, items);
  }

  // The account avatar was a button that did nothing — and the collapsed
  // rail had no way to sign out (only the expanded Navigator's footer did).
  function accountMenu(e: MouseEvent): void {
    const name = auth.me?.display_name ?? 'Account';
    const sub = auth.isRoot ? 'root' : auth.me?.username;
    ctxMenu.showAt(e.currentTarget as HTMLElement, [
      { label: sub && sub !== name ? `${name} (${sub})` : name, icon: 'user', disabled: true },
      { separator: true },
      // Help and Settings are the rail buttons right above this one.
      { label: 'Sign out', icon: 'logout', action: () => auth.logout() },
    ]);
  }
;
async () => {

 { svelteHTML.createElement("nav", {   "class":`rail sidebar-material`,"aria-label":`Modules`,});
   { svelteHTML.createElement("button", {           "class":`rail-btn`,"onclick":() => ui.toggleRail(),"title":`Expand sidebar`,"aria-label":`Expand sidebar`,"aria-keyshortcuts":`Meta+1`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {  "name":`sidebar`,}});}
   }
  
   { svelteHTML.createElement("button", {               "class":`rail-btn`,"onclick":() => router.back(),"disabled":!router.canBack,"title":`Go back`,"aria-label":`Go back`,"aria-keyshortcuts":`Meta+Shift+ArrowLeft`,...__sveltets_2_empty({"data-testid":`rail-back`}),});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":14,}});}
   }
   { svelteHTML.createElement("button", {               "class":`rail-btn`,"onclick":() => router.forward(),"disabled":!router.canForward,"title":`Go forward`,"aria-label":`Go forward`,"aria-keyshortcuts":`Meta+Shift+ArrowRight`,...__sveltets_2_empty({"data-testid":`rail-forward`}),});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":14,}});}
   }
   { const $$_lleBnoitacifitoN1C = __sveltets_2_ensureComponent(NotificationBell); new $$_lleBnoitacifitoN1C({ target: __sveltets_2_any(), props: {}});}

   { const $$_div1 = svelteHTML.createElement("div", {  "class":`rail-modules`,});listEl = $$_div1;
        for(let sec of __sveltets_2_ensureArray(sections)){let si = 1;sec.group.id;
      if(si > 0){
         { svelteHTML.createElement("div", {       "class":`rail-sep`,"role":`separator`,"aria-label":sec.group.label,...__sveltets_2_empty({"data-testid":`rail-sep`}),}); }
      }
         for(let m of __sveltets_2_ensureArray(sec.modules)){m.id;
        const inSide = sidePane.active && sidePane.key === m.id;
         {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {                      "class":`rail-btn`,"aria-current":isActive(m.id) ? 'page' : undefined,"onclick":(e) => navClick(e, m.id, m.label),"oncontextmenu":(e) => moduleMenu(e, m.id, m.label),"title":`${m.label} · ${sec.group.label}${inSide ? ' · in the side pane' : sidePane.supported ? ` — ${SPLIT_HINT}` : ''}`,"aria-label":`${m.label}${inSide ? ' (in the side pane)' : ''}${navBadge(m.id)?.spoken ?? ''}`,...__sveltets_2_empty({"data-testid":`rail-${m.id}`}),...__sveltets_2_empty({"data-nav-id":m.id}),"aria-busy":navPending.id === m.id || undefined,});isActive(m.id);inSide;
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {  "name":m.icon,}});}
          
          if(navBadge(m.id)){
            const b = navBadge(m.id)!;
             { svelteHTML.createElement("span", {    "class":`rail-badge`,"title":b.title,});b.tone === 'needs';b.count; }
          }
         }}
      }
    }
   }

   { svelteHTML.createElement("div", { "class":`rail-bottom`,});
     { svelteHTML.createElement("button", {            "class":`rail-btn`,"aria-current":router.module === 'walkthroughs' ? 'page' : undefined,"onclick":() => router.go('walkthroughs'),"title":`Help`,"aria-label":`Help`,});router.module === 'walkthroughs';
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {  "name":`info`,}});}
     }
     { svelteHTML.createElement("button", {              "class":`rail-btn`,"aria-current":router.module === 'settings' ? 'page' : undefined,"onclick":() => router.go('settings/appearance'),"title":`Settings`,"aria-label":`Settings`,"aria-keyshortcuts":`Meta+,`,});router.module === 'settings';
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {  "name":`gear`,}});}
     }
     { svelteHTML.createElement("button", {           "class":`rail-btn user`,"onclick":accountMenu,"title":auth.me?.display_name ?? 'Account',"aria-label":`Account`,"aria-haspopup":`menu`,});
       { svelteHTML.createElement("span", { "class":`avatar`,});(auth.me?.display_name ?? '?').slice(0, 1).toUpperCase(); }
     }
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Rail__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Rail__SvelteComponent_ = ReturnType<typeof Rail__SvelteComponent_>;
/*Ωignore_endΩ*/export default Rail__SvelteComponent_;