///<reference types="svelte" />
;// Help (route `#/walkthroughs`, kept for old links, the native Help menu and
// ⌘K). A list/detail page of section guides — one README per sidebar module
// plus the shell-wide Basics — with the single tour film on top of the
// default view.
//
//   #/walkthroughs            film + Getting started (never an empty pane)
//   #/walkthroughs/<id>       that guide (unknown id → inline "not found")
//
// Guides are ./sections/*.md, loaded at build time (./sections.ts) — the
// rail lists whatever files exist, grouped like the sidebar. The rail is
// searchable (title, summary, body and shortcut keys, ranked in ./guide.ts)
// and arrow-key navigable. A guide shows "Watch this part" when a film
// chapter maps to it; a chapter shows "Read the guide" when its guide exists.
// The ⌘K "Guide: …" commands are registered app-wide in App.svelte.
//
// Phone: push navigation — the film + list is the page; opening a guide
// replaces it and the header gets a back button.

import { scrollBehavior } from '../../lib/motion';
import { tick } from 'svelte';
import { router } from '../../lib/router.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { SIDEBAR_MODULES } from '../../lib/sidebar';
import { GUIDE_GROUPS, searchSections, splitChord, timeLabel, type GuideSection } from './guide';
import { DEFAULT_GUIDE_ID, FILM, GUIDES, guideById } from './sections';
import { renderGuideHtml } from './render';
import TourFilm from './TourFilm.svelte';
import PaneDivider from '../../lib/components/PaneDivider.svelte';
import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
function $$render() {
 const row/*Ωignore_positionΩ*/ = (g: GuideSection, match?: { kind: string; text: string })/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("button", {             "class":`rail-row`,"aria-current":g.id === selectedId && !viewport.isPhone ? 'page' : undefined,...__sveltets_2_empty({"data-guide":g.id}),...__sveltets_2_empty({"data-testid":`guide-row`}),"onclick":() => open(g.id),});!searching && g.id === selectedId && !viewport.isPhone;searching && g.id === railOrder[0];
     { svelteHTML.createElement("span", { "class":`row-title`,});g.title; }
    if(searching){
       { svelteHTML.createElement("span", { "class":`row-meta`,});
         { svelteHTML.createElement("span", { "class":`row-group`,});g.group; }
        if(match?.kind === 'shortcut'){
           { svelteHTML.createElement("span", { "class":`keys`,});    for(let k of __sveltets_2_ensureArray(splitChord(match.text))){let i = 1;i; { svelteHTML.createElement("kbd", {});k; }} }
        } else if (match?.text){
           { svelteHTML.createElement("span", { "class":`row-snippet`,});match.text; }
        }
       }
    } else if (g.summary && viewport.isPhone){
       { svelteHTML.createElement("span", { "class":`row-snippet`,});g.summary; }
    }
   }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const guideIds = new Set(GUIDES.map((g) => g.id));
  /** Guides rail width — the shared list-pane default/range, remembered. */
  let listW = $state(loadPaneWidth('walkthroughs.listW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));

  // ---- selection (from the route) ----
  const param = $derived(router.module === 'walkthroughs' ? router.parts[1] : undefined);
  const isDefault = $derived(!param);
  const selectedId = $derived(param ?? DEFAULT_GUIDE_ID);
  const selected = $derived(guideById(selectedId));

  /** The film shows on the default view, and on a guide after "Watch this part"
   *  / "Watch the tour" (reset whenever the guide changes). */
  let filmRequested = $state(false);
  const showFilm = $derived(isDefault || filmRequested);
  let film: ReturnType<typeof TourFilm> | undefined = $state();
  let mainEl: HTMLElement | undefined = $state();

  let lastId = '';
  $effect(() => {
    const id = selectedId;
    if (id === lastId) return;
    lastId = id;
    filmRequested = false;
    mainEl?.scrollTo({ top: 0 });
  });

  function open(id: string, replace = false): void {
    const path = `walkthroughs/${id}`;
    if (replace) router.replace(path);
    else router.go(path);
  }

  // ---- rendered body (memoised per guide) ----
  const htmlCache = new Map<string, string>();
  const html = $derived.by(() => {
    if (!selected) return '';
    let h = htmlCache.get(selected.id);
    if (h === undefined) {
      h = renderGuideHtml(selected.body);
      htmlCache.set(selected.id, h);
    }
    return h;
  });

  // ---- film ↔ guide ----
  const chapterFor = $derived(FILM?.chapters.find((c) => c.section === selectedId));

  async function watchPart(start: number): Promise<void> {
    filmRequested = true;
    await tick();
    mainEl?.scrollTo({ top: 0, behavior: scrollBehavior() });
    film?.playAt(start);
  }

  async function watchTour(): Promise<void> {
    filmRequested = true;
    await tick();
    mainEl?.scrollTo({ top: 0, behavior: scrollBehavior() });
  }

  // ---- "Open <module>" (respects the sidebar's RBAC gate) ----
  function routeAllowed(route: string): boolean {
    const mod = route.split('/')[0];
    if (mod === 'plugin') return auth.canPlugin(route.split('/')[1] ?? '', 'view');
    const def = SIDEBAR_MODULES.find((m) => m.id === mod);
    if (!def) return true;
    if (def.featureAny) return def.featureAny.some((f) => auth.can(f, 'view'));
    return def.feature == null || auth.can(def.feature, 'view');
  }
  const openAllowed = $derived(selected?.route ? routeAllowed(selected.route) : false);

  // ---- search ----
  let query = $state('');
  let searchEl: HTMLInputElement | undefined = $state();
  const hits = $derived(searchSections(GUIDES, query));
  const searching = $derived(query.trim().length > 0);
  const grouped = $derived(
    GUIDE_GROUPS.map((group) => ({ group, items: GUIDES.filter((g) => g.group === group) })).filter(
      (s) => s.items.length > 0,
    ),
  );
  /** What the rail's one always-mounted count region says: nothing until a
   *  search starts, then the match count (or that nothing matched). */
  function searchCountText(q: string, n: number): string {
    if (!q.trim()) return '';
    return n === 0 ? `No guides match “${q.trim()}”` : `${n} ${n === 1 ? 'guide' : 'guides'}`;
  }
  const countText = $derived(searchCountText(query, hits.length));
  /** The rail's rows in on-screen order (arrow-key navigation walks this). */
  const railOrder = $derived(searching ? hits.map((h) => h.section.id) : GUIDES.map((g) => g.id));

  // ---- keyboard: ↑/↓/Home/End move through the rail; Enter in search opens ----
  let railEl: HTMLElement | undefined = $state();

  function focusRow(id: string): void {
    railEl?.querySelector<HTMLElement>(`[data-guide="${CSS.escape(id)}"]`)?.focus();
  }

  function step(from: string | undefined, delta: number | 'first' | 'last'): string | undefined {
    const list = railOrder;
    if (!list.length) return undefined;
    if (delta === 'first') return list[0];
    if (delta === 'last') return list[list.length - 1];
    const i = from ? list.indexOf(from) : -1;
    if (i < 0) return delta > 0 ? list[0] : list[list.length - 1];
    return list[Math.min(Math.max(i + delta, 0), list.length - 1)];
  }

  function onRailKey(e: KeyboardEvent): void {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const inSearch = e.target === searchEl;
    // The phone rail also contains the film. Keep its native controls and
    // chapter buttons out of the guide list's keyboard navigation.
    if (!inSearch && !(e.target instanceof HTMLElement && e.target.closest('[data-guide]'))) return;
    const moves: Record<string, number | 'first' | 'last'> = { ArrowDown: 1, ArrowUp: -1 };
    if (!inSearch) Object.assign(moves, { Home: 'first', End: 'last' });
    if (inSearch && e.key === 'Enter') {
      const top = railOrder[0];
      if (top) {
        e.preventDefault();
        open(top);
      }
      return;
    }
    if (inSearch && e.key === 'Escape' && query) {
      e.preventDefault();
      query = '';
      return;
    }
    const mv = moves[e.key];
    if (mv === undefined) return;
    e.preventDefault();
    const current = (document.activeElement as HTMLElement | null)?.dataset?.guide ?? (inSearch ? undefined : selectedId);
    const next = inSearch && mv === 1 && !current ? step(undefined, 'first') : step(current, mv);
    if (!next) return;
    // Arrow browsing replaces the history entry (Back leaves the page, not
    // every guide you arrowed past); on a phone it only moves focus.
    if (!viewport.isPhone) open(next, true);
    void tick().then(() => focusRow(next));
  }

  const pageSubtitle = 'Guides to every part of Otto, with shortcuts and limits';
  const showList = $derived(!viewport.isPhone || isDefault);
  const showArticle = $derived(!viewport.isPhone || !isDefault);
;
async () => {



 { svelteHTML.createElement("div", { "class":`help-page`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":`Help`,"subtitle":pageSubtitle,leading:() => { async ()/*Ωignore_positionΩ*/ => {
      if(viewport.isPhone && !isDefault){
         { svelteHTML.createElement("button", {       "class":`icon-btn help-back`,"onclick":() => router.go('walkthroughs'),"aria-label":`Back to all guides`,"title":`Back to all guides`,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":16,}});}
         }
      }
    };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
      if(FILM && !showFilm && showArticle && !viewport.isPhone){
         { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":watchTour,...__sveltets_2_empty({"data-label":`Watch the tour`}),...__sveltets_2_empty({"data-icon":`play`}),});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}   
         }
      }
      if(selected?.route && showArticle){
         { svelteHTML.createElement("button", {           "class":`btn small primary`,"disabled":!openAllowed,"title":openAllowed ? `Go to ${selected.title}` : `You don’t have access to ${selected.title}. Ask an admin for access.`,"onclick":() => selected?.route && router.go(selected.route),...__sveltets_2_empty({"data-testid":`guide-open-module`}),});
           selected.title;
         }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
    
   PageHeader}

  if(GUIDES.length === 0){
     { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {         "variant":`page`,"icon":`book`,"title":`No guides in this build`,"body":`The Help guides ship with the app. Reinstall or update Otto to get them back.`,}});}
     PageBody}
  }else{
     { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
       { svelteHTML.createElement("div", {  "class":`help-layout`,});viewport.isPhone;
        if(showList){
          
           { const $$_nav3 = svelteHTML.createElement("nav", {        "class":`rail`,"aria-label":`Guides`,"onkeydown":onRailKey,"style":`--list-pane-w:${listW}px`,});railEl = $$_nav3;
             { svelteHTML.createElement("div", { "class":`rail-search`,});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":13,}});}
               { const $$_input5 = svelteHTML.createElement("input", {                 "dir":`ltr`,"bind:value":query,"type":`search`,"placeholder":`Filter guides and shortcuts…`,"aria-label":`Filter guides`,"autocomplete":`off`,"spellcheck":`false`,...__sveltets_2_empty({"data-testid":`guide-search`}),});searchEl = $$_input5;/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             }
            
             { svelteHTML.createElement("span", {     "class":`sr-only`,"role":`status`,...__sveltets_2_empty({"data-testid":`guide-search-count`}),});countText; }

            if(viewport.isPhone && FILM && !searching){
               { svelteHTML.createElement("div", { "class":`rail-film`,}); { const $$_mliFruoT5C = __sveltets_2_ensureComponent(TourFilm); const $$_mliFruoT5 = new $$_mliFruoT5C({ target: __sveltets_2_any(), props: {       "film":FILM,guideIds,"onopenguide":(id) => open(id),}});film = $$_mliFruoT5;} }
            }

             { svelteHTML.createElement("div", { "class":`rail-list`,});
              if(searching){
                if(hits.length === 0){
                   { svelteHTML.createElement("div", { "class":`rail-empty`,});
                     { svelteHTML.createElement("p", {});   query.trim();  }
                     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => { query = ''; searchEl?.focus(); },});  }
                   }
                }else{
                   { svelteHTML.createElement("div", { "class":`rail-count`,});hits.length; hits.length === 1 ? 'guide' : 'guides'; }
                     for(let h of __sveltets_2_ensureArray(hits)){h.section.id;
                    ;__sveltets_2_ensureSnippet(row(h.section, h.match));
                  }
                }
              }else{
                   for(let s of __sveltets_2_ensureArray(grouped)){s.group;
                   { svelteHTML.createElement("div", {     "class":`rail-group`,"role":`group`,"aria-label":s.group,});
                     { svelteHTML.createElement("div", {   "class":`rail-group-label`,"aria-hidden":`true`,});s.group; }
                       for(let g of __sveltets_2_ensureArray(s.items)){g.id;
                      ;__sveltets_2_ensureSnippet(row(g));
                    }
                   }
                }
              }
             }
           }
          if(!viewport.isPhone){
             { const $$_rediviDenaP3C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP3 = new $$_rediviDenaP3C({ target: __sveltets_2_any(), props: {       width:listW,"storageKey":`walkthroughs.listW`,"label":`Resize the guides list`,}});/*Ωignore_startΩ*/() => listW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP3.$$bindings = 'width';}
          }
        }

        if(showArticle){
           { const $$_main3 = svelteHTML.createElement("main", {    "class":`main`,...__sveltets_2_empty({"data-testid":`guide-main`}),});mainEl = $$_main3;
             { svelteHTML.createElement("div", { "class":`column`,});
              if(showFilm){
                 { const $$_mliFruoT5C = __sveltets_2_ensureComponent(TourFilm); const $$_mliFruoT5 = new $$_mliFruoT5C({ target: __sveltets_2_any(), props: {       "film":FILM,guideIds,"onopenguide":(id) => open(id),}});film = $$_mliFruoT5;}
              }

              if(selected){
                 { svelteHTML.createElement("article", {     "class":`guide`,...__sveltets_2_empty({"data-testid":`guide-article`}),...__sveltets_2_empty({"data-guide-id":selected.id}),});
                   { svelteHTML.createElement("header", { "class":`guide-head`,});
                     { svelteHTML.createElement("div", { "class":`eyebrow`,});selected.group; }
                     { svelteHTML.createElement("h2", {});selected.title; }
                    if(selected.summary){ { svelteHTML.createElement("p", { "class":`summary`,});selected.summary; }}
                    if(chapterFor){
                       { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => chapterFor && watchPart(chapterFor.start),...__sveltets_2_empty({"data-testid":`guide-watch-part`}),});
                         { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}     timeLabel(chapterFor.start);
                       }
                    }
                   }
                   { svelteHTML.createElement("div", { "class":`md-body guide-body`,}); html; }
                 }
              }else{
                 { const $$_etatSytpmE5C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE5C({ target: __sveltets_2_any(), props: {           "icon":`book`,"title":`There’s no guide called “${param}”`,"body":viewport.isPhone
                    ? 'It may have been renamed. Go back to all guides, or start at the beginning.'
                    : 'It may have been renamed. Pick one from the list, or start at the beginning.',"actionLabel":`Open Getting started`,"onaction":() => open(DEFAULT_GUIDE_ID),}});}
              }
             }
           }
        }
       }
     PageBody}
  }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Walkthroughs__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Walkthroughs__SvelteComponent_ = ReturnType<typeof Walkthroughs__SvelteComponent_>;
/*Ωignore_endΩ*/export default Walkthroughs__SvelteComponent_;