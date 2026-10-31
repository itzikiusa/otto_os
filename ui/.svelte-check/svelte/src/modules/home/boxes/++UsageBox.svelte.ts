///<reference types="svelte" />
;// Usage box: spend / tokens / events for the last N days (box config, default
// 7), a per-day cost sparkline and the per-provider split. Reads the same
// /usage endpoints the Usage page does, scoped to this box so it never
// disturbs the Usage page's own window selection.

import { untrack } from 'svelte';
import { radioKey } from '../../../lib/radioKey';
import EmptyState from '../../../lib/components/EmptyState.svelte';
import Skeleton from '../../../lib/components/Skeleton.svelte';
import Icon from '../../../lib/components/Icon.svelte';
import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
import { loadErrorText } from '../../../lib/loadError';
import { api } from '../../../lib/api/client';
import { router } from '../../../lib/router.svelte';
import type { UsageStatus, UsageSummary } from '../../../lib/api/usage.svelte';
import { home, type HomeBox } from '../home.svelte';
import { freshness, poll, type Poller } from './poll';
import LoadState from '../../../lib/components/LoadState.svelte';
;

  interface Props {
    box: HomeBox;
    viewId: string;
    zoomed: boolean;
    tick: number;
    /** False while this box's Home space is not on screen (or another box
     *  is zoomed): the poller stops, the data stays (review 06 F3). */
    active?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { box, viewId, zoomed: _zoomed, tick, active = true }: Props = $props();

  const DAYS = [1, 7, 30] as const;
  const days = $derived(DAYS.includes(Number(box.config.days) as (typeof DAYS)[number]) ? Number(box.config.days) : 7);

  let status = $state<UsageStatus | null>(null);
  let summary = $state<UsageSummary | null>(null);
  let loading = $state(true);
  let error = $state('');

  // Request token: a window change starts a new poller while the old one's
  // fetch may still be in flight — only the newest load may land, so the
  // box never shows 30 days of totals under a "7d" label.
  let seq = 0;

  async function load(): Promise<boolean> {
    const mine = ++seq;
    const span = days;
    try {
      const st = await api.bg.get<UsageStatus>('/usage/status');
      const sum = st.available
        ? await api.bg.get<UsageSummary>(`/usage/summary?days=${span}&otto_only=false`)
        : null;
      if (mine !== seq) return true;
      status = st;
      summary = sum;
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
    const key = String(days);
    if (!active) return;
    poller?.stop();
    poller = poll(fresh.run, 60_000, fresh.start(key, 60_000));
    return () => poller?.stop();
  });
  // Frame refresh only — not on mount, where the poller's first run is
  // already loading (a now() then queued a second load on every Home boot).
  let seenTick = untrack(() => tick);
  $effect(() => {
    const t = tick;
    if (t === seenTick) return;
    seenTick = t;
    untrack(() => poller?.now());
  });

  const fmtUsd = (n: number): string => (n >= 100 ? `$${Math.round(n)}` : `$${n.toFixed(2)}`);
  const fmtK = (n: number): string => (n >= 1e9 ? `${(n / 1e9).toFixed(1)}B` : n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}K` : String(n));

  // Sparkline path over daily cost (oldest → newest), normalized to the box.
  const spark = $derived.by(() => {
    const d = summary?.daily ?? [];
    if (d.length < 2) return '';
    const sorted = [...d].sort((a, b) => a.day.localeCompare(b.day));
    const max = Math.max(...sorted.map((x) => x.cost_usd), 0.0001);
    const W = 100;
    const H = 28;
    return sorted
      .map((x, i) => `${i === 0 ? 'M' : 'L'}${((i / (sorted.length - 1)) * W).toFixed(1)},${(H - (x.cost_usd / max) * (H - 2) - 1).toFixed(1)}`)
      .join(' ');
  });
  const providers = $derived([...(summary?.providers ?? [])].sort((a, b) => b.cost_usd - a.cost_usd));
  const maxProv = $derived(Math.max(...providers.map((p) => p.cost_usd), 0.0001));
;
async () => {

 { svelteHTML.createElement("div", { "class":`usage`,});
   { svelteHTML.createElement("div", { "class":`bar`,});
     { svelteHTML.createElement("span", { "class":`dim`,}); days === 1 ? 'day' : `${days} days`; }
     { svelteHTML.createElement("span", { "class":`spacer`,}); }
     { svelteHTML.createElement("div", {     "class":`seg`,"role":`radiogroup`,"aria-label":`Window`,});
         for(let d of __sveltets_2_ensureArray(DAYS)){d;
         { svelteHTML.createElement("button", {           "role":`radio`,"aria-checked":d === days,"tabindex":d === days ? 0 : -1,"onkeydown":radioKey,"onclick":() => home.updateBoxConfig(viewId, box.id, { days: d }),});d === days;d;  }
      }
     }
   }
  if(loading && !summary){
     { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {  "rows":3,}});}
  } else if (error && !summary){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {      "what":`Usage`,"error":error,"empty":true,"onretry":() => poller?.now(),}});}
  } else if (status && !status.available){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"icon":`chart`,"title":`Usage engine is off`,"body":`Enable the embedded ClickHouse engine on the Usage page.`,}});
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => router.go('usage'),});  }
     EmptyState}
  } else if (summary){
    if(error){
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {         "what":`Usage`,"variant":`compact`,error,"empty":false,"onretry":() => poller?.now(),}});}
    }
     { svelteHTML.createElement("div", { "class":`stats`,});
       { svelteHTML.createElement("div", { "class":`stat`,}); { svelteHTML.createElement("span", { "class":`n`,});fmtUsd(summary.total_cost_usd); } { svelteHTML.createElement("span", { "class":`l`,});  } }
       { svelteHTML.createElement("div", { "class":`stat`,}); { svelteHTML.createElement("span", { "class":`n`,});fmtK(summary.total_tokens); } { svelteHTML.createElement("span", { "class":`l`,});  } }
       { svelteHTML.createElement("div", { "class":`stat`,}); { svelteHTML.createElement("span", { "class":`n`,});fmtK(summary.total_output_tokens); } { svelteHTML.createElement("span", { "class":`l`,});  } }
       { svelteHTML.createElement("div", { "class":`stat`,}); { svelteHTML.createElement("span", { "class":`n`,});fmtK(summary.total_events); } { svelteHTML.createElement("span", { "class":`l`,});  } }
     }
    if(spark){
       { svelteHTML.createElement("svg", {       "class":`spark`,"viewBox":`0 0 100 28`,"preserveAspectRatio":`none`,"aria-label":`Daily spend`,});
         { svelteHTML.createElement("path", {          "d":spark,"fill":`none`,"stroke":`var(--accent)`,"stroke-width":`1.5`,"vector-effect":`non-scaling-stroke`,});}
       }
    }
     { svelteHTML.createElement("ul", { "class":`provs`,});
         for(let p of __sveltets_2_ensureArray(providers)){p.provider;
         { svelteHTML.createElement("li", {});
           { const $$_nocIredivorP3C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP3C({ target: __sveltets_2_any(), props: {    "provider":p.provider,"size":12,}});}
           { svelteHTML.createElement("span", { "class":`pn ellipsis`,});p.provider; }
           { svelteHTML.createElement("span", { "class":`meter`,}); { svelteHTML.createElement("i", { });__sveltets_2_ensureType(String, Number, `${(p.cost_usd / maxProv) * 100} `); } }
           { svelteHTML.createElement("span", { "class":`pc`,});fmtUsd(p.cost_usd); }
         }
      }
         { svelteHTML.createElement("li", { "class":`dim`,});      }
      
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const UsageBox__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type UsageBox__SvelteComponent_ = ReturnType<typeof UsageBox__SvelteComponent_>;
/*Ωignore_endΩ*/export default UsageBox__SvelteComponent_;