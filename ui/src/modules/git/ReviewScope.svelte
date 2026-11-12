<script lang="ts">
  import type { Review } from '../../lib/api/types';
  import { getFinding } from '../../lib/api/client';
  let { review }: { review: Review } = $props();
  // Prior findings are stored by id only; their titles are fetched the first
  // time the scope is opened (a raw id says nothing to a reader).
  let titles: Record<string, string> = $state({});
  let asked = '';
  function loadTitles(open: boolean): void {
    const ids = (review.scope_plan?.prior_findings ?? []).map((p) => p.finding_id);
    if (!open || ids.length === 0 || asked === review.id) return;
    asked = review.id;
    for (const id of ids) {
      getFinding(id)
        .then((d) => (titles = { ...titles, [id]: d.finding.title || d.finding.body.split('\n')[0] }))
        .catch(() => {});
    }
  }
</script>

{#if review.scope_plan}
  <details class="review-scope" open={review.status === 'running'} ontoggle={(e) => loadTitles((e.currentTarget as HTMLDetailsElement).open)}>
    <summary>Review scope · {review.scope_plan.lenses.length} {review.scope_plan.lenses.length === 1 ? 'lens' : 'lenses'}</summary>
    <p>{review.scope_plan.summary}</p>
    <p class="dim">{review.scope_plan.source} · {review.scope_plan.head_sha.slice(0, 12)}</p>
    <ul>
      {#each review.scope_plan.lenses as lens (lens.name)}
        <li><strong>{lens.name}</strong> — {lens.reason}
          {#each review.agents.filter(a => a.lens === lens.name) as agent (agent.name)}
            <span class="dim"> · {agent.provider}{agent.model ? ` / ${agent.model}` : ''}</span>
          {/each}
        </li>
      {/each}
    </ul>
    {#if review.scope_plan.related_modules.length}
      <p><strong>Affected consumers and modules</strong></p>
      <ul>{#each review.scope_plan.related_modules as module}<li>{module}</li>{/each}</ul>
    {/if}
    {#if review.scope_plan.risks.length}
      <p><strong>Risks</strong></p>
      <ul>{#each review.scope_plan.risks as risk}<li>{risk}</li>{/each}</ul>
    {/if}
    {#if review.scope_plan.prior_findings?.length}
      <p><strong>Previous findings · agent verification</strong></p>
      <ul>{#each review.scope_plan.prior_findings as prior (prior.finding_id)}
        <li><strong>{prior.disposition.replaceAll('_', ' ')}</strong> · {#if titles[prior.finding_id]}<strong>{titles[prior.finding_id]}</strong> <span class="dim">({prior.finding_id})</span>{:else}{prior.finding_id}{/if}: {prior.evidence}</li>
      {/each}</ul>
    {/if}
    {#if review.scope_plan.verification_notes?.length}
      <p><strong>Claims checked and excluded</strong></p>
      <ul>{#each review.scope_plan.verification_notes as note}<li>{note}</li>{/each}</ul>
    {/if}
  </details>
{/if}

<style>
  .review-scope { padding: 0.75rem; border: 1px solid var(--border); border-radius: var(--radius-s); font-size: var(--fs-s); overflow-wrap: anywhere; }
  summary { cursor: pointer; font-weight: 600; }
  p { margin-block: 0.5rem; }
  ul { padding-inline-start: 1.25rem; margin-block: 0.5rem; }
  li + li { margin-block-start: 0.5rem; }
</style>
