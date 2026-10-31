///<reference types="svelte" />
;
import { rowMenu } from '../../lib/rowMenu';
import { plural } from '../../lib/plural';
// Home: Otto's one "desktop". The ambient backdrop fills the page; on it sit
// a greeting + today's glance cards (HomeToday) and the active space's
// widgets — up to 8 live boxes (Agents, Mission Control, DB dashboards,
// Kubernetes, Insights, Usage) on a 12-column grid. Spaces 01–04 are Home's
// views, shared with the floating bar through lib/stores/spaces.svelte.ts;
// they slide (tabs, ←/→, swipe, ⌘K) and can cycle every 30 s. Any widget
// zooms to fill the page. Layout is per device (see home.svelte.ts).
import { untrack } from 'svelte';
import { onTabKey } from '../../lib/tabKeys';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import Modal from '../../lib/components/Modal.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { registry } from '../../lib/commands.svelte';
import HomeBox from './HomeBox.svelte';
import HomeToday from './HomeToday.svelte';
import { spaces, spaceNumber } from '../../lib/stores/spaces.svelte';
import { home, MAX_BOXES, MAX_VIEWS, ROTATE_MS, ROW_PX, GAP_PX } from './home.svelte';
import { HOME_KINDS, type HomeBoxKind } from '../../lib/home/kinds';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const can = (f: Parameters<typeof auth.can>[0]): boolean => auth.can(f, 'view');
  const kinds = $derived(HOME_KINDS.filter((k) => can(k.feature)));

  // First visit seeds a sensible view — only once the RBAC snapshot is in so
  // the seed reflects what this user can actually see.
  $effect(() => {
    if (auth.me) home.ensureDefault(can);
  });

  // ── Auto-rotation ────────────────────────────────────────────────────────
  // A single timeout per epoch; any manual navigation / toggle bumps the epoch
  // (store) which re-arms the countdown from zero. Paused while zoomed, mid-
  // gesture, or with a sheet open — those all read as "the user is busy here".
  let picking = $state(false);
  let hovering = $state(false);
  let focusInside = $state(false);
  $effect(() => {
    const on = home.rotating && !picking && !hovering && !focusInside;
    void home.rotationEpoch;
    if (!on) return;
    const h = window.setTimeout(() => {
      if (document.visibilityState === 'visible') home.next();
      else home.rotationEpoch += 1; // re-arm; try again next period
    }, ROTATE_MS);
    return () => clearTimeout(h);
  });
  const rotating = $derived(home.rotating && !picking && !hovering && !focusInside);

  // ── Keyboard + swipe ─────────────────────────────────────────────────────
  function onKey(e: KeyboardEvent): void {
    if (e.defaultPrevented || ui.overlayOpen || ui.navDrawerOpen || ctxMenu.open) return;
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable)) return;
    // A composite widget in a box (window radios, tabs) owns its arrow keys —
    // they must never slide the whole Home view away under it.
    if (t?.closest?.('[role="radiogroup"],[role="tablist"]')) return;
    const rtl = typeof document !== 'undefined' && document.documentElement.dir === 'rtl';
    if (e.key === 'Escape' && home.zoomedId) {
      e.preventDefault();
      home.zoomedId = null;
    } else if (e.key === 'ArrowLeft' && !e.metaKey && !e.altKey && !home.zoomedId && t?.getAttribute('role') !== 'slider') {
      if (rtl) home.next();
      else home.prev();
    } else if (e.key === 'ArrowRight' && !e.metaKey && !e.altKey && !home.zoomedId && t?.getAttribute('role') !== 'slider') {
      if (rtl) home.prev();
      else home.next();
    }
  }
  let touchX: number | null = null;
  function onTouchStart(e: TouchEvent): void {
    touchX = e.touches[0]?.clientX ?? null;
  }
  function onTouchEnd(e: TouchEvent): void {
    if (touchX == null) return;
    const dx = (e.changedTouches[0]?.clientX ?? touchX) - touchX;
    touchX = null;
    if (Math.abs(dx) < 60) return;
    const rtl = typeof document !== 'undefined' && document.documentElement.dir === 'rtl';
    if (rtl ? dx > 0 : dx < 0) home.next();
    else home.prev();
  }

  // ── Views ────────────────────────────────────────────────────────────────
  async function addView(): Promise<void> {
    const name = await confirmer.promptText('Space name', { title: `New space ${spaceNumber(home.views.length)}`, confirmLabel: 'Create', initial: spaces.label(home.views.length) });
    if (name) home.addView(name);
  }
  async function renameView(): Promise<void> {
    const v = home.active;
    if (!v) return;
    const name = await confirmer.promptText('Space name', { title: 'Rename space', confirmLabel: 'Rename', initial: v.name });
    if (name && name !== v.name) home.renameView(v.id, name);
  }
  async function removeView(): Promise<void> {
    const v = home.active;
    if (!v) return;
    if (await confirmer.ask(`Delete space “${v.name}” and its ${plural(v.boxes.length, 'widget')}?`, { title: 'Delete space', confirmLabel: 'Delete', danger: true })) {
      home.removeView(v.id);
    }
  }
  function viewMenu(e: MouseEvent | KeyboardEvent): void {
    ctxMenu.show(e, [
      { label: 'Rename space…', icon: 'edit', action: () => void renameView() },
      { label: 'Add space', icon: 'plus', disabled: home.views.length >= MAX_VIEWS, action: () => void addView() },
      { separator: true },
      { label: 'Delete space…', icon: 'trash', danger: true, disabled: home.views.length <= 1, action: () => void removeView() },
    ]);
  }

  // The floating bar (another window, or this one) switched space: restart
  // the rotation countdown like any manual navigation.
  let lastSpace = spaces.active;
  $effect(() => {
    const s = spaces.active;
    if (s === lastSpace) return;
    lastSpace = s;
    home.zoomedId = null;
    home.rotationEpoch += 1;
  });

  // A space renamed in the floating bar renames Home's view too (Home's own
  // renames reach the bar through home.svelte.ts → spaces.setNames).
  $effect(() => {
    const names = spaces.names;
    untrack(() => {
      home.views.forEach((v, i) => {
        if (names[i] && names[i] !== v.name) home.renameView(v.id, names[i]);
      });
    });
  });

  function addBox(kind: HomeBoxKind): void {
    const v = home.active;
    if (!v) return;
    home.addBox(v.id, kind);
    picking = false;
  }

  // ── Drag reorder ─────────────────────────────────────────────────────────
  let dragId: string | null = $state(null);
  function dropOn(targetId: string): void {
    const v = home.active;
    if (!v || !dragId || dragId === targetId) return;
    const to = v.boxes.findIndex((b) => b.id === targetId);
    if (to >= 0) home.moveBox(v.id, dragId, to);
    dragId = null;
  }

  // ⌘K: switch spaces / toggle cycling from anywhere on the page.
  $effect(() => {
    const cmds = home.views.map((v, i) => ({
      id: `home.view-${v.id}`,
      title: `Go to space ${spaceNumber(i)} · ${v.name}`,
      group: 'Home',
      keywords: `home desktop space view slide ${spaceNumber(i)} ${i + 1}`,
      run: () => home.goTo(i),
    }));
    const unreg = registry.register('home', [
      ...cmds,
      { id: 'home.add-widget', title: 'Add widget…', group: 'Home', keywords: 'home desktop widget box add pin', run: () => (picking = true) },
      { id: 'home.rotate', title: home.autoRotate ? 'Stop cycling spaces' : 'Cycle spaces every 30 s', group: 'Home', keywords: 'home dashboard slide auto rotate 30 seconds spaces', run: () => home.setAutoRotate(!home.autoRotate) },
    ]);
    return unreg;
  });

  const full = $derived((home.active?.boxes.length ?? 0) >= MAX_BOXES);
