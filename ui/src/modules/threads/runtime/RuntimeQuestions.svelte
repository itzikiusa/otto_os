<script lang="ts">
  import type { RuntimeRequest } from '../../../lib/api/types';
  import { pendingQuestions, questionOutcome } from './questions';
  import RuntimeQuestionCard from './RuntimeQuestionCard.svelte';
  import { announceEdge } from '../../../lib/announce.svelte';
  let { requests, sessionId, agent, onanswered }: { requests: RuntimeRequest[]; sessionId: string; agent: string; onanswered: () => void } = $props();
  const pending = $derived(pendingQuestions(requests));
  const settled = $derived(requests.filter((r) => r.kind === 'question' && r.status !== 'pending'));
  const announcePending = announceEdge();
  $effect(() => announcePending(pending.length > 0 ? `${pending.length} requests need your answer` : ''));
</script>

{#if pending.length > 0 || settled.length > 0}
  <div class="questions" data-testid="runtime-questions">
    {#each pending as request (request.id)}
      <details open>
        <summary>Needs your answer{pending.length > 1 ? ` · ${request.questions?.[0]?.header ?? 'Question'}` : ''}</summary>
        <RuntimeQuestionCard {request} {sessionId} {agent} {onanswered} />
      </details>
    {/each}
    {#if settled.length > 0}
      <details class="question-history">
        <summary>Previous questions · {settled.length}</summary>
        {#each settled as request (request.id)}
          <details><summary>{request.questions?.[0]?.header ?? 'Question'} · {questionOutcome(request)}</summary>
            <RuntimeQuestionCard {request} {sessionId} {agent} {onanswered} />
          </details>
        {/each}
      </details>
    {/if}
  </div>
{/if}

<style>
  .questions { max-height: 40dvh; overflow-y: auto; min-width: 0; display: grid; gap: 8px; }
  summary { cursor: pointer; padding: 8px 0; font-size: var(--fs-s); overflow-wrap: anywhere; }
  .question-history { color: var(--text-dim); }
</style>
