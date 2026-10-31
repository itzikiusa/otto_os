///<reference types="svelte" />
;// Phone bottom navigation bar. Shows the first few modules the current user
// can view as large tap targets, plus a "More" affordance that opens the
// shared shell Drawer with the rest (and Settings). Tapping a module routes
// via router.go(); it draws from the same shared sidebar registry as the Rail and
// Navigator and honours the user's saved order + hidden set, so all three
// navigations stay in lockstep.
//
// Rendered only on phone (App.svelte gates it behind viewport.isPhone), so it
// adds nothing to the desktop layout.

import Icon from '../lib/components/Icon.svelte';
import Drawer from './Drawer.svelte';
import { router } from '../lib/router.svelte';
import { navPending } from '../lib/navPending.svelte';
import { ui } from '../lib/stores/ui.svelte';
import { navBadge } from '../lib/navBadge';
import { auth } from '../lib/stores/auth.svelte';
import { plugins } from '../lib/stores/plugins.svelte';
import {
    availableModules,
    activeNavId,
    resolveOrder,
    sidebarSections,
    visibleOrder,
  } from '../lib/sidebar';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // How many primary tabs sit on the bar before everything spills into "More".
  const PRIMARY_COUNT = 4;

  const pluginEntries = $derived(
    plugins.list
      .filter((p) => auth.canPlugin(p.slug, 'view'))
      .map((p) => ({ id: `plugin/${p.slug}`, icon: p.icon, label: p.name })),
  );
  // Flattened section by section, so the bar's order matches the sidebar's —
  // Favorites first, so a user's favorites become the phone's primary tabs.
  const modules = $derived(
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
    ).flatMap((s) => s.modules),
  );
  // The entry the current route highlights (plugin slug, '' → Agents, …).
  const current = $derived(activeNavId(router.parts));
  const primary = $derived(modules.slice(0, PRIMARY_COUNT));
  const overflow = $derived(modules.slice(PRIMARY_COUNT));

  // Badges on spilled modules (a session waiting on you must not hide in More).
  const moreBadge = $derived.by(() => {
    let count = 0;
    let needs = false;
    for (const m of overflow) {
      const b = navBadge(m.id);
      if (!b) continue;
      count += b.count;
      needs ||= b.tone === 'needs';
    }
    return count > 0 ? { count, needs } : null;
  });

  // The overflow opens in the shell Drawer, which owns the modal plumbing
  // (pushModal, focus trap, Esc, backdrop, ✕).
  let moreOpen = $state(false);

  function go(id: string): void {
    router.openModule(id);
    moreOpen = false;
  }

  // "More" is active when the current module lives in the overflow set (or
  // Settings), so the bar reflects where you are even for spilled modules.
  const moreActive = $derived(
    router.module === 'settings' || overflow.some((m) => m.id === current),
  );
;
async () => {

 { svelteHTML.createElement("nav", {   "class":`bottomnav`,"aria-label":`Primary`,});
     for(let m of __sveltets_2_ensureArray(primary)){m.id;
     { svelteHTML.createElement("button", {            "class":`bn-btn`,"aria-current":current === m.id ? 'page' : undefined,...__sveltets_2_empty({"data-nav-id":m.id}),"aria-busy":navPending.id === m.id || undefined,"aria-label":`${m.label}${navBadge(m.id)?.spoken ?? ''}`,"onclick":() => go(m.id),});current === m.id;
       { svelteHTML.createElement("span", { "class":`bn-icon`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":m.icon,"size":20,}});}
        if(navBadge(m.id)){
          const b = navBadge(m.id)!;
           { svelteHTML.createElement("span", {    "class":`bn-badge`,"title":b.title,});b.tone === 'needs';b.count; }
        }
       }
       { svelteHTML.createElement("span", { "class":`bn-label`,});m.label; }
     }
  }

  
   { svelteHTML.createElement("button", {            "class":`bn-btn`,"aria-haspopup":`dialog`,"aria-expanded":moreOpen,"aria-label":moreBadge ? `More, ${moreBadge.count} ${moreBadge.needs ? 'need you' : 'active'}` : 'More',"title":`More`,"onclick":() => (moreOpen = true),});moreActive;
     { svelteHTML.createElement("span", { "class":`bn-icon`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":20,}});}
      if(moreBadge){ { svelteHTML.createElement("span", {  "class":`bn-badge`,});moreBadge.needs;moreBadge.count; }}
     }
     { svelteHTML.createElement("span", { "class":`bn-label`,});  }
   }
 }

 { const $$_rewarD0C = __sveltets_2_ensureComponent(Drawer); const $$_rewarD0 = new $$_rewarD0C({ target: __sveltets_2_any(), props: {        children:() => { return __sveltets_2_any(0); },open:moreOpen,"side":`right`,"title":`More`,"width":`min(86vw, 360px)`,}});/*Ωignore_startΩ*/() => moreOpen = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rewarD0.$$bindings = 'open';
     { svelteHTML.createElement("div", { "class":`sheet-grid`,});
       { svelteHTML.createElement("button", {     "class":`sheet-item`,"onclick":() => {
          moreOpen = false;
          ui.openPalette('commands');
        },});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`command`,"size":20,}});}
         { svelteHTML.createElement("span", {});  }
       }
         for(let m of __sveltets_2_ensureArray(overflow)){m.id;
        const b = navBadge(m.id);
         { svelteHTML.createElement("button", {            "class":`sheet-item`,"aria-current":current === m.id ? 'page' : undefined,...__sveltets_2_empty({"data-nav-id":m.id}),"aria-busy":navPending.id === m.id || undefined,"aria-label":`${m.label}${b?.spoken ?? ''}`,"onclick":() => go(m.id),});current === m.id;
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":m.icon,"size":20,}});}
           { svelteHTML.createElement("span", {});m.label; }
          if(b){ { svelteHTML.createElement("span", {    "class":`bn-badge sheet-badge`,"aria-hidden":`true`,});b.tone === 'needs';b.count; }}
         }
      }
       { svelteHTML.createElement("button", {        "class":`sheet-item`,"aria-current":router.module === 'walkthroughs' ? 'page' : undefined,"onclick":() => {
          router.go('walkthroughs');
          moreOpen = false;
        },});router.module === 'walkthroughs';
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":20,}});}
         { svelteHTML.createElement("span", {});  }
       }
       { svelteHTML.createElement("button", {        "class":`sheet-item`,"aria-current":router.module === 'settings' ? 'page' : undefined,"onclick":() => {
          router.go('settings/appearance');
          moreOpen = false;
        },});router.module === 'settings';
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`gear`,"size":20,}});}
         { svelteHTML.createElement("span", {});  }
       }
     }
 Drawer}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const BottomNav__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type BottomNav__SvelteComponent_ = ReturnType<typeof BottomNav__SvelteComponent_>;
/*Ωignore_endΩ*/export default BottomNav__SvelteComponent_;