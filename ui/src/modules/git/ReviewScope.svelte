<script lang="ts">
  import type { Review } from '../../lib/api/types';
  let { review }: { review: Review } = $props();
</script>

{#if review.scope_plan}
  <details class="review-scope" open={review.status === 'running'}>
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
        <li><strong>{prior.disposition.replaceAll('_', ' ')}</strong> · {prior.finding_id}: {prior.evidence}</li>
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
