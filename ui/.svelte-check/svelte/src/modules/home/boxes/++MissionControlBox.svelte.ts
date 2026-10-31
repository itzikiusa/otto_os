///<reference types="svelte" />
;// Mission Control box: the work-graph header summary (active / needs approval
// / total / spend) plus the most recently updated items. Reloads on every
// live `work_graph_updated` tick and on a 30s cadence as a safety net.

import { untrack } from 'svelte';
import Badge from '../../../lib/components/Badge.svelte';
import { sentenceCase } from '../../../lib/labels';
import Icon from '../../../lib/components/Icon.svelte';
import EmptyState from '../../../lib/components/EmptyState.svelte';
import Skeleton from '../../../lib/components/Skeleton.svelte';
import { ws } from '../../../lib/stores/workspace.svelte';
import { router } from '../../../lib/router.svelte';
import { missionControlBus } from '../../../lib/events.svelte';
import { missionControlApi } from '../../../lib/api/missionControl';
import { now } from '../../../lib/stores/now.svelte';
import type { MissionSummary, WorkItem } from '../../../lib/api/types';
import { KIND_ICON, STATUS_LABEL, fmtCost, relTime, statusColor } from '../../mission-control/lib';
import type { HomeBox } from '../home.svelte';
import { loadErrorText } from '../../../lib/loadError';
import { freshness, livePoll, type Poller } from './poll';
import LoadState from '../../../lib/components/LoadState.svelte';
;

  interface Props {
    box: HomeBox;
    zoomed: boolean;
    tick: number;
    /** False while this box's Home space is not on screen (or another box
     *  is zoomed): the poller stops, the data stays (review 06 F3). */
    active?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { box, zoomed, tick, active: onScreen = true }: Props = $props();

  let summary = $state<MissionSummary | null>(null);
  let items = $state<WorkItem[]>([]);
  let loading = $state(true);
  let error = $state('');

  const limit = $derived(zoomed ? 60 : Math.max(5, Number(box.config.limit) || 12));

  // Request token: a workspace/limit change starts a new poller while the old
  // one's fetch may still be in flight — only the newest load may land.
  let seq = 0;

  async function load(): Promise<boolean> {
    const id = ws.currentId;
    if (!id) return true;
    const mine = ++seq;
    try {
      const [s, its] = await Promise.all([
        missionControlApi.summary(id),
        missionControlApi.items(id, { limit }),
      ]);
      if (mine !== seq) return true;
      summary = s;
      items = its;
      error = '';
      return true;
    } catch (e) {
      if (mine !== seq) return true;
      error = loadErrorText(e);
      return false;
    } finally {
      if (mine === seq) loading = false;
    }
  }

  let poller: Poller | null = null;
  const fresh = freshness(load);
  $effect(() => {
    const key = `${ws.currentId}|${limit}`;
    if (!onScreen) return;
    const immediate = fresh.start(key, 30_000);
    if (immediate) loading = true;
    poller?.stop();
    // Live ticks arrive via missionControlBus (below); the 30 s cadence runs
    // only while the event socket is down (5-min safety net otherwise).
    poller = livePoll(fresh.run, 30_000, [], { immediate });
    return () => poller?.stop();
  });
  // Manual refresh (frame button) + live work-graph ticks: rerun immediately.
  // Not on mount: the poller's own first run is already loading, and a now()
  // then queued a second summary + items load on every cold Home boot.
  let seenTick = untrack(() => tick);
  let seenBus = untrack(() => missionControlBus.tick);
  $effect(() => {
    const t = tick;
    const b = missionControlBus.tick;
    if (t === seenTick && b === seenBus) return;
    seenTick = t;
    seenBus = b;
    untrack(() => poller?.now());
  });

  const active = $derived(items.filter((i) => i.status === 'running' || i.status === 'waiting' || i.status === 'blocked'));
  const recent = $derived(items.filter((i) => !active.includes(i)));
  function statusTextColor(s: WorkItem['status']): string {
    switch (s) {
      case 'running':
        return 'var(--success)';
      case 'waiting':
      case 'blocked':
        return 'var(--warning)';
      case 'failed':
        return 'var(--danger)';
      case 'done':
        return 'var(--accent-text)';
      default:
        return 'var(--text-dim)';
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`mc`,});
  if(loading && !summary){
     { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {  "rows":4,}});}
  } else if (error && !summary){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {      "what":`Mission Control`,"error":error,"empty":true,"onretry":() => poller?.now(),}});}
  } else if (summary){
    if(error){
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {         "what":`Mission Control`,"variant":`compact`,error,"empty":false,"onretry":() => poller?.now(),}});}
    }
     { svelteHTML.createElement("div", { "class":`stats`,});
       { svelteHTML.createElement("div", { "class":`stat`,}); { svelteHTML.createElement("span", {  "class":`n`,});summary.active > 0;summary.active; } { svelteHTML.createElement("span", { "class":`l`,});  } }
       { svelteHTML.createElement("div", { "class":`stat`,}); { svelteHTML.createElement("span", {  "class":`n`,});summary.needs_approval > 0;summary.needs_approval; } { svelteHTML.createElement("span", { "class":`l`,});  } }
       { svelteHTML.createElement("div", { "class":`stat`,}); { svelteHTML.createElement("span", { "class":`n`,});summary.total; } { svelteHTML.createElement("span", { "class":`l`,});  } }
       { svelteHTML.createElement("div", { "class":`stat`,}); { svelteHTML.createElement("span", { "class":`n`,});fmtCost(summary.total_cost); } { svelteHTML.createElement("span", { "class":`l`,});  } }
     }
     { svelteHTML.createElement("div", { "class":`chips`,});
         for(let b of __sveltets_2_ensureArray(summary.by_status)){b.key;
        if(b.count > 0){
           { const $$_egdaB2C = __sveltets_2_ensureComponent(Badge); new $$_egdaB2C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"variant":`outline`,}});
             { svelteHTML.createElement("i", {  "class":`sdot`,});__sveltets_2_ensureType(String, Number, statusColor(b.key as WorkItem['status'])); }STATUS_LABEL[b.key as WorkItem['status']] ?? sentenceCase(b.key); b.count;
           Badge}
        }
      }
     }
    if(items.length === 0){
       { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {      "icon":`radar`,"title":`Nothing in flight`,"body":`Sessions, reviews, loops and PRs show up here as they run.`,}});}
    }else{
       { svelteHTML.createElement("ul", { "class":`rows`,});
           for(let it of __sveltets_2_ensureArray(([...active, ...recent]))){it.id;
           { svelteHTML.createElement("li", {});
             { svelteHTML.createElement("button", {     "class":`mc-row`,"onclick":() => router.go(`mission-control/${it.id}`),"title":it.title,});
               { svelteHTML.createElement("span", {  "class":`dot`,});__sveltets_2_ensureType(String, Number, statusColor(it.status)); }
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":KIND_ICON[it.kind],"size":12,}});}
               { svelteHTML.createElement("span", { "class":`title ellipsis`,});it.title; }
               { svelteHTML.createElement("span", {  "class":`st`,});__sveltets_2_ensureType(String, Number, statusTextColor(it.status));STATUS_LABEL[it.status]; }
               { svelteHTML.createElement("span", {   "class":`ago`,"title":new Date(it.updated_at).toLocaleString(),});now() && relTime(it.updated_at); }
             }
           }
        }
       }
    }
     { svelteHTML.createElement("button", {   "class":`foot`,"onclick":() => router.go('mission-control'),});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});}
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MissionControlBox__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MissionControlBox__SvelteComponent_ = ReturnType<typeof MissionControlBox__SvelteComponent_>;
/*Ωignore_endΩ*/export default MissionControlBox__SvelteComponent_;