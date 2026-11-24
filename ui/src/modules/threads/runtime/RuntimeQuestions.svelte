<script lang="ts">
  // The record of questions the agent asked and how each ended ("Answered",
  // "Withdrawn by the agent" …). A question still waiting is answered in the
  // attention dock above the composer (RuntimeAttentionDock), never here, so
  // its form and Send exist once.
  import type { RuntimeRequest } from '../../../lib/api/types';
  import { questionOutcome } from './questions';
  import { QuestionCardView } from '../threadLazy.svelte';
  let { requests, sessionId, agent, onanswered }: { requests: RuntimeRequest[]; sessionId: string; agent: string; onanswered: () => void } = $props();
  const settled = $derived(requests.filter((r) => r.kind === 'question' && r.status !== 'pending'));
</script>

{#if settled.length > 0}
  <details class="question-history" data-testid="runtime-questions">
    <summary>Previous questions · {settled.length}</summary>
    <div class="questions">
      {#each settled as request (request.id)}
        <details><summary>{request.questions?.[0]?.header ?? 'Question'} · {questionOutcome(request)}</summary>
          {#if QuestionCardView.component}<QuestionCardView.component {request} {sessionId} {agent} {onanswered} />{/if}
        </details>
      {/each}
    </div>
  </details>
{/if}

<style>
  .questions { max-height: 40dvh; overflow-y: auto; min-width: 0; display: grid; gap: 8px; }
  summary { cursor: pointer; padding: 6px 0; font-size: var(--fs-s); overflow-wrap: anywhere; }
  .question-history { color: var(--text-dim); }
</style>
