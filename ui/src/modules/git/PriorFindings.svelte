<script lang="ts">
  // Findings an earlier run raised that this run re-checked and did not raise
  // again (the scope plan's prior findings, from GET /reviews/{id}/triage).
  // A resolved finding keeps its old review id, so the board — which lists
  // this run's findings — used to lose it: "fixed since the last review" was
  // invisible. Folded by default, under the board.
  import type { PriorFinding } from '../../lib/api/types';
  import Badge from '../../lib/components/Badge.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import type { BadgeTone } from '../../lib/status';
  import { fetchTriage } from './reviewTriageApi';

  let { reviewId }: { reviewId: string } = $props();
  let prior: PriorFinding[] = $state.raw([]);
  let failed = $state(false);

  // Generation guard: a slow answer for the previous review never lands.
  let gen = 0;
  function load(id: string): void {
    const mine = ++gen;
    prior = [];
    failed = false;
    fetchTriage(id)
      .then((t) => { if (mine === gen) prior = t.prior; })
      .catch(() => { if (mine === gen) failed = true; });
  }
  $effect(() => load(reviewId));

  const WORD: Record<string, [string, BadgeTone]> = {
    fixed: ['Fixed', 'ok'],
    still_open: ['Still open', 'warn'],
    not_relevant: ['Not relevant', 'neutral'],
  };
  const fixed = $derived(prior.filter((p) => p.disposition === 'fixed').length);
  const loc = (p: PriorFinding): string =>
    p.finding.path ? `${p.finding.path}${p.finding.line != null ? `:${p.finding.line}` : ''}` : 'General';
</script>

{#if prior.length > 0}
  <details class="pf" data-testid="prior-findings">
    <summary>Resolved since the last run ({fixed}){prior.length > fixed ? ` · ${prior.length - fixed} re-checked, not fixed` : ''}</summary>
    <ul>
      {#each prior as p (p.finding.id)}
        {@const w = WORD[p.disposition] ?? [p.disposition, 'neutral']}
        <li>
          <Badge tone={w[1]} label={w[0]} />
          <span class="pf-title">{p.finding.title || p.finding.body.split('\n')[0]}</span>
          <span class="mono dim pf-loc" dir="ltr">{loc(p)}</span>
          {#if p.evidence}<p class="dim pf-ev">{p.evidence}</p>{/if}
        </li>
      {/each}
    </ul>
  </details>
{:else if failed}
  <LoadState what="the findings an earlier run raised" error="Couldn’t load them." empty variant="compact" onretry={() => load(reviewId)} />
{/if}

<style>
  .pf { border: 1px solid var(--border); border-radius: var(--radius-m); padding: 6px 10px; font-size: var(--fs-s); overflow-wrap: anywhere; }
  .pf summary { cursor: pointer; font-weight: 600; min-height: 28px; display: flex; align-items: center; }
  .pf ul { list-style: none; margin: 6px 0 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .pf li { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .pf-title { font-weight: 600; min-width: 0; }
  .pf-loc { font-size: var(--fs-xs); }
  .pf-ev { flex-basis: 100%; margin: 0; font-size: var(--fs-xs); }
  .mono { font-family: var(--font-mono); }
  .dim { color: var(--text-dim); }
</style>