;
async () => {

 { svelteHTML.createElement("svelte:window", {  "onkeydown":onKey,});}


 { svelteHTML.createElement("div", {         "class":`home`,"role":`region`,"aria-label":`Home dashboard`,"ontouchstart":onTouchStart,"ontouchend":onTouchEnd,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: { "title":`Home`,tabs:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("div", { "class":`spaces-bar`,});
         { svelteHTML.createElement("div", {     "class":`spaces segmented`,"role":`tablist`,"aria-label":`Spaces`,});
              for(let v of __sveltets_2_ensureArray(home.views)){let i = 1;v.id;
             {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {                     "class":`space`,"onkeydown":onTabKey,"role":`tab`,"aria-selected":i === home.activeIndex,"tabindex":i === home.activeIndex ? 0 : -1,"aria-label":v.name,"title":`Space ${spaceNumber(i)} · ${v.name}`,"onclick":() => home.goTo(i),"oncontextmenu":(e) => {
                home.goTo(i);
                viewMenu(e);
              },});i === home.activeIndex;
               { svelteHTML.createElement("span", { "class":`num`,});spaceNumber(i); }
              if(i === home.activeIndex){ { svelteHTML.createElement("span", { "class":`sname`,});v.name; }}
             }}
          }
         }
        if(home.views.length < MAX_VIEWS){
          
           { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":addView,"title":`Add space`,"aria-label":`Add space`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":14,}});} }
        }
        if(home.active){
           { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":viewMenu,"title":`Space options`,"aria-label":`Space options`,"aria-haspopup":`menu`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
        }
       }
    };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
      if(home.views.length > 1){
         { svelteHTML.createElement("button", {                "class":`btn small ghost rot`,"onclick":() => home.setAutoRotate(!home.autoRotate),"title":home.autoRotate ? 'Cycling spaces every 30 s — click to pause' : 'Space cycling is paused — click to resume',"aria-pressed":home.autoRotate,"aria-label":`Auto-rotate spaces`,...__sveltets_2_empty({"data-label":home.autoRotate ? 'Pause cycling spaces' : 'Cycle spaces every 30 s'}),...__sveltets_2_empty({"data-icon":home.autoRotate ? 'refresh' : 'play'}),});home.autoRotate;
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":home.autoRotate ? 'refresh' : 'play',"size":12,}});}
          home.autoRotate ? '30s' : 'Paused';
         }
      }
      
      if(home.active && home.active.boxes.length > 0){
         { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":() => (picking = true),"disabled":full,"title":full ? `A space holds at most ${MAX_BOXES} widgets` : 'Add a widget to this space',});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} 
         }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {tabs, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
    
   PageHeader}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },"padded":false,"fill":true,}});
  if(rotating){
    home.rotationEpoch; {
       { svelteHTML.createElement("div", {   "class":`progress`,"aria-hidden":`true`,}); { svelteHTML.createElement("i", { });__sveltets_2_ensureType(String, Number, `${ROTATE_MS} `); } }
    }
  }

  
   { svelteHTML.createElement("div", {               "class":`stage`,"role":`group`,"aria-label":`Home widgets`,"onpointerenter":() => (hovering = true),"onpointerleave":() => (hovering = false),"onfocusin":() => (focusInside = true),"onfocusout":(e) => {
      if (!(e.currentTarget as HTMLElement).contains(e.relatedTarget as Node | null)) focusInside = false;
    },});
    if(home.zoomed && home.active){
       { svelteHTML.createElement("div", { "class":`zoom`,});
          { const $$_xoBemoH4C = __sveltets_2_ensureComponent(HomeBox); new $$_xoBemoH4C({ target: __sveltets_2_any(), props: {    "box":home.zoomed,"viewId":home.active.id,"zoomed":true,}});}
       }
    }
    
     { svelteHTML.createElement("div", {    "class":`desk`,"hidden":!!(home.zoomed && home.active),});viewport.isPhone;
       { const $$_yadoTemoH4C = __sveltets_2_ensureComponent(HomeToday); new $$_yadoTemoH4C({ target: __sveltets_2_any(), props: {}});}
      if(home.active){
            for(let v of __sveltets_2_ensureArray(home.views)){let vi = 1;v.id;
          const on = v.id === home.active.id;
           { svelteHTML.createElement("div", {             "class":`view`,"hidden":!on,"inert":!on,"aria-label":`Space ${spaceNumber(vi)} · ${v.name}`,});viewport.isPhone;__sveltets_2_ensureType(String, Number, `${ROW_PX} `);__sveltets_2_ensureType(String, Number, `${GAP_PX} `);__sveltets_2_ensureType(String, Number, `${48 * home.slideDir} `);
            if(v.boxes.length === 0){
               { svelteHTML.createElement("div", { "class":`empty`,});
                 { const $$_etatSytpmE6C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE6C({ target: __sveltets_2_any(), props: {               children:() => { return __sveltets_2_any(0); },"variant":`page`,"icon":`grid`,"title":`This space is empty`,"body":`Pin live widgets here: your agents, Mission Control, usage, clusters or a database dashboard. Resize from the corner, drag the grip to reorder.`,"actionLabel":`Add widget`,"actionIcon":`plus`,"onaction":() => (picking = true),}});
                  if(kinds.length){
                     { svelteHTML.createElement("div", {   "class":`quick`,"aria-label":`Quick add`,});
                         for(let k of __sveltets_2_ensureArray((kinds.slice(0, 4)))){k.kind;
                         { svelteHTML.createElement("button", {   "class":`chip quick-chip`,"onclick":() => addBox(k.kind),}); { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":k.icon,"size":12,}});}k.label; }
                      }
                     }
                  }
                 EmptyState}
               }
            }else{
                  for(let b of __sveltets_2_ensureArray(v.boxes)){let i = 1;b.id;
                 { const $$_xoBemoH5C = __sveltets_2_ensureComponent(HomeBox); new $$_xoBemoH5C({ target: __sveltets_2_any(), props: {              "box":b,"viewId":v.id,"index":i,"count":v.boxes.length,"active":on && !home.zoomedId,"ondragbox":(id) => (dragId = id),"ondropon":dropOn,}});}
              }
            }
           }
        }
      }else{
         { svelteHTML.createElement("div", { "class":`empty`,});
           { const $$_etatSytpmE5C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE5C({ target: __sveltets_2_any(), props: {              "variant":`page`,"icon":`grid`,"title":`No spaces yet`,"body":`Create a space, then pin widgets to it.`,"actionLabel":`Add space`,"actionIcon":`plus`,"onaction":addView,}});}
         }
      }
     }
   }
   PageBody}
 }

if(picking){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Add widget`,"width":520,"onclose":() => (picking = false),}});
     { svelteHTML.createElement("div", { "class":`kinds`,});
         for(let k of __sveltets_2_ensureArray(kinds)){k.kind;
         { svelteHTML.createElement("button", {   "class":`kind`,"onclick":() => addBox(k.kind),});
           { svelteHTML.createElement("span", { "class":`ki`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":k.icon,"size":16,}});} }
           { svelteHTML.createElement("span", { "class":`kt`,});
             { svelteHTML.createElement("b", {});k.label; }
             { svelteHTML.createElement("small", {});k.blurb; }
           }
         }
      }
         { svelteHTML.createElement("p", { "class":`dim`,});       }
      
     }
   Modal}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const HomePage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type HomePage__SvelteComponent_ = ReturnType<typeof HomePage__SvelteComponent_>;
/*Ωignore_endΩ*/export default HomePage__SvelteComponent_;