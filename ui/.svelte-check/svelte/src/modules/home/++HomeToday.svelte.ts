///<reference types="svelte" />
;// The top of the Home desktop: a greeting + today's line, then four glance
// cards — Needs you · Running · Up next · Recent (data: today.svelte.ts).
// Text that sits straight on the ambient backdrop is --text only (AA over
// any backdrop pixel — unit/ambient.test.ts); dim text lives on the cards.

import { untrack } from 'svelte';
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import LiveWorkingDot from '../../lib/components/LiveWorkingDot.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { now, rel } from '../../lib/stores/now.svelte';
import { today, type TodayRow } from './today.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  const ROWS = 3;
  let expanded = $state<Record<string, boolean>>({});

  // untrack: start() runs the first load synchronously, and that load reads
  // ws.currentId / auth.can(). Tracked, every boot change to them (grants
  // arriving, the saved workspace resolving) stopped the poller — aborting
  // its load AFTER the requests went out — and started a new one: three
  // full Today loads before Home's first paint.
  $effect(() => {
    untrack(() => today.start());
    return () => today.stop();
  });
  // A workspace switch changes every fetched list, and on a cold boot the
  // first load waits for the list to settle (today.start). Not on mount:
  // start() already loads once the list is in, and a refresh then queued a
  // second full load (needs-you, tasks, approvals, designs…).
  let seenWs: string | undefined;
  $effect(() => {
    const key = ws.listSettled ? `ws:${ws.currentId}` : 'pending';
    if (seenWs !== undefined && key !== seenWs) today.refresh();
    seenWs = key;
  });

  const name = $derived(auth.me ? auth.me.display_name || auth.me.username : '');
  const greeting = $derived.by(() => {
    void now();
    const h = new Date().getHours();
    return h < 5 ? 'Good evening' : h < 12 ? 'Good morning' : h < 18 ? 'Good afternoon' : 'Good evening';
  });
  const dateLine = $derived.by(() => {
    void now();
    return new Date().toLocaleDateString(undefined, { weekday: 'long', day: 'numeric', month: 'long' });
  });
  const summary = $derived.by(() => {
    const parts: string[] = [];
    if (today.needs.length) parts.push(`${today.needs.length} ${today.needs.length === 1 ? 'needs' : 'need'} you`);
    if (today.running.length) parts.push(`${today.running.length} working`);
    return parts.length ? parts.join(' · ') : 'All quiet';
  });

  /** "3m ago" for past rows, "in 2h" for what's coming (Up next): the
   *  shared `rel()` handles both directions. */
  const when = (iso: string): string => rel(iso);

  interface Card {
    id: string;
    title: string;
    icon: IconName;
    rows: TodayRow[];
    empty: string;
    /** Rows come from a fetch (skeleton until it settles). */
    fetched?: boolean;
  }
  const cards: Card[] = $derived([
    { id: 'needs', title: 'Needs you', icon: 'bell', rows: today.needs, empty: 'Nothing needs you right now.' },
    { id: 'running', title: 'Working', icon: 'play', rows: today.running, empty: 'No agent, task or workflow is working right now.' },
    {
      id: 'next',
      title: 'Up next',
      icon: 'calendar',
      rows: today.upNext,
      empty: 'Nothing due in the next 24 hours. Reminders and scheduled tasks land here.',
      fetched: true,
    },
    { id: 'recent', title: 'Recent', icon: 'clock', rows: today.recent, empty: 'Designs and pull requests you touch show up here.', fetched: true },
  ]);
;
async () => {

 { svelteHTML.createElement("section", {   "class":`today`,"aria-label":`Today`,});
   { svelteHTML.createElement("div", { "class":`greet`,});
     { svelteHTML.createElement("h2", { "class":`hello`,});greeting;name ? `, ${name}` : ''; }
     { svelteHTML.createElement("p", { "class":`line`,});
      dateLine;  summary;
      if(today.failed){
        
         { svelteHTML.createElement("span", { "class":`warn-line`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}     }
         { svelteHTML.createElement("button", {   "class":`retry`,"onclick":() => today.refresh(),});  }
      }
     }
   }

   { svelteHTML.createElement("div", { "class":`glance`,});
       for(let c of __sveltets_2_ensureArray(cards)){c.id;
       { svelteHTML.createElement("article", {     "class":`gcard`,...__sveltets_2_empty({"data-card":c.id}),"aria-labelledby":`today-${c.id}`,});
         { svelteHTML.createElement("header", { "class":`gc-head`,});
           { svelteHTML.createElement("span", {  "class":`gc-icon`,});c.id === 'needs' && c.rows.length > 0; { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":c.icon,"size":13,}});} }
           { svelteHTML.createElement("h3", { "id":`today-${c.id}`,});c.title; }
          if(c.rows.length){ { svelteHTML.createElement("span", {  "class":`gc-count`,});c.id === 'needs';c.rows.length; }}
         }
        if(c.fetched && !today.loaded){
           { svelteHTML.createElement("div", { "class":`gc-skel`,}); { const $$_notelekS4C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS4C({ target: __sveltets_2_any(), props: {    "rows":2,"height":20,}});} }
        } else if (c.rows.length === 0){
           { svelteHTML.createElement("div", { "class":`gc-empty`,});
             { svelteHTML.createElement("span", { "class":`gc-empty-mark`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":c.id === 'needs' ? 'check' : c.icon,"size":14,}});} }
             { svelteHTML.createElement("p", {});c.empty; }
           }
        }else{
           { svelteHTML.createElement("ul", { "class":`gc-rows`,});
               for(let r of __sveltets_2_ensureArray((c.rows.slice(0, expanded[c.id] ? undefined : ROWS)))){r.id;
               { svelteHTML.createElement("li", {});
                 { svelteHTML.createElement("button", {     "class":`gc-row`,"onclick":r.open,"title":r.title,});
                   { svelteHTML.createElement("span", { "class":`gc-mark`,});
                    if(r.tone === 'working'){
                       { const $$_toDgnikroWeviL7C = __sveltets_2_ensureComponent(LiveWorkingDot); new $$_toDgnikroWeviL7C({ target: __sveltets_2_any(), props: {  "size":7,}});}
                    }else{
                       { svelteHTML.createElement("span", { "class":`gc-dot ${r.tone}`,}); }
                    }
                   }
                   { svelteHTML.createElement("span", { "class":`gc-text`,});
                     { svelteHTML.createElement("span", { "class":`gc-title`,});r.title; }
                     { svelteHTML.createElement("span", {   "class":`gc-sub`,"title":r.detail,});r.detail;if(r.at && when(r.at)){` · ${when(r.at)}`;} }
                   }
                 }
               }
            }
           }
          if(c.rows.length > ROWS){
             { svelteHTML.createElement("button", {     "class":`gc-more`,"aria-expanded":!!expanded[c.id],"onclick":() => (expanded[c.id] = !expanded[c.id]),});expanded[c.id] ? 'Show less' : `${c.rows.length - ROWS} more`; }
          }
        }
       }
    }
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const HomeToday__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type HomeToday__SvelteComponent_ = ReturnType<typeof HomeToday__SvelteComponent_>;
/*Ωignore_endΩ*/export default HomeToday__SvelteComponent_;