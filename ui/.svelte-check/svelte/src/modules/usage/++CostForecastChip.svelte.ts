///<reference types="svelte" />
;// Pre-launch cost-forecast chip (B1): small inline widget that calls
// POST /usage/forecast and shows a projected cost estimate before a run.
// Usage: <CostForecastChip feature="review" provider="claude" />
//        <CostForecastChip feature="agent" provider="claude" estTokens={4000} />

import { untrack } from 'svelte';
import { api } from '../../lib/api/client';
import type { ForecastReq, ForecastResp } from './types';
import { formatUsd } from '../../lib/metric-format';
;

  interface Props {
    /** Otto feature label ("review" | "product" | "agent" | "channel" | …). */
    feature: string;
    /** Provider name ("claude" | "codex" | "shell" | …). */
    provider: string;
    /**
     * Optional explicit token estimate — when supplied the forecast is priced
     * directly rather than from historical averages.
     */
    estTokens?: number;
  };function $$render() {

  
  
  
  
  
  
  
  


  let { feature, provider, estTokens }: Props = $props();

  let resp: ForecastResp | null = $state(null);
  let loading = $state(false);

  // Request token: a newer load supersedes any in-flight one, so a slow
  // response for stale inputs can never overwrite the current forecast.
  let seq = 0;

  async function load(f: string, p: string, est: number | undefined): Promise<void> {
    const mine = ++seq;
    loading = true;
    try {
      const req: ForecastReq = { feature: f, provider: p };
      if (est && est > 0) req.est_tokens = est;
      const r = await api.post<ForecastResp>('/usage/forecast', req);
      if (mine === seq) resp = r;
    } catch {
      if (mine === seq) resp = null;
    } finally {
      if (mine === seq) loading = false;
    }
  }

  // Reload whenever inputs change. Only the three props are dependencies:
  // `load` reads and writes `loading`/`resp`, so it runs untracked — a
  // tracked `loading` re-ran this effect on every `finally`, which fired
  // POST /usage/forecast back-to-back for as long as the chip was mounted.
  $effect(() => {
    const f = feature;
    const p = provider;
    const est = estTokens;
    untrack(() => void load(f, p, est));
  });

;
async () => {


if(!loading && resp && resp.projected_cost_usd > 0){
   { svelteHTML.createElement("span", {   "class":`forecast`,"title":`Estimated cost of the next ${feature === 'agent' ? 'agent run' : `${feature} run`} on ${provider}. ${resp.basis}`,});
        formatUsd(resp.projected_cost_usd);
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const CostForecastChip__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type CostForecastChip__SvelteComponent_ = ReturnType<typeof CostForecastChip__SvelteComponent_>;
/*Ωignore_endΩ*/export default CostForecastChip__SvelteComponent_;